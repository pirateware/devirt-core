//! Array-literal destructuring split (src/passes/destructure_split.rs). Structural
//! shape; value preservation lives in tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    r.code
}

#[test]
fn array_literal_destructuring_splits_and_folds() {
    assert_eq!(deob("sink([10, 20]); var [a, b] = [10, 20]; sink(a + b);").trim().lines().last().unwrap(), "sink(30);");
}

#[test]
fn unsound_shapes_are_left_intact() {
    // Hole + rest.
    assert!(deob("var [a, , ...r] = [1, 2, 3, 4]; sink(a);").contains('['), "rest/hole split");
    // Length mismatch (a dropped element could carry a side effect).
    assert!(deob("var [a, b] = [1, 2, 3]; sink(a);").contains("1,"), "length-mismatch split");
}

#[test]
fn object_literal_destructuring_splits() {
    assert_eq!(deob("var {a, b} = { a: 1, b: 2 }; sink(a + b);").trim().lines().last().unwrap(), "sink(3);");
    assert_eq!(deob("var { a: x, b: y } = { a: 10, b: 20 }; sink(x + y);").trim().lines().last().unwrap(), "sink(30);");
}

#[test]
fn unsound_object_shapes_are_kept() {
    // Impure value: dropping/reordering would lose the effect.
    assert!(deob("function f(){ var {a}={a:1,b:side()}; return a; } sink(f());").contains("side()"), "impure-value split");
    // Missing key would bind undefined.
    assert!(deob("var {a,c}={a:1,b:2}; sink(c);").contains('{'), "missing-key split");
}
