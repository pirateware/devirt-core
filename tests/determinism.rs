//! Pipeline determinism.
//!
//! The deobfuscator must produce byte-identical output for the same input, run
//! after run. Nondeterminism showed up two ways:
//!
//!   1. A sandboxed partial evaluation reading the wall clock (`Date.now()`) —
//!      the in-process double-eval guard misses it (two millisecond reads agree),
//!      so a `Date.now()` seed/timestamp leaked into a folded literal and jittered
//!      across runs. This reproduces *within a single process* (two consecutive
//!      `deobfuscate` calls read the clock at different instants) and is what these
//!      tests pin down — they FAIL on the un-frozen sandbox and PASS once
//!      `Date.now()` is frozen.
//!   2. `HashMap`/`HashSet` iteration order feeding emission (seed varies per
//!      process). Guarded by the cross-process byte-stability sweep in the
//!      `report` dashboard; the in-process checks below cover the clock leak.
//!
//! Mid-size real samples (sample_3, sample_11) are the historical jitter sources,
//! so they are the headline fixtures here alongside a couple of smaller ones.

use js_deobfuscator::deobfuscate;

/// Deobfuscate `name` (from the `samples/` dir) `n` times in this process and
/// assert every run is byte-identical to the first.
fn assert_stable(name: &str, n: usize) {
    let path = format!("{}/samples/{name}", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let first = deobfuscate(&src, name).code;
    for i in 1..n {
        let again = deobfuscate(&src, name).code;
        assert_eq!(
            first.len(),
            again.len(),
            "{name}: run {i} length differs ({} vs {}) — nondeterministic output",
            first.len(),
            again.len(),
        );
        assert!(
            first == again,
            "{name}: run {i} bytes differ from run 0 — nondeterministic output",
        );
    }
}

#[test]
fn sample_3_is_deterministic() {
    // sample_3 jittered on `turnstileLoadInitTimeTsMs = Date.now()`.
    assert_stable("sample_3.js", 5);
}

#[test]
fn sample_11_is_deterministic() {
    // sample_11 jittered on a `Date.now()`-seeded PRNG constant.
    assert_stable("sample_11.js", 5);
}

#[test]
fn sample_7_is_deterministic() {
    // sample_7 jittered on a `+new Date()` token timestamp once decoder-lift began
    // folding its decoder chain: the double-eval guard misses sub-millisecond
    // `new Date()` reads, so the whole `new Date()` wall clock (not just
    // `Date.now()`) is frozen in the sandbox. This pins that fix.
    assert_stable("sample_7.js", 5);
}

#[test]
fn small_samples_are_deterministic() {
    for s in ["sample_2.js", "sample_5.js", "sample_6.js"] {
        assert_stable(s, 3);
    }
}

/// Output must not depend on the host *environment* (timezone, locale) — only on
/// the input. A decoder that reads `new Date().getTimezoneOffset()` or hashes
/// `Date.prototype.toString()` (sample_7 fingerprinting) would otherwise lift to a
/// machine-local value and jitter across CI runners — the bug this pins (the
/// sandbox pins all TZ/locale-dependent `Date` methods to UTC). Runs the built
/// binary in subprocesses with perturbed `TZ`/`LANG`, since those are read from
/// the process environment, not settable per in-process call.
#[test]
fn output_is_environment_invariant() {
    use std::process::Command;

    let bin = env!("CARGO_BIN_EXE_deob");
    let envs = [
        ("UTC", "C"),
        ("America/New_York", "en_US.UTF-8"),
        ("Asia/Kolkata", "de_DE.UTF-8"),
        ("Pacific/Auckland", "C"),
    ];
    // sample_7 is the known carrier (timezone + Date-string fingerprints); a few
    // others guard against regressions elsewhere.
    for sample in ["sample_7.js", "sample_3.js", "sample_11.js"] {
        let path = format!("{}/samples/{sample}", env!("CARGO_MANIFEST_DIR"));
        let mut baseline: Option<Vec<u8>> = None;
        for (tz, lang) in envs {
            let out = Command::new(bin)
                .arg(&path)
                .env("TZ", tz)
                .env("LANG", lang)
                .env("LC_ALL", lang)
                .output()
                .unwrap_or_else(|e| panic!("run {bin} on {sample}: {e}"));
            assert!(
                out.status.success(),
                "{sample}: deob exited non-zero under TZ={tz} LANG={lang}"
            );
            match &baseline {
                None => baseline = Some(out.stdout),
                Some(b) => assert!(
                    *b == out.stdout,
                    "{sample}: output differs under TZ={tz} LANG={lang} \
                     — a lift depends on the host environment (not portable)"
                ),
            }
        }
    }
}
