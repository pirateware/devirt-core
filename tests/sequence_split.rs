//! Sequence-expression splitting tests. Structural patterns only.

use js_deobfuscator::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge (sequence split likely not idempotent)");
    r.code
}

#[test]
fn statement_level_comma_chain_splits() {
    let out = deob("function g(){ a = f(), b(), c = d; }");
    assert!(out.contains("a = f();"), "got: {out}");
    assert!(out.contains("b();"), "got: {out}");
    assert!(out.contains("c = d;"), "got: {out}");
    // No top-level comma operator should remain in statement position.
    assert!(!out.contains("(), "), "sequence not fully split: {out}");
}

#[test]
fn return_sequence_keeps_last_as_value() {
    let out = deob("function g(){ return j(), k(), m; }");
    assert!(out.contains("j();") && out.contains("k();"), "leading effects lost: {out}");
    assert!(out.contains("return m;"), "return value changed: {out}");
}

#[test]
fn throw_sequence_keeps_last_as_value() {
    let out = deob("function g(){ throw p(), q; }");
    assert!(out.contains("p();"), "leading effect lost: {out}");
    assert!(out.contains("throw q;"), "thrown value changed: {out}");
}

#[test]
fn assignment_rhs_sequence_is_not_split() {
    // `a = (b, c)` is one assignment whose value is the sequence; the comma is
    // NOT in statement position and must stay intact.
    let out = deob("function g(){ a = (sideEffect(), value); return a; }");
    assert!(out.contains("(sideEffect(), value)") || out.contains("sideEffect(), value"),
        "RHS sequence wrongly split: {out}");
}
