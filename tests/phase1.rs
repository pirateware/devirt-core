//! Phase 1 behavioral tests + the cross-cutting safety invariant (idempotence).
//! These assert on *behavior shape*, never on a specific sample — the pipeline
//! must stay sample-agnostic.

use js_deobfuscator::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn member_normalization() {
    let out = deob(r#"a["foo"]; a["1bad"]; a["with"]();"#);
    assert!(out.contains("a.foo"));
    assert!(out.contains("a.with()"));
    assert!(out.contains(r#"a["1bad"]"#)); // invalid identifier untouched
}

#[test]
fn member_normalization_on_assignment_target() {
    // The LHS of an assignment is a SimpleAssignmentTarget, not an Expression;
    // it must normalize just like read-position member access.
    // (param name may be renamed; assert on the access form, not the name.)
    let out = deob(r#"function g(o){ o["foo"] = o["foo"] || 1; return o; }"#);
    assert!(out.contains(".foo = "), "LHS not normalized: {out}");
    assert!(!out.contains(r#"["foo"]"#), "bracket access remained: {out}");
}

#[test]
fn assigned_once_inline_and_fold() {
    let out = deob("function g(){ var n = (0x10, 256); return 5 + n; }");
    assert!(out.contains("261"), "got: {out}");
    assert!(!out.contains("var n"), "declaration should be DCE'd: {out}");
}

#[test]
fn opaque_predicates() {
    let out = deob("if (!![]) { keep(); } if (![]) { drop(); }");
    assert!(out.contains("keep()"));
    assert!(!out.contains("drop()"), "dead branch survived: {out}");
}

#[test]
fn unreachable_after_return() {
    let out = deob("function g(){ return 1; sideEffect(); }");
    assert!(!out.contains("sideEffect"), "unreachable code survived: {out}");
}

#[test]
fn mutated_var_not_inlined() {
    // x is reassigned, so it must NOT be treated as a constant: the return
    // value stays a variable, not a folded literal. (Name-agnostic: rename may
    // have changed `x`.)
    let out = deob("function g(){ var x = 1; x = 2; return x; }");
    assert!(out.contains("= 2"), "must preserve reassignment: {out}");
    assert!(!out.contains("return 1") && !out.contains("return 2"), "must not constant-fold a mutated var: {out}");
}

#[test]
fn idempotent_on_nested_obfuscation() {
    let src = "function f(){ var a = 1, b = 2; if (!!1) return a + b; }";
    let once = deobfuscate(src, "t.js").code;
    let twice = deobfuscate(&once, "t.js").code;
    assert_eq!(once, twice, "pipeline is not idempotent");
}
