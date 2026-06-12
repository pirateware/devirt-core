//! Opaque-predicate elimination tests. Structural patterns only — never a
//! specific sample.

use devirt_core::deobfuscate;

/// The canonical always-false trap: `"<key>" in <empty fn>`. An empty function
/// declaration that never gains a property has no such key, so the predicate is
/// statically false and the decoy it guards is dead. The fold + short-circuit +
/// DCE must delete the whole `if`, the decoy, and leave behavior intact.
#[test]
fn folds_in_empty_trap_function_and_drops_dead_branch() {
    let src = r#"
        function trap() {}
        if ("N8dRlF2" in trap && (function () { sink("decoy"); return true; })()) {
            sink("dead body");
        }
        sink("live");
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(!out.contains("decoy"), "decoy IIFE kept: {out}");
    assert!(!out.contains("dead body"), "dead branch kept: {out}");
    assert!(out.contains("sink(\"live\")"), "live code lost: {out}");
}

/// Soundness guard: a genuine feature-detection probe against a *real* object
/// (not an empty trap function) must be left untouched — we cannot prove the
/// property absent. `"onLine" in nav` is real branchy logic, not an opaque
/// predicate.
#[test]
fn keeps_feature_detection_against_real_object() {
    let src = r#"
        var nav = navigator;
        if ("onLine" in nav) {
            sink("supported");
        }
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("\"onLine\" in nav"), "real feature-detect folded: {out}");
    assert!(out.contains("sink(\"supported\")"), "live branch lost: {out}");
}

/// Soundness guard: a built-in property that genuinely *is* `in` a function
/// (`length`, `call`, `constructor`, …) must NOT fold to false — `"call" in fn`
/// is true, so the guarded branch is live.
#[test]
fn keeps_builtin_property_probe_on_function() {
    let src = r#"
        function trap() {}
        if ("call" in trap) {
            sink("has call");
        }
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("sink(\"has call\")"), "live builtin-prop branch lost: {out}");
}
