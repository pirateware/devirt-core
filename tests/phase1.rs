//! Phase 1 behavioral tests + the cross-cutting safety invariant (idempotence).
//! These assert on *behavior shape*, never on a specific sample — the pipeline
//! must stay sample-agnostic.

use js_deobfuscator::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(
        r.parse_errors.is_empty(),
        "parse errors: {:?}",
        r.parse_errors
    );
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
    assert!(
        !out.contains(r#"["foo"]"#),
        "bracket access remained: {out}"
    );
}

#[test]
fn assigned_once_inline_and_fold() {
    let out = deob("function g(){ var n = (0x10, 256); return 5 + n; }");
    assert!(out.contains("261"), "got: {out}");
    assert!(!out.contains("var n"), "declaration should be DCE'd: {out}");
}

#[test]
fn string_concatenation_folds() {
    let out = deob(r#"sink("foo" + "bar" + "baz", "n" + 5);"#);
    assert!(
        out.contains(r#""foobarbaz""#),
        "string concat not folded: {out}"
    );
    assert!(
        out.contains(r#""n5""#),
        "mixed string+number concat not folded: {out}"
    );
}

#[test]
fn void_literal_folds_to_undefined() {
    let out = deob("f(void 0, a === void 0);");
    assert!(out.contains("f(undefined, a === undefined)"), "got: {out}");
    assert!(!out.contains("void 0"), "void 0 survived: {out}");
}

#[test]
fn void_not_folded_when_undefined_is_shadowed() {
    // A local binding named `undefined` shadows the global; rewriting `void 0`
    // to `undefined` here would be unsound.
    let out = deob("function f(undefined){ return void 0; }");
    assert!(
        out.contains("void 0"),
        "unsoundly folded shadowed undefined: {out}"
    );
}

#[test]
fn constant_ternary_folds_to_taken_branch() {
    let out = deob("f(true ? 10 : 0, false ? a : b());");
    assert!(
        out.contains("f(10, b())"),
        "constant ternary not folded: {out}"
    );
}

#[test]
fn opaque_predicates() {
    let out = deob("if (!![]) { keep(); } if (![]) { drop(); }");
    assert!(out.contains("keep()"));
    assert!(!out.contains("drop()"), "dead branch survived: {out}");
}

#[test]
fn dead_literal_statements_removed() {
    let out = deob(r#"function g(){ 80214130; true; null; keep(); }"#);
    assert!(out.contains("keep()"), "real statement dropped: {out}");
    assert!(
        !out.contains("80214130"),
        "dead numeric literal survived: {out}"
    );
    assert!(
        !out.contains("true;") && !out.contains("null;"),
        "dead literal survived: {out}"
    );
}

#[test]
fn unreachable_after_return() {
    let out = deob("function g(){ return 1; sideEffect(); }");
    assert!(
        !out.contains("sideEffect"),
        "unreachable code survived: {out}"
    );
}

#[test]
fn reassigned_var_folds_to_last_dominating_value() {
    // The SSA dataflow pass resolves `return x` to its single reaching
    // definition (`x = 2` dominates the return), so the read folds to 2 and the
    // dead assignments are removed. assigned-once alone bailed on the
    // reassignment; SSA makes this sound by construction.
    let out = deob("function g(){ var x = 1; x = 2; return x; }");
    assert!(
        out.contains("return 2"),
        "should fold to the dominating value: {out}"
    );
}

#[test]
fn ambiguous_reassignment_not_folded() {
    // The reaching definition at the return is a phi (1 on the fall-through
    // path, 2 on the branch), so it must NOT fold to either literal.
    let out = deob("function g(c){ var x = 1; if (c) { x = 2; } return x; }");
    assert!(
        !out.contains("return 1") && !out.contains("return 2"),
        "merge wrongly folded: {out}"
    );
}

#[test]
fn idempotent_on_nested_obfuscation() {
    let src = "function f(){ var a = 1, b = 2; if (!!1) return a + b; }";
    let once = deobfuscate(src, "t.js").code;
    let twice = deobfuscate(&once, "t.js").code;
    assert_eq!(once, twice, "pipeline is not idempotent");
}
