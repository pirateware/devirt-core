//! Pure-call constant evaluation (src/passes/pure_eval.rs). Structural shape only;
//! value preservation (incl. the impure-receiver decline) lives in
//! tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    r.code
}

#[test]
fn pure_method_call_on_literal_folds() {
    assert_eq!(deob("x(['H','e','l','l','o'].join(''));").trim(), r#"x("Hello");"#);
    assert_eq!(deob("x('hello'.toUpperCase());").trim(), r#"x("HELLO");"#);
}

#[test]
fn single_use_string_receiver_folds() {
    // Single-use inlining feeds the literal receiver, which then collapses.
    let out = deob("var s = 'abc'; sink(s.toUpperCase());");
    assert!(out.contains(r#""ABC""#), "method call not folded: {out}");
    assert!(!out.contains("toUpperCase"), "call survived: {out}");
}

#[test]
fn unknown_receiver_method_call_is_kept() {
    // The receiver is an unknown host value (`window`, a free name outside the
    // side-effect-free globals), so the gate declines: the call survives untouched.
    let out = deob("sink(window.parts.join(','));");
    assert!(out.contains("join"), "unknown-receiver call wrongly folded: {out}");
}
