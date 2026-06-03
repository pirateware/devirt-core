//! js-deobfuscator: a generic, sample-agnostic JS deobfuscator built as a
//! fixpoint of semantics-preserving AST rewrites over oxc.

mod driver;
mod ident;
mod ir;
mod pass;
mod passes;
pub mod sandbox;
mod util;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use std::path::Path;

pub struct Report {
    pub code: String,
    pub parse_errors: Vec<String>,
    pub rounds: usize,
    pub converged: bool,
    /// (pass name, number of rounds in which it changed the tree)
    pub changes: Vec<(String, usize)>,
}

/// Parse and re-print `source` with no transforms. Useful as the "before" side
/// of a diff: it shares the deobfuscator's exact formatter, so a diff against
/// `deobfuscate` output shows only real deobfuscation changes, not formatting.
pub fn format_only(source: &str, filename: &str) -> String {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(filename)).unwrap_or_default();
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions { preserve_parens: false, ..ParseOptions::default() })
        .parse();
    Codegen::new()
        .with_options(CodegenOptions::default())
        .build(&parsed.program)
        .code
}

/// Deobfuscate `source`. `filename` is used only to infer the source type
/// (module vs script, jsx, ts). The pipeline is the sample-agnostic default.
pub fn deobfuscate(source: &str, filename: &str) -> Report {
    let allocator = Allocator::default();
    let source_type = SourceType::from_path(Path::new(filename)).unwrap_or_default();

    // Drop `ParenthesizedExpression` wrappers — they carry no meaning for
    // deobfuscation and would force every pass to see through them.
    let parsed = Parser::new(&allocator, source, source_type)
        .with_options(ParseOptions { preserve_parens: false, ..ParseOptions::default() })
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
    }
}
