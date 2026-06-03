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
fn numeric_decoder_result_stays_numeric() {
    // A decoder that returns a *number* must be spliced as a numeric literal,
    // not a string — otherwise `dec(i) + 1` flips from arithmetic to concat.
    let src = r#"
        (function () {
            var tbl = [10, 20, 30];
            function d(i) { return tbl[i]; }
            sink(d(1) + 1);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // 20 + 1 must fold to 21 (numeric), never "201" (string concat) or "20".
    assert!(out.contains("sink(21)"), "numeric decode mis-typed as string: {out}");
    assert!(!out.contains("\"20\""), "spliced a string literal for a number: {out}");
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
fn lifts_decoder_nested_in_a_function_scope() {
    // The Tier-C/D shape: the decoder + table live inside a *function body*, which
    // `collect_top_level` never descends into (unlike an expression-statement
    // IIFE). Scoped lifting must still resolve `dec(K)` to its literal.
    let src = r#"
        function setup() {
            var tbl = ["alpha", "beta", "gamma"];
            function dec(i) { return tbl[i]; }
            sink(dec(0), dec(2));
        }
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("\"alpha\"") && out.contains("\"gamma\""), "nested decoder not lifted: {out}");
    assert!(!out.contains("dec("), "decoder call survived in nested scope: {out}");
}

#[test]
fn lifts_decoder_nested_in_var_assigned_iife() {
    // Exactly the DataDome wall: the library lives inside `var X = (() => {…})()`,
    // a VariableDeclaration whose initializer `collect_top_level` cannot reach.
    let src = r#"
        var R = (function () {
            var tbl = ["createElement", "appendChild"];
            function dec(i) { return tbl[i]; }
            return win[dec(0)](dec(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("win.createElement"), "computed member not resolved: {out}");
    assert!(out.contains("\"appendChild\""), "argument decode not lifted: {out}");
    assert!(!out.contains("dec("), "decoder call survived: {out}");
}

#[test]
fn does_not_lift_scoped_decoder_over_member_mutated_table() {
    // The hazard scoped lifting widens (plan §5.3 / §8): the table is mutated by a
    // member write, which the name-based "assigned once" count is blind to. The
    // symbol-precise escape gate must refuse to bake it, leaving the decoder calls
    // in place rather than resolving them against a stale table.
    let src = r#"
        function run() {
            var tbl = ["a", "b", "c"];
            function dec(i) { return tbl[i]; }
            tbl[1] = "Z";
            sink(dec(0), dec(1));
        }
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The decoder calls must survive unresolved: the table is mutated, so a baked
    // ["a","b","c"] would mis-resolve dec(1) (whose slot is overwritten to "Z").
    assert!(out.contains("dec(0)") && out.contains("dec(1)"),
        "decoder over a member-mutated table must be left in place: {out}");
    assert!(out.contains("tbl[1] = \"Z\""), "the mutation must survive: {out}");
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
