//! Constant-selector de-multiplexer tests. Structural patterns only.

use devirt_core::deobfuscate;

#[test]
fn specializes_constant_selector_multiplexer() {
    // A constant-selector call into an immutable function array collapses to the
    // live branch once the selector is bound and const-folded.
    let src = r#"
        var A = [
            function (s, a, b) {
                return ((s - 1 | 15) >= s && s < 100) ? a + b : a - b;
            }
        ];
        sink(A[0](46, 10, 4));
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The guard `(46-1|15)>=46 && 46<100` is true → the live branch `a + b` runs;
    // with constant args it folds all the way to 14. The magic selector 46 must be
    // gone from the call (it was specialized into a hoisted function).
    assert!(
        !out.contains("[0](46"),
        "constant-selector call not specialized: {out}"
    );
    assert!(
        out.contains("14") && !out.contains("- 4") && !out.contains("46"),
        "live branch not folded through specialization: {out}"
    );
}

#[test]
fn does_not_specialize_mutable_array() {
    // An element write makes the array mutable: the slot called is not statically
    // known, so the call must be left intact (not specialized).
    let src = r#"
        var A = [function (s, a) { return s === 1 ? a * 2 : a + 1; }];
        A[0] = function (s, a) { return a; };
        sink(A[0](1, 5));
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The call keeps its constant selector (1) — no specialization occurred.
    assert!(
        out.contains("(1, 5)"),
        "mutable-array call must not be specialized: {out}"
    );
}
