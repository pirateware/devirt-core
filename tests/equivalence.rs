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
fn proxy_inline_keeps_effect_order() {
    // The proxy body uses params in reversed order; inlining two effectful args
    // would swap their effects. Result must observe `f` before `g`.
    assert_equivalent(
        r#"
        var ord = "";
        function f(){ ord += "f"; return 1; }
        function g(){ ord += "g"; return 2; }
        function sub(a, b){ return b - a; }
        var v = sub(f(), g());
        __r = v + ":" + ord;
        "#,
    );
}

#[test]
fn decoder_lift_keeps_method_mutation() {
    // A "decoder"-shaped function that mutates a free array via `push` must not be
    // lifted away — the mutation is observable.
    assert_equivalent(
        r#"
        var sink = [];
        function tag(i){ sink.push(i); return i + 1; }
        var a = tag(10), b = tag(20);
        __r = a + ":" + b + ":" + sink.length;
        "#,
    );
}

#[test]
fn proxy_inline_keeps_unused_arg_side_effect() {
    // Param `a` is never used in the body, but `(seen = 1)` must still run.
    assert_equivalent(
        r#"
        var seen = 0;
        function p(a, b) { return b; }
        var v = p((seen = 1), 5);
        __r = v + ":" + seen;
        "#,
    );
}

#[test]
fn call_apply_proxy_forms_preserve_value() {
    assert_equivalent(
        r#"
        var O = { add: function(a,b){ return a + b; }, mul: function(a,b){ return a * b; } };
        __r = O.add.call(null, 6, 7) + ":" + O.mul.apply(null, [3, 4]);
        "#,
    );
}

#[test]
fn flatten_else_preserves_value() {
    assert_equivalent(
        r#"
        function classify(n){
            if (n < 0) { return "neg"; }
            else { if (n === 0) { return "zero"; } else { return "pos"; } }
        }
        __r = classify(-5) + classify(0) + classify(7);
        "#,
    );
    // break/continue terminators inside a loop.
    assert_equivalent(
        r#"
        var acc = 0;
        for (var i = 0; i < 5; i++) {
            if (i === 3) { continue; } else { acc += i; }
        }
        __r = acc;
        "#,
    );
}

#[test]
fn dead_object_prop_removal_preserves_value() {
    assert_equivalent(
        r#"
        var O = { used: 7, deadFn: function(){ return 99; }, deadData: 42 };
        __r = O.used;
        "#,
    );
    // Object escapes via computed access → must keep all members & behavior.
    assert_equivalent(
        r#"
        var O = { a: 1, b: 2 };
        var k = "b";
        __r = O[k] + O.a;
        "#,
    );
    // Assignment-initialized table (the param=`{…}` form).
    assert_equivalent(
        r#"
        var O;
        O = { keep: 5, lt: function(a,b){ return a < b; }, deadData: 9 };
        __r = O.keep;
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

#[test]
fn deletion_passes_must_not_drop_side_effects() {
    // DCE must not drop an unused binding whose initializer has a side effect.
    assert_equivalent("var hit = 0; var unused = (hit = 1); __r = hit;");
    // Dead-store removal must keep a store whose RHS is impure (a call).
    assert_equivalent(
        r#"
        var log = 0;
        function bump(){ log++; return 1; }
        var dead;
        dead = bump(), dead = bump();
        __r = log;
        "#,
    );
    // An unreachable tail with effects is fine to drop (it never runs), but the
    // reachable prefix's effects must survive.
    assert_equivalent("__r = (function(){ var s = 0; s = 5; return s; never(); })();");
}

#[test]
fn fold_must_not_drop_embedded_side_effects() {
    // The real-sample shape: an obfuscator hides a side-effecting assignment
    // inside an arithmetic expression that evaluates to a constant. oxc's
    // `evaluate_value` reports the constant but folding to it would delete the
    // assignment. `out` must survive — and be observable afterwards.
    assert_equivalent("var out = 0, v = 0; v << ((v | (out = 7)) & 15); __r = out;");
    // Sequence whose last operand is a constant but whose first has an effect.
    assert_equivalent("var hit = 0; (hit = 1, 42); __r = hit;");
    // Conditional/if tests with a side effect must keep running the test.
    assert_equivalent("var t = 0; var __x = (t = 5, 1) ? 'a' : 'b'; __r = __x + t;");
    assert_equivalent("var t = 0; if ((t = 9, 1)) { __r = 'y'; } __r = __r + t;");
}

#[test]
fn literal_member_fold_preserves_value() {
    // Array index / length on a literal.
    assert_equivalent("__r = [10, 20, 30][1];");
    assert_equivalent("__r = [10, 20, 30].length;");
    // Object property on a literal — static and computed key forms.
    assert_equivalent("__r = ({ a: 7, b: 8 }).b;");
    assert_equivalent(r#"__r = ({ a: 7, b: 8 })["a"];"#);
    // String index / length — value must stay a string, not become a number.
    assert_equivalent(r#"__r = "abc"[1];"#);
    assert_equivalent(r#"__r = "abc".length;"#);
    // The selected element keeps its own type (numeric stays numeric).
    assert_equivalent(r#"__r = ["1", "0", "0"][0] + [1, 2][1];"#);
    // Nested: index into an array literal whose element is another literal.
    assert_equivalent("__r = [[1, 2], [3, 4]][1][0];");
    // Out-of-range and holes are left as `undefined` — still must match.
    assert_equivalent("__r = String([5, 6][9]);");
    assert_equivalent("__r = String([1, , 3][1]);");
    // A non-selected sibling with a side effect must NOT be folded away: index 0
    // sets `seen`, so the fold must decline and keep that effect. Equivalence
    // holds precisely because we refuse to drop the impure sibling.
    assert_equivalent("var seen = 0; __r = [ (seen = 1, 'x'), 'y' ][1]; __r = __r + seen;");
}
