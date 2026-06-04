//! js-deobfuscator: a generic, sample-agnostic JS deobfuscator built as a
//! fixpoint of semantics-preserving AST rewrites over oxc.

mod driver;
mod ident;
pub(crate) mod ir;
mod pass;
mod passes;
pub mod sandbox;
mod util;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use std::path::Path;

/// WASM/JS entry point (browser, Cloudflare Workers, Node). Deobfuscates
/// `source` and returns the transformed code. `filename` only steers source-type
/// inference (module vs script, jsx, ts) — pass e.g. `"input.js"`. On an internal
/// pipeline panic it returns the input cleanly reformatted, the same graceful
/// fallback as the native [`deobfuscate`] API (callers never see a trap on the
/// normal error paths; see the crate's WASM notes for the one engine edge case).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn deobfuscate_js(source: &str, filename: &str) -> String {
    deobfuscate(source, filename).code
}

pub struct Report {
    pub code: String,
    pub parse_errors: Vec<String>,
    pub rounds: usize,
    pub converged: bool,
    /// (pass name, number of rounds in which it changed the tree)
    pub changes: Vec<(String, usize)>,
    /// `Some(msg)` when the transform pipeline panicked and the deobfuscator fell
    /// back to returning the input cleanly reformatted (an identity transform).
    /// `None` on the normal path. Callers can surface this without crashing.
    pub error: Option<String>,
}

/// Parse and re-print `source` with no transforms. Useful as the "before" side
/// of a diff: it shares the deobfuscator's exact formatter, so a diff against
/// `deobfuscate` output shows only real deobfuscation changes, not formatting.
pub fn format_only(source: &str, filename: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(filename)).unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions {
            preserve_parens: false,
            ..ParseOptions::default()
        })
        .parse();
    Codegen::new()
        .with_options(CodegenOptions::default())
        .build(&parsed.program)
        .code
}

/// Test-support: parse `source`, run a single named pass once, and return the
/// printed result. Pass-level unit tests use this to observe one pass free of
/// pipeline interactions (e.g. an earlier pass inlining a fixture away before the
/// pass under test ever sees it). Not part of the public deobfuscation API.
#[doc(hidden)]
pub fn run_pass(source: &str, filename: &str, pass_name: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(filename)).unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions {
            preserve_parens: false,
            ..ParseOptions::default()
        })
        .parse();
    let mut program = parsed.program;
    let mut p: Box<dyn pass::Pass> = match pass_name {
        "rename" => Box::new(passes::RenameByRole::default()),
        // Unknown pass: identity (reprint unchanged). Test-support only, but a
        // public entry point should not panic on bad input.
        _ => {
            return Codegen::new()
                .with_options(CodegenOptions::default())
                .build(&program)
                .code;
        }
    };
    p.run(&mut program, &allocator);
    Codegen::new()
        .with_options(CodegenOptions::default())
        .build(&program)
        .code
}

/// Deobfuscate `source`. `filename` is used only to infer the source type
/// (module vs script, jsx, ts). The pipeline is the sample-agnostic default.
///
/// A deobfuscator runs over adversarial input through deep machinery (oxc
/// traversal, the SSA IR, the Boa sandbox). Two host-crash vectors are
/// contained here so a malicious file degrades gracefully instead of taking down
/// the process:
///
/// * **Panics** (an invariant violation, an oxc internal panic) — the worker
///   thread's `join()` returns `Err`; we fall back to returning the input cleanly
///   reformatted (an identity transform) with [`Report::error`] set.
/// * **Stack overflow** from deeply nested input — `catch_unwind` *cannot* catch
///   this (it aborts, not unwinds), so the pipeline runs on a dedicated worker
///   thread with a large explicit stack. This also makes the depth ceiling
///   independent of the caller's stack (a 2 MiB server worker thread would
///   otherwise overflow on a few hundred nesting levels).
///
/// Either way the contract holds: never a host-process crash, and never a
/// half-transformed (possibly non-equivalent) program — correctness over
/// completeness.
pub fn deobfuscate(source: &str, filename: &str) -> Report {
    // 256 MiB ≈ tens of thousands of nesting levels — far beyond any realistic
    // input, while still bounded (truly unbounded adversarial depth remains a
    // known limit; a stack overflow there aborts, as it must).
    const WORKER_STACK: usize = 256 * 1024 * 1024;

    let joined = util::run_isolated("deob-pipeline", WORKER_STACK, || {
        run_pipeline(source, filename)
    });

    match joined {
        Ok(report) => report,
        Err(payload) => {
            let msg = panic_message(payload.as_ref());
            // Fallback: reparse and reprint with no transforms. Guard this too —
            // if even formatting panics, hand back the source verbatim.
            let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                format_only(source, filename)
            }))
            .unwrap_or_else(|_| source.to_string());
            Report {
                code,
                parse_errors: Vec::new(),
                rounds: 0,
                converged: false,
                changes: Vec::new(),
                error: Some(format!(
                    "transform pipeline panicked; returned input unchanged: {msg}"
                )),
            }
        }
    }
}

/// The uncontained body of [`deobfuscate`]: parse, run the fixpoint pipeline, and
/// reprint. Any panic here is caught by [`deobfuscate`].
fn run_pipeline(source: &str, filename: &str) -> Report {
    // Test-only hook to exercise the panic-containment path deterministically.
    // Entirely absent from production builds (`#[cfg(test)]`).
    #[cfg(test)]
    if source.contains("__DEOB_FORCE_PANIC__") {
        panic!("forced pipeline panic (test)");
    }

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(filename)).unwrap_or_default();

    // Drop `ParenthesizedExpression` wrappers — they carry no meaning for
    // deobfuscation and would force every pass to see through them.
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions {
            preserve_parens: false,
            ..ParseOptions::default()
        })
        .parse();
    let parse_errors = parsed.errors.iter().map(|e| e.to_string()).collect();

    let mut program = parsed.program;
    let mut pipeline = passes::default_pipeline();
    let stats = driver::run_fixpoint(&mut program, &allocator, &mut pipeline, 50);

    let code = Codegen::new()
        .with_options(CodegenOptions::default())
        .build(&program)
        .code;

    Report {
        code,
        parse_errors,
        rounds: stats.rounds,
        converged: stats.converged,
        changes: stats.changes,
        error: None,
    }
}

/// Best-effort human-readable text from a `catch_unwind` payload.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

#[cfg(test)]
mod containment_tests {
    use super::*;

    #[test]
    fn pipeline_panic_degrades_to_identity_reprint() {
        // The forced-panic hook fires inside the pipeline; deobfuscate must
        // contain it, set `error`, and return the input cleanly reformatted
        // rather than crashing or emitting a half-transformed program.
        let src = r#"var x = "__DEOB_FORCE_PANIC__"; console.log(x);"#;
        let report = deobfuscate(src, "t.js");
        assert!(
            report
                .error
                .as_deref()
                .is_some_and(|e| e.contains("panicked")),
            "expected contained-panic error, got {:?}",
            report.error
        );
        // Fallback output is the formatted input — same as a transform-free run.
        assert_eq!(report.code, format_only(src, "t.js"));
        assert!(!report.converged);
        // And it must re-parse (i.e. be valid JS, not corrupt).
        let reparse = deobfuscate(&report.code, "t.js");
        assert!(reparse.parse_errors.is_empty());
    }

    #[test]
    fn normal_input_has_no_error() {
        let report = deobfuscate("var a = 1 + 2; console.log(a);", "t.js");
        assert!(report.error.is_none());
    }
}
