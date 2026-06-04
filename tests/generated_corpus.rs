//! Synthetic obfuscated-corpus correctness net.
//!
//! `scripts/gen-samples.js` obfuscates each deterministic seed in
//! `samples/seeds/` under several `javascript-obfuscator` profiles (string
//! arrays, control-flow flattening, dead-code injection, …) into
//! `samples/generated/`, recording each sample's *known* console output in
//! `manifest.json`. This test runs both the obfuscated original and its
//! deobfuscated form in QuickJS and asserts each reproduces that exact output:
//!
//!   * **original ≡ expected** — guards that the fixture itself is portable
//!     (the seed runs the same in QuickJS as in the Node that generated the
//!     manifest); a mismatch flags a non-portable seed, not a deobfuscator bug.
//!   * **deobfuscated ≡ expected** — the real assertion: deobfuscation preserved
//!     observable behavior across every obfuscation technique.
//!   * **idempotent** — a second pass changes nothing.
//!
//! Unlike the byte-exact goldens (which pin the *real* corpus), this corpus is
//! large and regenerable, so it is checked by *behavior*, not byte snapshots.
//! Regenerate with `node scripts/gen-samples.js`.

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use js_deobfuscator::deobfuscate;
use js_deobfuscator::sandbox::{behavior_signature, RunOutcome};

fn generated_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("generated")
}

/// Samples with a *known, open* deobfuscator bug — excluded from the hard
/// assertion but reported loudly every run so they are never silently forgotten.
/// See `issues/13-mutated-array-length-fold.md`.
///
/// Empty: the mutated-array `.length`/element fold (`a.length`→0, `a[i]`→undefined
/// for a `const a=[]` that is later `a.push(…)`-ed) is now fully fixed under every
/// obfuscation profile. The remaining hole was decoder-lift baking a mutated
/// aggregate into the sandbox preamble as its empty literal: closed by excluding
/// `mutable_aggregate_tables` from the pure-data seed and by treating an opaque
/// computed-method call (`a[_dec("push")](x)`) as a possible mutator in the SSA
/// escape scan. Keep this list as the quarantine hook for any future regression.
const KNOWN_BUGS: &[&str] = &[];

struct Case {
    name: String,
    expected: Vec<String>,
}

/// Parse `samples/generated/manifest.json` into (sample name, expected lines).
fn load_cases() -> Vec<Case> {
    let path = generated_dir().join("manifest.json");
    let raw = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "read {} (run `node scripts/gen-samples.js`): {e}",
            path.display()
        )
    });
    let json: serde_json::Value = serde_json::from_str(&raw).expect("manifest.json is valid JSON");
    let obj = json.as_object().expect("manifest is a JSON object");
    let mut cases: Vec<Case> = obj
        .iter()
        .map(|(name, entry)| {
            let expected = entry["expected"]
                .as_array()
                .expect("expected is an array")
                .iter()
                .map(|v| v.as_str().expect("expected line is a string").to_string())
                .collect();
            Case {
                name: name.clone(),
                expected,
            }
        })
        .collect();
    cases.sort_by(|a, b| a.name.cmp(&b.name));
    cases
}

#[test]
fn deobfuscation_preserves_output_across_obfuscation_profiles() {
    let cases = load_cases();
    assert!(
        cases.len() >= 100,
        "expected a broad generated corpus (≥100), found {} — regenerate with \
         `node scripts/gen-samples.js`",
        cases.len()
    );

    let budget = Duration::from_secs(5);
    let dir = generated_dir();
    let failures: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let non_portable: Mutex<Vec<String>> = Mutex::new(Vec::new());

    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunks: Vec<&[Case]> = cases.chunks(cases.len().div_ceil(nthreads)).collect();

    std::thread::scope(|scope| {
        for chunk in &chunks {
            let (dir, failures, non_portable) = (&dir, &failures, &non_portable);
            scope.spawn(move || {
                for case in *chunk {
                    if KNOWN_BUGS.contains(&case.name.as_str()) {
                        continue; // reported below; excluded from the hard assertion
                    }
                    let src = match fs::read_to_string(dir.join(&case.name)) {
                        Ok(s) => s,
                        Err(e) => {
                            failures
                                .lock()
                                .unwrap()
                                .push(format!("{}: read error {e}", case.name));
                            continue;
                        }
                    };

                    // 1. The obfuscated fixture must itself reproduce the expected
                    //    output in QuickJS — else the seed is non-portable (a
                    //    fixture problem, recorded separately, not a deob failure).
                    let orig = behavior_signature(&src, budget);
                    if orig.outcome != RunOutcome::Completed || orig.output != case.expected {
                        non_portable.lock().unwrap().push(format!(
                            "{}: {:?} {:?}",
                            case.name, orig.outcome, orig.output
                        ));
                        continue;
                    }

                    // 2. Deobfuscate and require identical observable output.
                    let deob = deobfuscate(&src, &case.name);
                    if !deob.parse_errors.is_empty() {
                        failures.lock().unwrap().push(format!(
                            "{}: deob parse errors {:?}",
                            case.name, deob.parse_errors
                        ));
                        continue;
                    }
                    let sig = behavior_signature(&deob.code, budget);
                    if sig.outcome != RunOutcome::Completed || sig.output != case.expected {
                        failures.lock().unwrap().push(format!(
                            "{}: deob output changed\n   expected: {:?}\n   got:      {:?} {:?}",
                            case.name, case.expected, sig.outcome, sig.output
                        ));
                        continue;
                    }

                    // 3. Idempotent: a second pass is a no-op.
                    let again = deobfuscate(&deob.code, &case.name);
                    if again.code != deob.code {
                        failures.lock().unwrap().push(format!(
                            "{}: not idempotent (second pass changed output)",
                            case.name
                        ));
                    }
                }
            });
        }
    });

    let failures = failures.into_inner().unwrap();
    let non_portable = non_portable.into_inner().unwrap();

    // A non-portable seed is a fixture bug — surface it loudly but separately, and
    // require the vast majority of the corpus to stay testable (so a broken seed
    // can't silently shrink the net).
    if !non_portable.is_empty() {
        eprintln!(
            "\n{} non-portable fixture(s) (seed runs differently in QuickJS — fix the seed):",
            non_portable.len()
        );
        for n in &non_portable {
            eprintln!("  · {n}");
        }
    }
    assert!(
        non_portable.len() <= cases.len() / 10,
        "too many non-portable fixtures ({}/{}) — the corpus is unreliable",
        non_portable.len(),
        cases.len()
    );

    assert!(
        failures.is_empty(),
        "deobfuscation changed behavior on {} generated sample(s):\n{}",
        failures.len(),
        failures.join("\n")
    );

    let known: Vec<&str> = cases
        .iter()
        .map(|c| c.name.as_str())
        .filter(|n| KNOWN_BUGS.contains(n))
        .collect();
    if !known.is_empty() {
        eprintln!(
            "\n⚠ {} KNOWN-BUG sample(s) excluded (open deobfuscator soundness bug — \
             see issues/13): {known:?}",
            known.len()
        );
    }

    eprintln!(
        "generated-corpus: {} samples behavior-checked ({} skipped non-portable, {} known-bug)",
        cases.len() - non_portable.len() - known.len(),
        non_portable.len(),
        known.len()
    );
}
