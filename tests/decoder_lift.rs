//! Decoder-lifting tests. Structural patterns only — never a specific sample.

use js_deobfuscator::deobfuscate;

#[test]
fn lifts_string_array_decoder_with_rotation() {
    // Classic shape: array + indexer + (no-op) rotation IIFE, calls everywhere.
    let src = r#"
        (function () {
            var arr = ["log", "Hello", "warn"];
            function d(i) { return arr[i]; }
            (function (a, b) { while (true) { if (a === b) break; arr.push(arr.shift()); } })(0, 0);
            console[d(0)](d(1));
            console[d(2)]("x");
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("console.log(\"Hello\")"), "got: {out}");
    assert!(out.contains("console.warn(\"x\")"), "got: {out}");
}

#[test]
fn does_not_lift_impure_decoder() {
    // "Decoder" touches the host (window) → must not be evaluated/lifted.
    let src = r#"
        (function () {
            function d(i) { return window.secret[i]; }
            foo(d(0));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The host access must survive and no string literal may replace the call.
    assert!(out.contains("window") && out.contains("secret"), "impure decoder body lost: {out}");
    assert!(!out.contains("\"\"") && !out.contains("foo(\""), "impure call was wrongly lifted: {out}");
}

#[test]
fn does_not_lift_mutating_library_calls() {
    // `arr.push(arr.shift())` mutates a library array; it must NOT be folded to
    // its (length-preserving, deterministic) return value, which would destroy
    // the rotation and corrupt every subsequent decode.
    let src = r#"
        (function () {
            var arr = ["log", "Hello", "warn"];
            function d(i) { return arr[i]; }
            (function (a, b) { while (true) { if (a === b) break; arr.push(arr.shift()); } })(1, 1);
            console[d(0)](d(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("arr.push(arr.shift())"), "rotation was wrongly lifted: {out}");
    assert!(out.contains("console.log(\"Hello\")"), "decode corrupted: {out}");
}

#[test]
fn lifts_computed_index_into_pure_array() {
    // `tbl[K.a]` is a pure read into a library array — safe to lift.
    let src = r#"
        (function () {
            var tbl = ["alpha", "beta", "gamma"];
            var K = { a: 0, b: 2 };
            sink(tbl[K.a], tbl[K.b]);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("\"alpha\"") && out.contains("\"gamma\""), "index read not lifted: {out}");
}

#[test]
fn lifts_object_keyed_arguments() {
    // Argument is a member of a pure object literal — still fully evaluable.
    let src = r#"
        (function () {
            var K = { a: 0, b: 1 };
            var tbl = ["alpha", "beta"];
            function d(i) { return tbl[i]; }
            sink(d(K.a), d(K.b));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("\"alpha\"") && out.contains("\"beta\""), "got: {out}");
}
