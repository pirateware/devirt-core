//! Full-sample **behavior-equivalence** regression net.
//!
//! The snippet-level harness in `equivalence.rs` proves individual passes
//! preserve value on hand-written fixtures. This proves the *whole pipeline*
//! preserves *observable behavior* on the *real obfuscated corpus*.
//!
//! For each sample we compute a [`BehaviorSignature`] — terminal outcome
//! (completed / threw `<ErrorName>` / timed-out) plus the exact `console` output
//! — for the original and for the deobfuscated form, running both under the same
//! deterministic permissive browser-host shim (see [`sandbox::behavior_signature`]).
//! A sound deobfuscation must not change that signature.
//!
//! Why this works even though the samples are anti-bot / fingerprinting code
//! that needs a browser: the shim makes them *runnable* (every host global is a
//! permissive proxy) and *deterministic* (frozen clock + RNG, inert timers), so
//! the only thing that can move the signature is the transform itself. We don't
//! need the samples to do anything *meaningful* — only to do the *same* thing
//! before and after.
//!
//! Samples that don't terminate under the shim (a polling / animation loop the
//! proxy never breaks) are recorded as skipped with a reason, never silently
//! dropped — the test prints the full checked/skipped ledger and fails if the
//! checked set unexpectedly collapses.

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use devirt_core::deobfuscate;
use devirt_core::sandbox::{behavior_signature, BehaviorSignature, RunOutcome};

/// Per-sample wall-clock budget for one program run under the shim. Overridable
/// (`DEOB_EQUIV_BUDGET_MS`) so a slow CI box can loosen it without a code change.
fn budget() -> Duration {
    let ms = std::env::var("DEOB_EQUIV_BUDGET_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(5000);
    Duration::from_millis(ms)
}

fn samples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples")
}

fn sorted_samples() -> Vec<(String, String)> {
    let mut out: Vec<_> = fs::read_dir(samples_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("js"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The net is only meaningful if a signature actually *discriminates* behavior.
/// Pin that: differing output, differing throw, and output-vs-throw must all be
/// distinguishable, while identical programs must match. A harness bug that
/// collapsed every run to one constant would make the corpus test vacuously
/// pass — this catches that.
#[test]
fn behavior_signature_discriminates() {
    let b = budget();
    let sig = |s: &str| behavior_signature(s, b);

    let a = sig("console.log(1); console.log(2);");
    let a2 = sig("console.log(1); console.log(2);");
    let diff_out = sig("console.log(1); console.log(3);");
    let threw = sig("console.log(1); null.x;");
    let threw_ref = sig("nope_undefined_global();");

    assert_eq!(a, a2, "same program must produce the same signature");
    assert_eq!(a.outcome, RunOutcome::Completed);
    assert_eq!(a.output, vec!["1".to_string(), "2".to_string()]);
    assert_ne!(a, diff_out, "different console output must differ");
    assert_ne!(a, threw, "completed vs threw must differ");
    assert!(matches!(threw.outcome, RunOutcome::Threw(ref n) if n == "TypeError"));
    // Output captured *before* a throw is retained, and the throw class is part
    // of the signature.
    assert_eq!(threw.output, vec!["1".to_string()]);
    assert_ne!(threw, threw_ref, "different error classes must differ");

    // A non-terminating program is its own (skippable) outcome.
    let spin = behavior_signature("while (true) {}", Duration::from_millis(300));
    assert_eq!(
        spin,
        BehaviorSignature {
            outcome: RunOutcome::TimedOut,
            output: Vec::new()
        }
    );
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: full real-sample corpus, deobfuscated + executed under the shim. Run with `--features slow-tests` (CI) or `cargo nextest run --run-ignored all`."
)]
fn deobfuscation_preserves_observable_behavior_on_every_sample() {
    let budget = budget();
    let mut checked = Vec::new();
    let mut skipped = Vec::new();
    let mut failures = Vec::new();

    for (name, src) in sorted_samples() {
        let orig = behavior_signature(&src, budget);

        // A program that won't terminate under the shim has no comparable
        // signature — record the reason and move on. (Running the deob form too
        // would just burn another full budget for the same non-answer.)
        if orig.outcome == RunOutcome::TimedOut {
            skipped.push(format!(
                "{name}: non-terminating under host shim (>{} ms) — not comparable",
                budget.as_millis()
            ));
            continue;
        }

        let deob = deobfuscate(&src, &name);
        assert!(
            deob.parse_errors.is_empty(),
            "{name}: deob produced parse errors: {:?}",
            deob.parse_errors
        );
        let deob_sig = behavior_signature(&deob.code, budget);

        if deob_sig == orig {
            checked.push(format!(
                "{name}: {:?}, {} line(s)",
                orig.outcome,
                orig.output.len()
            ));
        } else if deob_sig.outcome == RunOutcome::TimedOut {
            // Original terminated but deob didn't: that *is* a behavior change
            // (the transform introduced or unmasked a non-terminating path).
            failures.push(format!(
                "{name}: ORIGINAL terminated ({:?}) but DEOBFUSCATED did not (timed out)",
                orig.outcome
            ));
        } else {
            failures.push(format!(
                "{name}: signature changed\n    original: {:?} | {:?}\n    deob:     {:?} | {:?}",
                orig.outcome, orig.output, deob_sig.outcome, deob_sig.output
            ));
        }
    }

    // Print the full ledger so a passing run still documents coverage.
    eprintln!("\n=== sample behavior-equivalence ledger ===");
    eprintln!("checked ({}):", checked.len());
    for c in &checked {
        eprintln!("  ✓ {c}");
    }
    eprintln!("skipped ({}):", skipped.len());
    for s in &skipped {
        eprintln!("  · {s}");
    }

    assert!(
        failures.is_empty(),
        "behavior-equivalence regressions:\n{}",
        failures.join("\n")
    );

    // Guard against the net silently going slack: if a future change makes
    // *every* sample time out (e.g. a broken shim), we'd "pass" with zero
    // checks. Require that the bulk of the corpus stays comparable.
    assert!(
        checked.len() >= 8,
        "only {} samples were behavior-checked (expected ≥ 8); the harness may be broken.\nskipped:\n{}",
        checked.len(),
        skipped.join("\n")
    );
}
