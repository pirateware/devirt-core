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
