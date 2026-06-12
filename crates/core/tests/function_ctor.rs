//! `Function` constructor reconstruction (src/passes/function_ctor.rs). Structural
//! shape only; value preservation (incl. the local-capture decline) lives in
//! tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    r.code
}

#[test]
fn constant_function_constructor_is_reconstructed() {
    // The reconstructed function is inlined and the call folded.
    assert_eq!(deob("sink(new Function('a','b','return a+b')(2,3));").trim(), "sink(5);");
}

#[test]
fn local_capture_function_constructor_is_kept() {
    // The body references a name that is local at the splice site; reconstructing a
    // closure would capture it, so the constructor must be left intact.
    let out = deob("function g(){ var localX = 5; return new Function('return localX')(); } sink(g());");
    assert!(out.contains("new Function") || out.contains("Function("), "wrongly reconstructed: {out}");
}

#[test]
fn body_breakout_is_rejected() {
    // A body crafted to break out of the wrapper must not splice injected code.
    // It parses as multiple statements, so reconstruction declines.
    let out = deob(r#"sink(new Function("}); stolen(); ({")());"#);
    // Rejected: the constructor survives verbatim (the breakout text stays quoted
    // inside the string argument, never spliced as a statement).
    assert!(out.contains("new Function("), "breakout body wrongly reconstructed: {out}");
}
