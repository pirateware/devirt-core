//! Single-iteration loop unwrapping (src/passes/single_iter_loop.rs). Structural
//! shape; value preservation lives in tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    r.code
}

#[test]
fn while_true_break_wrapper_is_unwrapped() {
    let out = deob("var r = 0; while (1) { r = inp() * 2; break; } sink(r);");
    assert!(!out.contains("while"), "single-iteration loop survived: {out}");
    assert!(out.contains("inp()"), "body lost: {out}");
}

#[test]
fn switch_break_inside_is_preserved() {
    let out = deob("function f(n){ var r = 0; while (true) { switch (n) { case 1: r = 1; break; default: r = 2; } break; } return r; } sink(f(inp()));");
    assert!(!out.contains("while"), "wrapper survived: {out}");
    assert!(out.contains("switch"), "inner switch lost: {out}");
}

#[test]
fn looping_body_is_kept() {
    // A `continue` makes it a real loop.
    assert!(deob("var i = 0, r = 0; while (1) { i++; if (i < 3) continue; r = i; break; } sink(r);").contains("while"), "continue loop unwrapped");
    // A second (conditional) break exits at a different point.
    assert!(deob("var r = 0; while (1) { r = 1; if (inp()) break; r = 2; break; } sink(r);").contains("while"), "early-break loop unwrapped");
}
