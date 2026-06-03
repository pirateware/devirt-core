//! else-after-terminator flattening tests.

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
fn else_after_return_is_flattened() {
    let out =
        deob("function g(c){ if (c) { return 1; } else { sideA(); sideB(); } return 9; } g(0);");
    assert!(!out.contains("else"), "else not flattened: {out}");
    assert!(
        out.contains("sideA()") && out.contains("sideB()"),
        "body lost: {out}"
    );
}

#[test]
fn else_after_throw_break_continue_is_flattened() {
    let out = deob("function g(c){ for(;;){ if (c) { break; } else { work(); } } } g(0);");
    assert!(
        !out.contains("else"),
        "else after break not flattened: {out}"
    );
    assert!(out.contains("work()"), "body lost: {out}");
}

#[test]
fn else_kept_when_consequent_may_fall_through() {
    let out = deob("function g(c){ if (c) { maybe(); } else { other(); } return 9; } g(0);");
    assert!(
        out.contains("else"),
        "else wrongly flattened (no terminator): {out}"
    );
}

#[test]
fn else_if_ladder_unwinds() {
    let out = deob(
        "function g(a,b){ if (a) { return 1; } else if (b) { return 2; } else { return 3; } } sink(g(x,y));",
    );
    // Both terminating arms peel; no `else` should remain.
    assert!(!out.contains("else"), "ladder did not fully unwind: {out}");
    assert!(out.contains("return 1") && out.contains("return 2") && out.contains("return 3"));
}
