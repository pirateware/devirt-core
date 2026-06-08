//! Transparent bare-block flattening (src/passes/block_flatten.rs). Structural
//! shape only; value preservation (incl. the `let`/`const` decline) lives in
//! tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn constant_if_collapse_leaves_no_bare_block_or_dead_tail() {
    // The opaque-predicate shape: a constant-true guard collapses to its branch.
    // Block-flatten must dissolve the residual block so DCE can drop the now-dead
    // tail, leaving clean straight-line code.
    let out = deob(
        "function g(n){ var a = 7; if ((a * a - 49) === 0) { return n * 2; } \
         else { return dead(); } } sink(g(window.v));",
    );
    assert!(!out.contains('{') || !out.contains("\n\t{"), "bare block survived: {out}");
    assert!(!out.contains("dead()"), "unreachable tail survived: {out}");
    assert!(out.contains("* 2"), "live branch lost: {out}");
}

#[test]
fn var_only_block_is_dissolved() {
    let out = deob("{ var a = 1; sideEffect(a); }");
    assert!(!out.contains('{'), "transparent var block not flattened: {out}");
    assert!(out.contains("sideEffect"), "body lost: {out}");
}

#[test]
fn lexical_block_is_kept() {
    // A block introducing a `let` is NOT transparent — dissolving it would change
    // scoping (or collide with an outer binding). It must survive.
    let out = deob("let a = 1; { let a = 2; sink(a); } sink(a);");
    assert!(out.contains('{'), "lexical block wrongly flattened: {out}");
}
