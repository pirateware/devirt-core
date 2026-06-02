//! Differential safety net over the real sample corpus. Asserts the
//! cross-cutting invariants that must hold for *every* sample, without ever
//! checking sample-specific output:
//!   1. the output re-parses with no new errors (we emit valid JS);
//!   2. the pipeline is idempotent (a second run changes nothing) — our proxy
//!      for "reached a fixpoint and didn't thrash".

use std::fs;
use std::path::PathBuf;

use js_deobfuscator::deobfuscate;

fn samples() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) == Some("js") {
                let name = p.file_name().unwrap().to_string_lossy().into_owned();
                out.push((name, fs::read_to_string(&p).unwrap()));
            }
        }
    }
    out
}

#[test]
fn every_sample_is_safe() {
    let samples = samples();
    assert!(!samples.is_empty(), "no samples found");

    for (name, src) in samples {
        let first = deobfuscate(&src, &name);
        assert!(!first.code.is_empty(), "{name}: empty output");

        // Output must be valid JS (re-parsing introduces no errors).
        let second = deobfuscate(&first.code, &name);
        assert!(
            second.parse_errors.is_empty(),
            "{name}: output did not re-parse cleanly: {:?}",
            second.parse_errors
        );

        // Idempotent: the second run's output equals the first's.
        assert_eq!(first.code, second.code, "{name}: pipeline is not idempotent");
    }
}
