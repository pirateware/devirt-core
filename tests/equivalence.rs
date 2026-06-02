//! Differential *semantic* equivalence tests.
//!
//! The samples test proves the output re-parses and is idempotent, but not that
//! it computes the same thing. Here we execute a battery of small obfuscated
//! snippets and their deobfuscated forms in a fresh QuickJS context and require
//! the observable result to match exactly. This is the real soundness net for
//! every value-changing pass (const-fold, void/string folding, sequence split,
//! dead-literal DCE, decoder lift): a pass that alters behavior fails here.

use js_deobfuscator::deobfuscate;
use js_deobfuscator::sandbox::{EvalValue, Sandbox};

/// Evaluate `program` (an expression-returning IIFE body wrapper) and return its
/// primitive result. Each call uses a fresh sandbox for isolation.
fn eval(program: &str) -> Option<EvalValue> {
    let sb = Sandbox::new().expect("sandbox");
    sb.eval_to_value(program)
}

/// Assert that deobfuscating `src` preserves the value of the wrapped program.
/// `src` must be a sequence of statements ending by assigning to `__r`.
fn assert_equivalent(src: &str) {
    // Tag the result with its type so `undefined` is observable (eval_to_value
    // maps a bare undefined to None, same as an error) and so a number vs. the
    // string of that number is never conflated — exactly the soundness we care
    // about (a numeric decode must not become a string, etc.).
    let wrap =
        |body: &str| format!("(function(){{ var __r; {body}; return typeof __r + \":\" + String(__r); }})()");
    let original = wrap(src);

    let deob = deobfuscate(src, "t.js");
    assert!(deob.parse_errors.is_empty(), "deob parse errors: {:?}", deob.parse_errors);
    let deobbed = wrap(&deob.code);

    let a = eval(&original);
    let b = eval(&deobbed);
    assert!(a.is_some(), "original did not evaluate: {original}");
    assert_eq!(a, b, "value changed by deobfuscation.\n--- src ---\n{src}\n--- deob ---\n{}", deob.code);
}

#[test]
fn const_and_void_folding_preserve_value() {
    assert_equivalent("__r = (0x10 + 256) * 2;");
    assert_equivalent("__r = void 0;");
    assert_equivalent("__r = (a === void 0) === (a === undefined); var a;");
    assert_equivalent("__r = !![] && !0 && !1 === false;");
}

#[test]
fn string_concatenation_preserves_value() {
    assert_equivalent(r#"__r = "foo" + "bar" + "baz";"#);
    assert_equivalent(r#"__r = "n" + 5 + "!";"#);
    // The classic trap: a numeric-looking concat must stay a string.
    assert_equivalent(r#"__r = "1" + "0" + "0";"#);
}

#[test]
fn sequence_split_preserves_value() {
    assert_equivalent("var x; x = 1, x += 2, __r = x * 10;");
    // Return-position sequence: value is the last operand.
    assert_equivalent("__r = (function(){ return sideA(), sideB(), 42; function sideA(){} function sideB(){} })();");
}

#[test]
fn numeric_decoder_arithmetic_preserves_value() {
    // A numeric decoder feeding arithmetic — the type-tagged splice must keep
    // it numeric (21), not turn it into the string "201".
    assert_equivalent(
        r#"
        var tbl = [10, 20, 30];
        function d(i) { return tbl[i]; }
        __r = d(1) + 1;
        "#,
    );
}

#[test]
fn string_decoder_preserves_value() {
    assert_equivalent(
        r#"
        var arr = ["lo", "HELLO", "wo"];
        function d(i) { return arr[i]; }
        __r = d(1) + d(0) + d(2);
        "#,
    );
}

#[test]
fn object_method_proxy_preserves_value() {
    assert_equivalent(
        r#"
        var O = {
            lt: function(a, b) { return a < b; },
            an: function(a, b) { return a & b; },
            mul: function(a, b) { return a * b; }
        };
        var x = 6, y = 10;
        __r = "" + O.lt(x, y) + ":" + O.an(x, 3) + ":" + O.mul(x, y);
        "#,
    );
}

#[test]
fn apply_proxy_preserves_value_and_receiver() {
    // Apply proxy with a plain function: inlinable, value preserved.
    assert_equivalent(
        r#"
        var O = { ap: function(f, x){ return f(x); } };
        function inc(n){ return n + 1; }
        __r = O.ap(inc, 9);
        "#,
    );
    // Apply proxy with a METHOD argument: the proxy detaches `this`, so the
    // result must stay whatever the detached call yields. The pass must not
    // re-bind the receiver (it declines to inline); either way values match.
    assert_equivalent(
        r#"
        var obj = { v: 42, get: function(){ return this === obj ? "BOUND" : "free"; } };
        var O = { ap: function(f){ return f(); } };
        __r = O.ap(obj.get);
        "#,
    );
}

#[test]
fn dead_literal_removal_preserves_value() {
    assert_equivalent("80214130; true; null; __r = 7;");
}

#[test]
fn member_normalization_preserves_value() {
    assert_equivalent(r#"var o = {}; o["foo"] = 3; __r = o["foo"] + o.foo;"#);
}
