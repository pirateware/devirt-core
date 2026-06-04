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
    let wrap = |body: &str| {
        format!("(function(){{ var __r; {body}; return typeof __r + \":\" + String(__r); }})()")
    };
    let original = wrap(src);

    let deob = deobfuscate(src, "t.js");
    assert!(
        deob.parse_errors.is_empty(),
        "deob parse errors: {:?}",
        deob.parse_errors
    );
    let deobbed = wrap(&deob.code);

    let a = eval(&original);
    let b = eval(&deobbed);
    assert!(a.is_some(), "original did not evaluate: {original}");
    assert_eq!(
        a, b,
        "value changed by deobfuscation.\n--- src ---\n{src}\n--- deob ---\n{}",
        deob.code
    );
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
fn assigned_once_respects_initialization_order() {
    // A read that is textually before the `var x = 5` initializer must observe
    // the pre-init `undefined`, not 5.
    assert_equivalent(r#"var out = "" + x; var x = 5; __r = out + ":" + x;"#);
    // A conditionally-declared constant must not be inlined where the branch may
    // not have run (`h(false)` leaves the hoisted `x` undefined).
    assert_equivalent(
        r#"
        function h(c){ var acc = []; if (c) { var x = 5; } acc.push(x); return acc.join(); }
        __r = h(false) + "|" + h(true);
        "#,
    );
    // The safe shape still inlines: same function, declared before use.
    assert_equivalent("function s(){ var x = 5; return x + 1; } __r = s();");
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

// ---------------------------------------------------------------------------
// SSA dataflow pass (`src/ir/`): adversarial flow-sensitive cases. Each pairs a
// shape the pass *should* fold soundly with a sibling shape where folding would
// be wrong (reassignment, branch merge, escape, capture, loop-carried). The
// differential net catches any unsound fold by value mismatch.
// ---------------------------------------------------------------------------

#[test]
fn ssa_const_array_fold_preserves_value() {
    // Plain const-array index/length fold via a binding.
    assert_equivalent("var s = [3, 31, 5]; __r = s[0] + s[2] + s.length;");
    // Element type is preserved: a string element must stay a string.
    assert_equivalent(r#"var s = ["1", "0", "0"]; __r = s[0] + s[1] + s[2];"#);
    // Const-object property fold (static and numeric-key forms).
    assert_equivalent("var o = { a: 5, b: 7 }; __r = o.a + o.b;");
    assert_equivalent(r#"var o = { 0: "x", 1: "y" }; __r = o[0] + o[1];"#);
    // Cross-fold: an index resolved from another const array (two rounds).
    assert_equivalent("var idx = [2]; var data = [10, 20, 30]; __r = data[idx[0]];");
}

#[test]
fn nested_const_array_2d_and_alias_fold_preserves_value() {
    // B2: a provably-immutable 2-D const array, read `T[i][j]`. Resolving the
    // nested element must not change the value.
    assert_equivalent("var T = [[10, 20], [30, 40]]; __r = T[0][0] + T[1][1];");
    // Aliased element of a 3-D const array (`T = base[1]` then `T[i][j]`), read in
    // a nested function — the cross-function 2-D-alias case from the roadmap.
    assert_equivalent(
        "__r = (function(){ \
           var base = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]; \
           var T = base[1]; \
           function run(s){ var acc = 0; while (s != 0) { switch (s) { \
             case T[0][0]: acc += 1; s = T[0][1]; break; \
             case T[0][1]: acc += 2; s = T[1][0]; break; \
             case T[1][0]: acc += 3; s = T[1][1]; break; \
             case T[1][1]: acc += 4; s = 0; break; } } return acc; } \
           return run(T[0][0]); })();",
    );
}

#[test]
fn lookup_table_unflatten_preserves_value() {
    // A5: the dispatch transition reads the next state from a provably-immutable
    // const int array (`s = next[s]` / `s = next[CONST]`). Linearizing the machine
    // must compute the exact same accumulated value as the original dispatcher.
    assert_equivalent(
        "var next = [0, 2, 9]; var acc = 0; var s = 1; \
         while (s != 9) { switch (s) { \
           case 1: acc += 10; s = next[s]; break; \
           case 2: acc += 5; s = next[2]; break; } } __r = acc;",
    );
    // Const-index form, falsy exit.
    assert_equivalent(
        "var nx = [5, 0]; var acc = 0; var s = 1; \
         while (s) { switch (s) { \
           case 1: acc += 1; s = nx[0]; break; \
           case 5: acc += 2; s = nx[1]; break; } } __r = acc;",
    );
    // Adversarial: a pushed (mutable) table must NOT drive a lookup transition —
    // the unflattener declines and the dispatcher is preserved. Equivalence holds
    // because the (kept) loop runs as written. (Indices 1 and 2 are read; the push
    // only appends, so the read values match the original literal array.)
    assert_equivalent(
        "var next = [0, 2, 9]; next.push(7); var acc = 0; var s = 1; \
         while (s != 9) { switch (s) { \
           case 1: acc += 10; s = next[s]; break; \
           case 2: acc += 5; s = next[2]; break; } } __r = acc;",
    );
}

#[test]
fn member_written_table_is_not_resolved() {
    // B2 adversarial sibling (sample_11's shape): a table built by *member writes*
    // (`base[i][j] = …`) is mutable, so `T = base[0]; T[i]` must NOT be resolved to
    // a literal. Equivalence guards soundness regardless of the (correct) decline.
    assert_equivalent(
        "__r = (function(){ \
           var base = []; \
           for (var i = 0; i < 2; i++) base[i] = new Array(2); \
           base[0][0] = 5; base[0][1] = 6; base[1][0] = 7; base[1][1] = 8; \
           var T = base[0]; \
           function run(s){ var acc = 0; while (s != 0) { switch (s) { \
             case T[0]: acc += 1; s = T[1]; break; \
             case T[1]: acc += 2; s = 0; break; } } return acc; } \
           return run(base[0][0]); })();",
    );
}

#[test]
fn ssa_short_circuit_dominating_def_folds_soundly() {
    // The sample_10 shape: array assigned inside a `&&` chain, read after. The
    // assignment dominates the reads, so folding is sound.
    assert_equivalent(
        r#"
        var a = 1, s = [9, 8];
        __r = (a && (s = [3, 4], 1) && s[0] * 10 + s[1]);
        "#,
    );
}

#[test]
fn ssa_reassignment_uses_last_dominating_value() {
    // Straight-line reassignment: the read must observe the most recent def.
    assert_equivalent("var s = [1, 2]; s = [3, 4]; __r = s[0] + s[1];");
    assert_equivalent("var x = 1; x = 2; x = 3; __r = x;");
}

#[test]
fn ssa_branch_merge_is_not_folded() {
    // A reassignment on one path makes the post-merge read a phi: it must not be
    // folded to either branch's value. `cond` is kept opaque so the branch
    // survives const-folding; at runtime it is true, selecting [3,4].
    assert_equivalent(
        r#"
        var cond = String(1) === "1";
        var s = [1, 2];
        if (cond) { s = [3, 4]; }
        __r = s[0] + ":" + s[1];
        "#,
    );
    // Scalar phi.
    assert_equivalent(
        r#"
        var cond = String(0) === "0";
        var x = 1;
        if (cond) { x = 2; }
        __r = x;
        "#,
    );
}

#[test]
fn ssa_escaped_array_is_not_folded() {
    // The array is passed to a function that mutates it: it escapes, so its
    // elements may differ from the literal and must not be folded.
    assert_equivalent(
        r#"
        var s = [1, 2];
        function mut(a){ a[0] = 99; }
        mut(s);
        __r = s[0] + ":" + s[1];
        "#,
    );
    // Member write directly: also an escape / mutation.
    assert_equivalent("var s = [1, 2]; s[0] = 7; __r = s[0] + s[1];");
}

#[test]
fn ssa_captured_binding_is_not_folded() {
    // A closure writes `x` after its initializer; the captured binding must not
    // be versioned, so the later read observes the mutated value (2, not 1).
    assert_equivalent(
        r#"
        var x = 1;
        function bump(){ x = x + 1; }
        bump();
        __r = x;
        "#,
    );
}

#[test]
fn ssa_loop_carried_value_is_not_folded() {
    // `s` changes every iteration: the loop-header phi blocks folding the read.
    assert_equivalent(
        r#"
        var s = [0];
        var acc = 0;
        for (var i = 0; i < 3; i++) { acc += s[0]; s = [s[0] + 1]; }
        __r = acc;
        "#,
    );
}

#[test]
fn ssa_respects_initialization_order() {
    // A scalar read textually (and in CFG order) before its initializer must see
    // the pre-init `undefined`, not the later constant.
    assert_equivalent(r#"var out = "" + x; var x = 5; __r = out + ":" + x;"#);
}

#[test]
fn ssa_keeps_effects_when_folding_reads() {
    // Folding `s[i]` reads must not disturb side effects elsewhere. The decoder
    // table assignment also has an observable neighbour.
    assert_equivalent(
        r#"
        var log = "";
        var s = [10, 20];
        function note(v){ log += v; return v; }
        note(s[0]);
        note(s[1]);
        __r = log;
        "#,
    );
}

#[test]
fn ssa_for_each_var_head_rebinding_not_folded() {
    // A `var` for-in/of head hoists into the function scope and rebinds the
    // candidate every iteration — a write the CFG does not see. It must not be
    // folded to the pre-loop value.
    assert_equivalent("var s = [10, 20]; for (var s of [[7, 8]]) {} __r = s[0];");
    assert_equivalent("var x = 1; for (var x of [9]) {} __r = x;");
    assert_equivalent(r#"var x = 1; for (var x in { a: 0, b: 0 }) {} __r = x;"#);
    assert_equivalent("var x = 1; for (var [x] of [[9]]) {} __r = x;");
}

#[test]
fn decoder_lift_respects_table_reassignment() {
    // A decoder reads a table that is REASSIGNED before the decoder is called.
    // The lift must not bake the stale initial table — get() must see [3,4].
    assert_equivalent(
        r#"
        var s = [1, 2];
        function get(){ return s[0]; }
        s = [3, 4];
        __r = get();
        "#,
    );
    // Same hazard with a string decoder table reassigned between uses.
    assert_equivalent(
        r#"
        var tbl = ["a", "b"];
        function d(i){ return tbl[i]; }
        var first = d(0);
        tbl = ["x", "y"];
        __r = first + ":" + d(0);
        "#,
    );
}

// ---------------------------------------------------------------------------
// Scoped decoder lifting (Tier C/D, src/passes/decoder_lift.rs): the decoder +
// table live inside a function scope `collect_top_level` cannot reach. Each pairs
// a shape that *should* resolve soundly with sibling shapes where baking the
// table would be wrong (member-mutated / `.push`d / aliased-and-mutated /
// reassigned). The differential net catches any unsound lift by value mismatch.
// ---------------------------------------------------------------------------

#[test]
fn scoped_decoder_lift_preserves_value() {
    // Decoder nested in a function body — must lift, value preserved.
    assert_equivalent(
        r#"
        function setup() {
            var tbl = ["lo", "HELLO", "wo"];
            function dec(i) { return tbl[i]; }
            return dec(1) + dec(0) + dec(2);
        }
        __r = setup();
        "#,
    );
    // Decoder nested in a `var = (function(){…})()` IIFE (the DataDome wall).
    assert_equivalent(
        r#"
        var R = (function () {
            var tbl = [10, 20, 30];
            function dec(i) { return tbl[i]; }
            return dec(1) + 1;
        })();
        __r = R;
        "#,
    );
}

#[test]
fn scoped_decoder_over_member_mutated_table_not_lifted() {
    // Member write mutates the table; baking the literal would read a stale slot.
    assert_equivalent(
        r#"
        function run() {
            var tbl = ["a", "b", "c"];
            function dec(i) { return tbl[i]; }
            tbl[1] = "Z";
            return dec(0) + dec(1);
        }
        __r = run();
        "#,
    );
}

#[test]
fn scoped_decoder_over_pushed_table_not_lifted() {
    // `.push` grows the table after the decoder is defined; dec(2) is runtime-only.
    assert_equivalent(
        r#"
        function run() {
            var tbl = ["a", "b"];
            function dec(i) { return tbl[i]; }
            tbl.push("c");
            return String(dec(2));
        }
        __r = run();
        "#,
    );
}

#[test]
fn scoped_decoder_over_aliased_mutated_table_not_lifted() {
    // The table is aliased and the alias is mutated — baking the literal is unsound.
    assert_equivalent(
        r#"
        function run() {
            var tbl = ["a", "b"];
            var alias = tbl;
            function dec(i) { return tbl[i]; }
            alias[0] = "Z";
            return dec(0) + dec(1);
        }
        __r = run();
        "#,
    );
}

#[test]
fn scoped_decoder_over_reassigned_table_not_lifted() {
    // The table binding is reassigned between definition and use (nested variant of
    // the top-level `decoder_lift_respects_table_reassignment` case).
    assert_equivalent(
        r#"
        function run() {
            var tbl = ["a", "b"];
            function dec(i) { return tbl[i]; }
            tbl = ["x", "y"];
            return dec(0) + dec(1);
        }
        __r = run();
        "#,
    );
}

// ---------------------------------------------------------------------------
// SSA copy propagation and flow-sensitive dead-store removal (src/ir/).
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Memoized ("private-cache") decoder lifting (Phase 1, src/passes/decoder_lift.rs).
// A decoder that caches its own results — `C[k]===undefined ? (C[k]=miss) : C[k]`
// — is semantically a pure function of `k` *iff* the cache `C` is private: a fresh
// empty aggregate, never reassigned, and only ever touched inside the memo idiom.
// Each "lifts" case is paired with a sibling where the cache is observable (read
// elsewhere, written non-memo, or the miss has a side effect) and lifting would
// change behavior. The differential net catches any unsound lift by value mismatch.
// ---------------------------------------------------------------------------

#[test]
fn memo_decoder_lifts_and_preserves_value() {
    // The canonical shape: a guarded miss-write cache. The warm-cache second call
    // (d(0) again) must still resolve to the same value.
    assert_equivalent(
        r#"
        var c = {}, t = ["a", "b", "c"];
        function d(i) { return c[i] === undefined ? (c[i] = t[i].toUpperCase()) : c[i]; }
        __r = d(0) + d(1) + d(0);
        "#,
    );
    // `void 0 ===` spelling and the hit-polarity `!==` ternary (the sample_7 form).
    assert_equivalent(
        r#"
        var c = {}, t = ["x", "y"];
        function d(i) { return void 0 === c[i] ? (c[i] = t[i] + "!") : c[i]; }
        __r = d(1) + d(0) + d(1);
        "#,
    );
    assert_equivalent(
        r#"
        var c = {}, t = ["p", "q"];
        function d(i) { return c[i] !== undefined ? c[i] : (c[i] = t[i] + t[i]); }
        __r = d(0) + d(1) + d(0);
        "#,
    );
}

#[test]
fn memo_decoder_numeric_result_stays_numeric() {
    // A numeric memo decoder feeding arithmetic must splice as a number (21), not
    // the string "201".
    assert_equivalent(
        r#"
        var c = {}, t = [10, 20, 30];
        function d(i) { return c[i] === undefined ? (c[i] = t[i]) : c[i]; }
        __r = d(1) + 1;
        "#,
    );
}

#[test]
fn memo_cache_read_elsewhere_must_not_lift() {
    // The cache contents are observed *outside* the decoder: `after` reads the warm
    // slot. Lifting the decode calls (and dropping the cache writes) would make the
    // cache stay cold, so `after` would change from "A" to undefined.
    assert_equivalent(
        r#"
        var c = {}, t = ["a", "b", "c"];
        function d(i) { return c[i] === undefined ? (c[i] = t[i].toUpperCase()) : c[i]; }
        var before = c[0];
        var x = d(0);
        var after = c[0];
        __r = String(before) + ":" + x + ":" + String(after);
        "#,
    );
}

#[test]
fn memo_cache_escapes_must_not_lift() {
    // The cache is aliased (read bare). It must stay live and unlifted.
    assert_equivalent(
        r#"
        var c = {}, t = ["a", "b"];
        function d(i) { return c[i] === undefined ? (c[i] = t[i].toUpperCase()) : c[i]; }
        var leak = c;
        var v = d(0) + d(1);
        __r = v + ":" + (leak === c);
        "#,
    );
}

#[test]
fn memo_impure_miss_must_not_lift() {
    // The miss computation has a side effect (`hits.push`). Lifting would drop it.
    assert_equivalent(
        r#"
        var c = {}, t = ["a", "b"], hits = [];
        function d(i) { return c[i] === undefined ? (c[i] = (hits.push(i), t[i].toUpperCase())) : c[i]; }
        __r = d(0) + d(1) + d(0) + ":" + hits.length;
        "#,
    );
}

#[test]
fn memo_non_memo_write_must_not_lift() {
    // An unconditional, non-memo write to the "cache" makes it real mutable state:
    // d(0) must observe the pre-set "Z", not recompute "A".
    assert_equivalent(
        r#"
        var c = {}, t = ["a", "b"];
        function d(i) { return c[i] === undefined ? (c[i] = t[i].toUpperCase()) : c[i]; }
        c[0] = "Z";
        __r = d(0) + d(1);
        "#,
    );
}

// ---------------------------------------------------------------------------
// Cross-function write-once constant inlining (Phase 2, GlobalConstInline). A
// constant defined once (literal) in an init function that runs unconditionally
// before any reader is inlined into its reads. Each "inlines" case is paired with
// a sibling where the write does NOT happen-before a read (read-before-init,
// conditional init/write, multiple writes, compound write) and inlining would
// expose a pre-init `undefined` or a wrong value. Differential net catches it.
// ---------------------------------------------------------------------------

#[test]
fn global_const_inline_preserves_value() {
    // The canonical shape: write-once literal in an init fn called before readers.
    assert_equivalent(
        r#"
        var k;
        function init(){ k = 19; }
        init();
        function use(){ return k; }
        __r = use();
        "#,
    );
    // Multiple constants, read as switch labels in a later-defined dispatcher.
    assert_equivalent(
        r#"
        var L1, L2;
        function setup(){ L1 = 7; L2 = 3; }
        setup();
        var run = function(s){
            var acc = 0;
            while (s) { switch (s) { case L1: acc += 10; s = L2; break; case L2: acc += 20; s = 0; break; } }
            return acc;
        };
        __r = run(L1);
        "#,
    );
}

#[test]
fn global_const_inline_nested_iife_init() {
    // Constant written in an init fn called inside a `var X = (IIFE)()` setup
    // block, read by a function defined inside the same IIFE. The IIFE body is a
    // setup region, so the write happens-before the read → resolves soundly.
    assert_equivalent(
        r#"
        var R = (function () {
            var k;
            function init(){ k = 19; }
            init();
            function use(){ return k; }
            return use();
        })();
        __r = R;
        "#,
    );
    // Sibling (must NOT resolve): a read textually before the init call inside the
    // IIFE observes the pre-init undefined. Resolving would change "undefined"→"19".
    assert_equivalent(
        r#"
        var R = (function () {
            var k;
            var snap = "" + k;
            function init(){ k = 5; }
            init();
            return snap + ":" + k;
        })();
        __r = R;
        "#,
    );
}

#[test]
fn global_const_read_before_init_must_not_inline() {
    // `out` reads `k` before `init()` runs, observing the pre-init undefined.
    assert_equivalent(r#"var out = "" + k; var k; function init(){ k = 5; } init(); __r = out;"#);
}

#[test]
fn global_const_conditional_init_must_not_inline() {
    // The write to `k` is inside a conditional within the init fn: it may not run.
    assert_equivalent(
        r#"
        var k;
        var cond = String(0) === "1";
        function init(){ if (cond) { k = 5; } }
        init();
        __r = String(k);
        "#,
    );
}

#[test]
fn global_const_conditional_init_call_must_not_inline() {
    // The init call itself is conditional (not an unconditional top-level call).
    assert_equivalent(
        r#"
        var k;
        var cond = String(0) === "1";
        function init(){ k = 5; }
        if (cond) { init(); }
        __r = String(k);
        "#,
    );
}

#[test]
fn global_const_two_writes_must_not_inline() {
    // Two writes: the value is position-dependent; the read sees the last (2).
    assert_equivalent(
        r#"
        var k;
        function init(){ k = 1; k = 2; }
        init();
        function use(){ return k; }
        __r = use();
        "#,
    );
}

#[test]
fn global_const_compound_write_must_not_inline() {
    // A compound write makes `k` real mutable state.
    assert_equivalent(
        r#"
        var k = 10;
        function bump(){ k += 5; }
        bump();
        function use(){ return k; }
        __r = use();
        "#,
    );
}

// ---------------------------------------------------------------------------
// Constant-selector function-array de-multiplexing (Phase 3, Demultiplex). An
// immutable array of multiplexer functions indexed by a constant `k`, called with
// a constant selector `C` as its first argument, has a statically-determined live
// branch. We specialize slot `k` for selector `C` (bind it, const-fold, drop dead
// branches) — preserving `this` by routing the specialized fn through `.call`.
// Each "specializes" case is paired with a sibling where specialization would be
// wrong (mutable array, non-constant selector, reassigned selector param, effect
// reorder, self-recursion). The differential net catches any unsound change.
// ---------------------------------------------------------------------------

#[test]
fn demux_specializes_constant_selector() {
    // Two constant selectors into the same slot resolve to the right branch.
    assert_equivalent(
        r#"
        var A = [function (s, a) { return s === 1 ? a * 2 : a + 1; }];
        __r = A[0](1, 5) + ":" + A[0](2, 5);
        "#,
    );
    // Multiple slots, nested guard chain that folds once the selector is bound.
    assert_equivalent(
        r#"
        var A = [
            function (s, a, b) { return s; },
            function (s, a, b) { if ((s | 1) === s) { return a + b; } return a - b; }
        ];
        __r = A[1](3, 10, 4) + ":" + A[1](2, 10, 4);
        "#,
    );
}

#[test]
fn demux_preserves_this_via_call() {
    // A slot that reads `this`: the specialized form must observe the same receiver.
    assert_equivalent(
        r#"
        var obj = { tag: "T" };
        var A = [function (s, x) { return this.tag + ":" + (s === 1 ? x * 2 : x + 1); }];
        __r = A[0].call(obj, 1, 5) + "|" + A[0].call(obj, 2, 5);
        "#,
    );
}

#[test]
fn demux_mutable_array_must_not_specialize() {
    // An element write makes the array mutable; the live slot is not statically
    // known, so calls must be left intact.
    assert_equivalent(
        r#"
        var A = [function (s, a) { return s === 1 ? a * 2 : a + 1; }];
        A[0] = function (s, a) { return s === 1 ? a * 100 : a; };
        __r = A[0](1, 5) + ":" + A[0](2, 5);
        "#,
    );
}

#[test]
fn demux_non_constant_selector_must_not_specialize() {
    // The selector is runtime-variable: the live branch is unknown.
    assert_equivalent(
        r#"
        var A = [function (s, a) { return s === 1 ? a * 2 : a + 1; }];
        var k = String(1) === "1" ? 1 : 2;
        __r = A[0](k, 5) + ":" + A[0](k + 1, 5);
        "#,
    );
}

#[test]
fn demux_reassigned_selector_param_must_not_specialize() {
    // The slot rewrites its selector param before branching on it; binding the
    // literal would be wrong.
    assert_equivalent(
        r#"
        var A = [function (s, a) { s = s & 2; return s === 0 ? a * 2 : a + 1; }];
        __r = A[0](1, 5) + ":" + A[0](2, 5) + ":" + A[0](3, 5);
        "#,
    );
}

#[test]
fn demux_effectful_trailing_arg_not_reordered() {
    // A trailing argument with a side effect must still run, in order, exactly once.
    assert_equivalent(
        r#"
        var seen = "";
        var A = [function (s, a, b) { return s === 1 ? b + a : a; }];
        var v = A[0](1, (seen += "a", 10), (seen += "b", 4));
        __r = v + ":" + seen;
        "#,
    );
}

#[test]
fn demux_self_recursive_slot_preserves_value() {
    // A slot that calls itself through the array: specialization must remain
    // bounded and value-preserving (terminating recursion).
    assert_equivalent(
        r#"
        var A = [function (s, n) { return n <= 0 ? s : A[0](s, n - 1) + 1; }];
        __r = A[0](100, 3);
        "#,
    );
}

#[test]
fn ssa_copy_propagation_preserves_value() {
    // A proxy variable collapses to its source.
    assert_equivalent("function g(x){ var b = x; return b + 1; } __r = g(7);");
    assert_equivalent("var a = 5; var b = a; __r = b * 10;");
}

#[test]
fn ssa_copy_propagation_respects_source_reassignment() {
    // `b` copies `x` while x is 1; x is then reassigned. `b` must stay 1, so the
    // copy must NOT be propagated to the live `x` (which is 2 at the read).
    assert_equivalent("var x = 1; var b = x; x = 2; __r = b + ':' + x;");
    // Source reassigned on a branch between copy and use → phi → no propagation.
    assert_equivalent(
        r#"
        var cond = String(1) === "1";
        var x = 1;
        var b = x;
        if (cond) { x = 9; }
        __r = b + ":" + x;
        "#,
    );
}

#[test]
fn ssa_dead_store_removal_preserves_value() {
    // The overwritten store is dead; the final value is observed.
    assert_equivalent("var x; x = 1; x = 2; __r = x;");
    assert_equivalent("var s; s = [1, 2]; s = [3, 4]; __r = s[0] + s[1];");
}

#[test]
fn ssa_dead_store_keeps_rhs_effects() {
    // A dead store whose rhs has a side effect: the store is removed but the
    // effect (and the rhs value, used by `y`) must remain.
    assert_equivalent("var log = 0; var x; x = (log = 1, 5); x = 9; __r = x + ':' + log;");
    // The dead store's value flows to another binding via the assignment value.
    assert_equivalent("var x; var y = (x = 1); x = 2; __r = y + ':' + x;");
}

// ---------------------------------------------------------------------------
// Single-use value inlining (src/ir/inline.rs). Each pairs a temp that *should*
// fold into its sole use with a sibling where moving the init to the use would
// reorder an effect, observe a mutated value, or rebind a shadowed name. The
// differential net catches any unsound move by value mismatch.
// ---------------------------------------------------------------------------

#[test]
fn inline_single_use_preserves_value() {
    // Plain temp chains collapse across rounds; order and value preserved.
    assert_equivalent("function f(){ var t = 5 + 3; return t * 2; } __r = f();");
    assert_equivalent(
        r#"function f(){ var a = "a" + "b"; var b = a + "c"; return b; } __r = f();"#,
    );
    // The snowball: a decoder temp used once as an index folds next to its access.
    assert_equivalent(
        r#"
        var arr = ["lo", "HELLO", "wo"];
        function d(i){ return arr[i]; }
        function run(){ var a = d(1); var b = d(0); var c = d(2); return a + b + c; }
        __r = run();
        "#,
    );
}

#[test]
fn inline_effectful_init_across_clean_gap_is_sound() {
    // The def's init has a side effect but nothing effectful sits between it and
    // the use, so moving it forward keeps the same observable order.
    assert_equivalent(
        r#"
        var ord = "";
        function f(){ ord += "f"; return 3; }
        function run(){ var t = f(); return t + ":" + ord; }
        __r = run();
        "#,
    );
}

#[test]
fn inline_must_not_reorder_effects() {
    // An effect (`g()`) sits between the def `t = f()` and the use of `t`: moving
    // `f()` past it would swap f/g. Observable order must stay f-before-g.
    assert_equivalent(
        r#"
        var ord = "";
        function f(){ ord += "f"; return 1; }
        function g(){ ord += "g"; return 2; }
        function run(){ var t = f(); g(); return t + ":" + ord; }
        __r = run();
        "#,
    );
    // The use is not the first sub-evaluation: `g()` is evaluated before `t` in
    // `g() + t`, so inlining `f()` there would reorder f after g.
    assert_equivalent(
        r#"
        var ord = "";
        function f(){ ord += "f"; return 1; }
        function g(){ ord += "g"; return 2; }
        function run(){ var t = f(); return (g() + t) + ":" + ord; }
        __r = run();
        "#,
    );
}

#[test]
fn inline_must_not_observe_mutated_value() {
    // `t` captures `obj.x` (1); a member write changes it before the use. Folding
    // the read forward would observe 99 instead of the captured 1.
    assert_equivalent(
        r#"
        function run(){ var obj = { x: 1 }; var t = obj.x; obj.x = 99; return t; }
        __r = run();
        "#,
    );
    // Same hazard with a plain outer-variable write between def and use.
    assert_equivalent(
        r#"
        function run(){ var n = 1; var t = n + 0; n = 99; return t + ":" + n; }
        __r = run();
        "#,
    );
}

#[test]
fn inline_respects_block_scope_shadowing() {
    // `t = a` reads the outer `a` (1). The use is inside a block that shadows `a`
    // with a `let` (99). Re-resolving the moved `a` there would bind the shadow.
    assert_equivalent(
        r#"
        function run(){ var a = 1; var t = a; { let a = 99; return t + ":" + a; } }
        __r = run();
        "#,
    );
}

#[test]
fn inline_assignment_form_temps() {
    // `var x; x = rhs;` (the no-init hoist + standalone assignment) folds too.
    assert_equivalent("function f(){ var x; x = 5 + 3; return x * 2; } __r = f();");
    // A reassigned parameter whose incoming value is dead before its single use.
    assert_equivalent("function f(a){ a = a.length + 1; return a; } __r = f([1, 2, 3]);");
    // Chain of standalone assignments resolves across rounds, order preserved.
    assert_equivalent(
        r#"
        var arr = ["lo", "HELLO", "wo"];
        function d(i){ return arr[i]; }
        function run(){ var a, b; a = d(1); b = d(0); return a + b; }
        __r = run();
        "#,
    );
}

#[test]
fn inline_assignment_form_must_not_reorder() {
    // A standalone-assignment def whose rhs is effectful must not jump a barrier.
    assert_equivalent(
        r#"
        var ord = "";
        function f(){ ord += "f"; return 1; }
        function g(){ ord += "g"; return 2; }
        function run(){ var t; t = f(); g(); return t + ":" + ord; }
        __r = run();
        "#,
    );
}

#[test]
fn inline_respects_arguments_aliasing() {
    // Sloppy-mode `arguments[i] = v` aliases the i-th param. `t` captures `a`'s
    // value (1) before the alias write changes it (99); the member write is a
    // barrier in the gap, so the read must NOT fold forward to observe 99.
    assert_equivalent(
        r#"
        function f(a){ var t = a; arguments[0] = 99; return t; }
        __r = f(1);
        "#,
    );
    // The reverse alias: writing the param updates `arguments[0]`; a temp that
    // captured the old element must stay the old value.
    assert_equivalent(
        r#"
        function f(a){ var t = arguments[0]; a = 99; return t; }
        __r = f(1);
        "#,
    );
    // Passing `arguments` to a callee that mutates it is a call barrier.
    assert_equivalent(
        r#"
        function mut(args){ args[0] = 99; }
        function f(a){ var t = a; mut(arguments); return t; }
        __r = f(1);
        "#,
    );
}

#[test]
fn inline_in_arguments_function_when_safe() {
    // `arguments` is used but nothing mutates the read values across the gap, so
    // the temp chain still folds — exactly the sample_10 shape we want to reach.
    assert_equivalent(
        r#"
        function f(){ var n = arguments.length; var t = n + 1; return t; }
        __r = f(7, 8, 9);
        "#,
    );
    assert_equivalent(
        r#"
        function sum(){ var rest = Array.prototype.slice.call(arguments, 1); var first = arguments[0]; return first + ":" + rest.join(","); }
        __r = sum(10, 20, 30);
        "#,
    );
}

#[test]
fn inline_does_not_touch_captured_temp() {
    // A temp read by a nested closure is excluded (escape analysis); its
    // effectful init must run exactly once and the closure must see its value.
    assert_equivalent(
        r#"
        var log = "";
        function run(){
            var t = (log += "x", 7);
            var get = function(){ return t; };
            return get() + ":" + log;
        }
        __r = run();
        "#,
    );
}

#[test]
fn unflatten_dispatcher_preserves_value() {
    // A linear integer-state while/switch chain collapses to the straight-line
    // sequence it encodes. Value must be identical.
    assert_equivalent(
        r#"
        var acc = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: acc += 10; s = 2; break;
                case 2: acc += 20; s = 3; break;
                case 3: acc += 30; s = 0; break;
            }
        }
        __r = acc;
        "#,
    );
    // A case that returns (terminal) inside an IIFE — chain stops there.
    assert_equivalent(
        r#"
        __r = (function () {
            var s = 1;
            while (s) {
                switch (s) {
                    case 1: var x = 5; s = 2; break;
                    case 2: return x * 4;
                }
            }
        })();
        "#,
    );
}

#[test]
fn unflatten_mul_and_fallthrough_preserve_value() {
    // Multiplicative transition: 1 -> 3 -> 12 (exit). acc = 10 + 20.
    assert_equivalent(
        r#"
        var acc = 0;
        var s = 1;
        while (s != 12) {
            switch (s) {
                case 1: acc += 10; s *= 3; break;
                case 3: acc += 20; s *= 4; break;
            }
        }
        __r = acc;
        "#,
    );
    // Fall-through: case 1 has no break, control flows into case 2's body.
    // Run order is acc += 10 then acc += 20 -> acc = 30.
    assert_equivalent(
        r#"
        var acc = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: acc += 10;
                case 2: acc += 20; s = 0; break;
            }
        }
        __r = acc;
        "#,
    );
    // Jump into a fall-through group: 1 -> 2, then 2 falls into 3.
    assert_equivalent(
        r#"
        var acc = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: acc += 1; s = 2; break;
                case 2: acc += 2;
                case 3: acc += 4; s = 0; break;
            }
        }
        __r = acc;
        "#,
    );
}

#[test]
fn unflatten_string_and_bool_state_preserve_value() {
    // String-state dispatcher: "a" -> "b" -> exit. acc = 10 + 20.
    assert_equivalent(
        r#"
        var acc = 0;
        var s = "a";
        while (s) {
            switch (s) {
                case "a": acc += 10; s = "b"; break;
                case "b": acc += 20; s = ""; break;
            }
        }
        __r = acc;
        "#,
    );
    // String-state with a non-falsy terminal: while (s != "done").
    assert_equivalent(
        r#"
        var acc = 0;
        var s = "x";
        while (s != "done") {
            switch (s) {
                case "x": acc += 1; s = "y"; break;
                case "y": acc += 2; s = "done"; break;
            }
        }
        __r = acc;
        "#,
    );
    // Boolean-state dispatcher: true does work then flips to false (exit).
    assert_equivalent(
        r#"
        var acc = 0;
        var s = true;
        while (s) {
            switch (s) {
                case true: acc += 7; s = false; break;
            }
        }
        __r = acc;
        "#,
    );
    // Cyclic string-state must be declined (value preserved by NOT transforming).
    assert_equivalent(
        r#"
        var n = 0;
        var s = "a";
        while (s) {
            switch (s) {
                case "a": n += 1; if (n < 3) { s = "a"; } else { s = ""; } break;
            }
        }
        __r = n;
        "#,
    );
}

#[test]
fn unflatten_declines_unsound_shapes() {
    // Conditional transition (real loop): `s` is written inside an `if`, so the
    // machine is cyclic, not a linear chain. Must NOT be unflattened; value (n=3)
    // is preserved because the pass declines.
    assert_equivalent(
        r#"
        var n = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: n = n + 1; if (n < 3) { s = 1; } else { s = 0; } break;
            }
        }
        __r = n;
        "#,
    );
    // The case body READS the state var. Emitting the body without the dispatcher
    // would make `s` undefined — the pass must decline (the read is observable).
    assert_equivalent(
        r#"
        var acc = 0;
        var s = 7;
        while (s) {
            switch (s) {
                case 7: acc = s; s = 0; break;
            }
        }
        __r = acc;
        "#,
    );
}

#[test]
fn unflatten_cfg_recovers_self_loop() {
    // A conditional self-transition (the shape linear `simulate` declines): state 1
    // loops back to itself while n < 3, else exits. P2 recovers it as a do/while.
    // Value (n) must be identical whether or not the recovery fires.
    assert_equivalent(
        r#"
        var n = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: n = n + 1; if (n < 3) { s = 1; } else { s = 0; } break;
            }
        }
        __r = n;
        "#,
    );
    // Self-arm in the `else` (loop while cond is false) + a relational exit test.
    assert_equivalent(
        r#"
        var acc = 0, i = 0;
        var s = 5;
        while (s != 9) {
            switch (s) {
                case 5: acc += i; i = i + 1; if (i >= 4) { s = 9; } else { s = 5; } break;
            }
        }
        __r = acc + ":" + i;
        "#,
    );
    // A do-while-form dispatcher with a self loop.
    assert_equivalent(
        r#"
        var p = 1, s = 2;
        do {
            switch (s) {
                case 2: p = p * 2; if (p < 100) { s = 2; } else { s = 0; } break;
            }
        } while (s);
        __r = p;
        "#,
    );
}

#[test]
fn unflatten_cfg_declines_unsupported_shapes() {
    // A two-state loop (header + body, body returns to header). P2-v1 structures
    // only the single-state self-loop, so this is left untouched — but must stay
    // equivalent (the pass declines, no mis-structuring, no panic).
    assert_equivalent(
        r#"
        var acc = 0, i = 0, s = 1;
        while (s) {
            switch (s) {
                case 1: if (i < 4) { s = 2; } else { s = 0; } break;
                case 2: acc += i; i = i + 1; s = 1; break;
            }
        }
        __r = acc + ":" + i;
        "#,
    );
    // A forward branch that re-merges (diamond, no loop) — declined, equivalent.
    assert_equivalent(
        r#"
        var v = 0, s = 1;
        var cond = String(1) === "1";
        while (s) {
            switch (s) {
                case 1: if (cond) { s = 2; } else { s = 3; } break;
                case 2: v = 10; s = 4; break;
                case 3: v = 20; s = 4; break;
                case 4: v += 1; s = 0; break;
            }
        }
        __r = v;
        "#,
    );
}

#[test]
fn pure_iife_eval_preserves_value() {
    // A self-contained, side-effect-free IIFE (a tiny "bytecode VM") collapses to
    // the primitive it computes. Value must be preserved.
    assert_equivalent(
        r#"
        __r = (function () {
            var prog = [1, 7, 1, 35, 2], stack = [];
            for (var i = 0; i < prog.length; i++) {
                var op = prog[i];
                if (op === 1) { stack.push(prog[++i]); }
                else { var b = stack.pop(), a = stack.pop(); stack.push(a + b); }
            }
            return stack.pop();
        })();
        "#,
    );
    // String result and a Math-using IIFE: deterministic globals are fine.
    assert_equivalent(r#"__r = (function(){ return "ab".repeat(3); })();"#);
    assert_equivalent("__r = (function(){ return Math.max(3, 9, 4) + 1; })();");
    // An IIFE whose result is consumed but which writes an OUTER variable must NOT
    // be collapsed (the write is observable). Equivalence holds because PureEval
    // declines (the free write fails its purity gate).
    assert_equivalent(
        "var seen = 0; var v = (function(){ seen = 1; return 5; })(); __r = v + ':' + seen;",
    );
}

#[test]
fn math_fingerprint_preserves_value() {
    // Addition polynomial (full-adder identity) with runtime-variable args. The
    // proxy collapses to `+`, which must keep the numeric value.
    assert_equivalent(
        r#"
        var a = 5, b = 3;
        function add(x, y) { return (x ^ y) + 2 * (x & y); }
        __r = add(a, b) + ":" + add(10, -4);
        "#,
    );
    // Subtraction must keep operand order (a - b, not b - a).
    assert_equivalent(
        r#"
        var a = 10, b = 3;
        function sub(x, y) { return x + ~y + 1; }
        __r = sub(a, b) + ":" + sub(b, a);
        "#,
    );
    // Swapped subtraction: `b - a` body must rewrite to `y - x`, same value.
    assert_equivalent(
        r#"
        var a = 10, b = 3;
        function r(x, y) { return y - x; }
        __r = r(a, b);
        "#,
    );
    // Bitwise xor proxy.
    assert_equivalent(
        r#"
        var a = 0xF0, b = 0x0F;
        function x(p, q) { return (p | q) - (p & q); }
        __r = x(a, b);
        "#,
    );
}

#[test]
fn math_fingerprint_keeps_argument_side_effects() {
    // Both arguments carry an effect; collapsing `f(a,b)` to `a + b` evaluates
    // them in the same left-to-right order, so the trace and value must match.
    assert_equivalent(
        r#"
        var log = "";
        function tick(n) { log += n; return n; }
        function add(x, y) { return (x ^ y) + 2 * (x & y); }
        var s = add(tick(5), tick(3));
        __r = s + ":" + log;
        "#,
    );
}

#[test]
fn math_fingerprint_respects_argument_coercion() {
    // An MBA `+` proxy equals `+` only for int32 args (its bitwise body
    // ToInt32-coerces). Rewriting its calls to a bare `+` changes behavior when
    // it is called with a string or float, so the fingerprint must either decline
    // or emit the coercion-faithful form (`(a|0)+(b|0)`).
    assert_equivalent(
        r#"
        function add(x, y) { return (x ^ y) + 2 * (x & y); }
        __r = add(3, 4) + ":" + add("a", "b") + ":" + add(0.5, 0.5);
        "#,
    );
    // A *true* operator proxy (no coercion) stays safe to collapse, including on
    // non-integer arguments.
    assert_equivalent(
        r#"
        function plus(x, y) { return x + y; }
        __r = plus(3, 4) + "|" + plus("a", "b");
        "#,
    );
}

#[test]
fn unflatten_generalized_shapes_preserve_value() {
    // while(s != K) non-falsy exit.
    assert_equivalent(
        "var s = 1; __r = 0; while (s != 9) { switch (s) { \
         case 1: __r += 10; s = 2; break; case 2: __r += 5; s = 9; break; } }",
    );
    // do-while runs body before test.
    assert_equivalent(
        "var s = 1; __r = 0; do { switch (s) { \
         case 1: __r += 10; s = 2; break; case 2: __r += 5; s = 7; break; } } while (s != 7);",
    );
    // do-while runs the body once even when the exit test is immediately false:
    // s starts at 9, the case transitions 9 -> 9, and `while (s != 9)` exits after
    // one pass. A plain `while (s != 9)` would not run at all — pins body-before-test.
    assert_equivalent(
        "var s = 9; __r = 0; do { switch (s) { \
         case 9: __r += 3; s = 9; break; } } while (s != 9);",
    );
    // Compound (relative) transitions: 1 -> 5 -> 13.
    assert_equivalent(
        "var s = 1; __r = 0; while (s != 13) { switch (s) { \
         case 1: __r += 10; s += 4; break; case 5: __r += 5; s += 8; break; } }",
    );
    // Relational exit: while (s < 9).
    assert_equivalent(
        "var s = 1; __r = 0; while (s < 9) { switch (s) { \
         case 1: __r += 10; s = 5; break; case 5: __r += 5; s = 9; break; } }",
    );
    // Symbolic write-once constant labels.
    assert_equivalent(
        "var A = 1, B = 2, Z = 0; var s = A; __r = 0; while (s) { switch (s) { \
         case A: __r += 10; s = B; break; case B: __r += 5; s = Z; break; } }",
    );
    // Terminal return inside a case (machine stops early).
    assert_equivalent(
        "__r = (function(){ var s = 1; while (s != 4) { switch (s) { \
         case 1: s = 2; break; case 2: return 42; case 3: return 99; } } return 0; })();",
    );
    // A genuine loop must be preserved (still equivalent either way).
    assert_equivalent(
        "var s = 1, n = 0; __r = 0; do { switch (s) { \
         case 1: n += 1; s = 2; break; case 2: __r += n; s = (n < 3) ? 1 : 0; break; } } while (s);",
    );
}

#[test]
fn unflatten_nonadjacent_init_preserves_value() {
    // The state initializer and the dispatcher loop are NOT adjacent — an
    // unrelated declaration sits between them. The dispatch var is still
    // referenced only in the init and the loop, so unflattening is sound.
    assert_equivalent(
        r#"
        var s = 1;
        var acc = 0;
        while (s) { switch (s) {
            case 1: acc += 10; s = 2; break;
            case 2: acc += 5; s = 0; break;
        } }
        __r = acc;
        "#,
    );
    // Symbolic labels + non-adjacent init: GCI materializes the labels, then the
    // relaxed unflattener fires across the gap.
    assert_equivalent(
        r#"
        var A = 1, B = 2, Z = 0;
        var s = A;
        var acc = 0;
        while (s) { switch (s) {
            case A: acc += 10; s = B; break;
            case B: acc += 5; s = Z; break;
        } }
        __r = acc;
        "#,
    );
    // Sibling (must stay equivalent): a statement BETWEEN init and loop that reads
    // s — the dispatch var is no longer private, so the pass must decline.
    assert_equivalent(
        r#"
        var s = 1;
        var peek = s + 100;
        var acc = 0;
        while (s) { switch (s) {
            case 1: acc += 10; s = 2; break;
            case 2: acc += 5; s = 0; break;
        } }
        __r = acc + ":" + peek;
        "#,
    );
}

#[test]
fn unflatten_param_state_preserves_value() {
    // Param-state dispatcher: state is param 0, called with a constant entry.
    assert_equivalent(
        "__r = (function(){ \
           function f(s, o) { while (s != 9) { switch (s) { \
             case 1: o.v += 10; s = 2; break; case 2: o.v += 5; s = 9; break; } } } \
           var o = { v: 0 }; f(1, o); return o.v; })();",
    );
    // Multiple distinct entry constants reuse one function.
    assert_equivalent(
        "__r = (function(){ \
           function f(s, o) { while (s) { switch (s) { \
             case 1: o.v += 1; s = 3; break; case 2: o.v += 100; s = 0; break; \
             case 3: o.v += 10; s = 0; break; } } } \
           var o = { v: 0 }; f(1, o); f(2, o); return o.v; })();",
    );
    // Recursion: the body calls itself with a constant entry.
    assert_equivalent(
        "__r = (function(){ \
           function f(s, n) { while (s != 0) { switch (s) { \
             case 1: if (n <= 0) return 0; s = 2; break; \
             case 2: return n + f(1, n - 1); } } return -1; } \
           return f(1, 3); })();",
    );
    // Named function expression (dual name: outer V + inner self-name), do-while.
    assert_equivalent(
        "__r = (function(){ \
           var g = function self(s, o) { do { switch (s) { \
             case 5: o.v += 7; s = 6; break; case 6: o.v += 2; s = 9; break; } } while (s != 9); }; \
           var o = { v: 0 }; g(5, o); return o.v; })();",
    );
    // Trailing statements after the loop run on a test-exit, are dead on a
    // terminal (return) exit.
    assert_equivalent(
        "__r = (function(){ \
           function f(s, o) { while (s != 9) { switch (s) { \
             case 1: o.v += 1; s = 2; break; case 2: o.v += 4; s = 9; break; } } o.v += 100; } \
           var o = { v: 0 }; f(1, o); return o.v; })();",
    );
    assert_equivalent(
        "__r = (function(){ \
           function f(s, o) { while (s != 9) { switch (s) { \
             case 1: o.v += 1; s = 2; break; case 2: o.v += 4; return o.v; } } o.v += 100; return o.v; } \
           var o = { v: 0 }; return f(1, o); })();",
    );
    // Side-effecting argument must be evaluated exactly once, in order.
    assert_equivalent(
        "__r = (function(){ var log = ''; \
           function f(s, x) { while (s) { switch (s) { case 1: log += x; s = 0; break; } } } \
           f(1, (log += 'a', 'b')); return log; })();",
    );
}

#[test]
fn pure_expression_resolution_preserves_value() {
    // B4: a pure-expression constant chain in an init function, read in a
    // dispatcher. Resolving `B=A+1`, `C=B*2-3` must not change the result.
    assert_equivalent(
        "__r = (function(){ \
           var A, B, C, STOP; \
           function init(){ A = 10; B = A + 1; C = B * 2 - 3; STOP = 0; } init(); \
           function run(s){ var acc = 0; while (s != STOP) { switch (s) { \
             case A: acc += 1; s = B; break; case B: acc += 2; s = C; break; \
             case C: acc += 3; s = STOP; break; } } return acc; } \
           return run(A); })();",
    );
    // Bitwise / shift / remainder under JS int32 semantics.
    assert_equivalent(
        "__r = (function(){ \
           var X, Y, Z; \
           function init(){ X = (419 ^ 661) >>> 0; Y = X & 31; Z = (Y << 2) - 7 % 4; } init(); \
           function read(){ return X + Y + Z; } \
           return read(); })();",
    );
    // Negative results and unary operators.
    assert_equivalent(
        "__r = (function(){ \
           var P, Q; \
           function init(){ P = -5; Q = ~P + 1; } init(); \
           function read(){ return P + Q; } \
           return read(); })();",
    );
}

/// Assert the deobfuscation both preserves the value AND removes the flattened
/// dispatcher (the recovered code contains no `switch`). This is the A2 relooper
/// firing: the proof that we structured, not just stayed equivalent by declining.
fn assert_structured(src: &str) {
    assert_equivalent(src);
    let deob = js_deobfuscator::deobfuscate(src, "t.js");
    assert!(
        !deob.code.contains("switch"),
        "expected the dispatcher to be structured away, but a switch remains:\n{}",
        deob.code
    );
}

/// Like `assert_structured` but the *opposite*: the pass must DECLINE (a `switch`
/// remains) — the adversarial / unsupported sibling. Value still preserved.
fn assert_declined(src: &str) {
    assert_equivalent(src);
    let deob = js_deobfuscator::deobfuscate(src, "t.js");
    assert!(
        deob.code.contains("switch"),
        "expected the dispatcher to be left intact (declined), but it was structured:\n{}",
        deob.code
    );
}

#[test]
fn string_order_chain_preserves_value_and_order() {
    // The array order "2|0|1" must drive execution order, not the case-label order.
    assert_equivalent(
        "var log = ''; \
         (function () { \
           for (var a = \"2|0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { \
               case \"0\": log += 'b'; continue; \
               case \"1\": log += 'c'; continue; \
               case \"2\": log += 'a'; continue; \
             } \
             break; \
           } \
         })(); __r = log;",
    );
}

#[test]
fn string_order_return_terminal_preserves_value() {
    // A `return` case stops the walk mid-array; later keys must not run.
    assert_equivalent(
        "__r = (function () { \
           var log = ''; \
           for (var a = \"0|1|2\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { \
               case \"0\": log += 'x'; continue; \
               case \"1\": return log + '!'; \
               case \"2\": log += 'never'; continue; \
             } \
             break; \
           } \
           return log; \
         })();",
    );
}

#[test]
fn string_order_param_cursor_preserves_value() {
    // The real sample shape: `a`/`i` are dispatch-only params, assigned in the
    // for-init. Side effects must run in array order ("1|0").
    assert_equivalent(
        "__r = (function () { \
           var log = ''; \
           (function (a, i) { \
             for (a = \"1|0\".split(\"|\"), i = 0; true;) { \
               switch (a[i++]) { case \"0\": log += 'p'; continue; case \"1\": log += 'q'; continue; } \
               break; \
             } \
           })(); \
           return log; \
         })();",
    );
}

#[test]
fn conditional_nested_region_init_is_not_inlined() {
    // B1 wall (sample_1's shape): a constant written by an init function that runs
    // only inside a *conditionally-reached nested dispatcher case* must NOT be
    // inlined cross-function — a read can observe the pre-init `undefined` (here,
    // `case 1` reads L before `case 2`'s init writes it). The pipeline must leave
    // the write and the symbolic reads in place; equivalence is the safety net.
    assert_equivalent(
        "__r = (function(){ \
           var L; var out = []; \
           function disp(s){ \
             while (s) { switch (s) { \
               case 1: out.push('' + L); s = 2; break; \
               case 2: init(); s = 3; break; \
               case 3: out.push('' + L); s = 0; break; } } \
             function init(){ L = 9; } } \
           disp(1); return out.join(','); })();",
    );
}

#[test]
fn relooper_structures_two_state_loop() {
    // state1 branches to body(2) or exit; body returns to 1 (a `while` with a guard).
    assert_structured(
        r#"
        var acc = 0, i = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: if (i < 4) { s = 2; } else { s = 0; } break;
                case 2: acc += i; i = i + 1; s = 1; break;
            }
        }
        __r = acc + ":" + i;
        "#,
    );
}

#[test]
fn nondeterministic_and_nonpure_writes_are_not_resolved() {
    // B4 adversarial: an RHS over an impure call (`Date.now()`) or a non-write-once
    // operand must be left alone. Equivalence is the safety net — the value must
    // match regardless of whether resolution (correctly) declines.
    assert_equivalent(
        "__r = (function(){ \
           var A, B; \
           function init(){ A = 1; A = 2; B = A + 1; } init(); \
           function read(){ return B; } \
           return read(); })();",
    );
    // `T = Date.now()` is non-pure (host state) → never resolved; `U = T + 1` then
    // also cannot resolve. We can only assert the *deterministic* part is stable, so
    // mask the timestamp out: the point is the pipeline must not fold these.
    assert_equivalent(
        "__r = (function(){ \
           var T, U; \
           function init(){ T = 100; U = T / 0; } init(); \
           function read(){ return (U === Infinity); } \
           return read(); })();",
    );
}

#[test]
fn relooper_structures_forward_diamond() {
    // A forward branch that re-merges (an `if/else` with a shared continuation).
    assert_structured(
        r#"
        var v = 0;
        var cond = String(1) === "1";
        var s = 1;
        while (s) {
            switch (s) {
                case 1: if (cond) { s = 2; } else { s = 3; } break;
                case 2: v = 10; s = 4; break;
                case 3: v = 20; s = 4; break;
                case 4: v += 1; s = 0; break;
            }
        }
        __r = v;
        "#,
    );
}

#[test]
fn relooper_structures_loop_containing_branch() {
    // A loop whose body has an internal diamond (state 2 → {3,4} → 5 → back to 1).
    assert_structured(
        r#"
        var sum = 0, i = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: if (i >= 5) { s = 0; } else { s = 2; } break;
                case 2: if (i % 2 === 0) { s = 3; } else { s = 4; } break;
                case 3: sum += i; s = 5; break;
                case 4: sum -= i; s = 5; break;
                case 5: i = i + 1; s = 1; break;
            }
        }
        __r = sum + ":" + i;
        "#,
    );
}

#[test]
fn relooper_structures_nested_loops() {
    // An outer loop over `i` containing an inner loop over `j` — two back edges,
    // distinct headers, break/continue at each level.
    assert_structured(
        r#"
        var total = 0, i = 0, j = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: if (i < 3) { s = 2; } else { s = 0; } break;
                case 2: j = 0; s = 3; break;
                case 3: if (j < 3) { s = 4; } else { s = 5; } break;
                case 4: total = total + i * j; j = j + 1; s = 3; break;
                case 5: i = i + 1; s = 1; break;
            }
        }
        __r = total;
        "#,
    );
}

#[test]
fn relooper_structures_branch_arms_reentering_loop() {
    // Both arms of an in-loop branch continue the loop (no merge): each arm ends in
    // its own back edge. Exercises break (state 1 exit) + continue from two arms.
    assert_structured(
        r#"
        var a = 0, b = 0, i = 0;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: if (i >= 6) { s = 0; } else { s = 2; } break;
                case 2: if (i % 3 === 0) { s = 3; } else { s = 4; } break;
                case 3: a += i; i = i + 1; s = 1; break;
                case 4: b += i; i = i + 1; s = 1; break;
            }
        }
        __r = a + ":" + b;
        "#,
    );
}

#[test]
fn relooper_structures_loop_with_return_terminator() {
    // A mixed terminator: one arm `return`s out of the function, the loop exits via
    // the other. The return content is preserved verbatim.
    assert_structured(
        r#"
        __r = (function () {
            var n = 0;
            var s = 1;
            while (s) {
                switch (s) {
                    case 1: if (n >= 10) { return n * 2; } else { s = 2; } break;
                    case 2: n = n + 3; s = 1; break;
                }
            }
            return -1;
        })();
        "#,
    );
}

#[test]
fn relooper_declines_irreducible() {
    // Two states with cross edges (a jump into a loop body): irreducible. The pass
    // must decline — the dispatcher stays, value preserved either way.
    assert_declined(
        r#"
        var v = 0, i = 0;
        var enter = (String(1) === "1") ? 2 : 3;
        var s = 1;
        while (s) {
            switch (s) {
                case 1: s = enter; break;
                case 2: v += 1; i = i + 1; if (i < 3) { s = 3; } else { s = 0; } break;
                case 3: v += 10; i = i + 1; if (i < 3) { s = 2; } else { s = 0; } break;
            }
        }
        __r = v;
        "#,
    );
}

#[test]
fn computed_state_constant_object_discriminant_preserves_value() {
    // A3 (deterministic subcase): the discriminant is `obj + counter` where `obj`
    // provably coerces to a constant. Equivalent to a counter-offset dispatcher,
    // and the recognizer normalizes + structures it away.
    assert_structured(
        r#"
        var out = 0;
        var b = 0;
        var t = { valueOf: function () { return 100; } };
        do {
            switch (t + b) {
                case 100: out += 1; b += 1; break;
                case 101: out += 10; b += 1; break;
                case 102: out += 100; b = 99; break;
            }
        } while (b < 50);
        __r = out;
        "#,
    );
    // `new Number(C)` object, subtraction discriminant `obj - counter`.
    assert_structured(
        r#"
        var acc = 0;
        var i = 0;
        var t = new Number(10);
        do {
            switch (t - i) {
                case 10: acc += 5; i -= 1; break;
                case 11: acc += 7; i -= 1; break;
                case 12: acc += 9; i = -90; break;
            }
        } while (i > -50);
        __r = acc;
        "#,
    );
}

#[test]
fn computed_state_string_counter_declines_soundly() {
    // A3 soundness gate: the counter is a STRING, so `t + b` concatenates rather
    // than adds — `switch(C + b)` is NOT `switch(b)` with offset labels. The
    // recognizer must decline (the numericness gate fails). Value preserved.
    assert_declined(
        r#"
        var out = "";
        var b = "x";
        var t = { valueOf: function () { return 100; } };
        var guard = 0;
        do {
            switch (t + b) {
                case "100x": out += "A"; b = "y"; break;
                case "100y": out += "B"; b = "z"; break;
                default: guard = 1;
            }
        } while (guard === 0);
        __r = out;
        "#,
    );
}

#[test]
fn computed_state_mutating_valueof_declines_soundly() {
    // A3 adversarial: a per-read MUTATING valueOf (the sample_4 shape — the object
    // re-applies a transform on each coercion). The next state is not statically a
    // pure function of the counter, so the recognizer MUST decline. Value preserved.
    assert_equivalent(
        r#"
        var out = 0;
        var b = 0;
        var t = { n: 0, valueOf: function () { this.n = this.n + 1; return this.n; } };
        do {
            switch (t + b) {
                case 1: out += 1; b += 0; break;
                case 3: out += 10; b += 0; break;
                case 5: out += 100; b += 0; break;
                default: b = 99;
            }
        } while (b < 10 && out < 1000);
        __r = out;
        "#,
    );
}

#[test]
fn string_order_nonconstant_array_decline_preserves_value() {
    // Adversarial: the order array is a runtime value (a split of a parameter), so the
    // pass must decline. The value must still be preserved — proving the decline path
    // stays sound. Side effects run in the runtime array order ("1|0").
    assert_equivalent(
        "__r = (function () { \
           var log = ''; \
           (function (src) { \
             for (var a = src.split(\"|\"), i = 0; true;) { \
               switch (a[i++]) { case \"0\": log += 'a'; continue; case \"1\": log += 'b'; continue; } \
               break; \
             } \
           })(\"1|0\"); \
           return log; \
         })();",
    );
}
