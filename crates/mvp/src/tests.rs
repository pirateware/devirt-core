use crate::DEMO;
use crate::emit::emit;
use crate::error::Error;
use crate::ir::disposition::Disposition;
use crate::ir::lower_ssa;
use crate::ir::lower_tree;
use crate::ir::lower_tree::{EmitConfig, lower_module_with_config};
use crate::ir::ssa::{
    self, BasicBlock, BlockId, Constant, FuncId, Function, FunctionBody, FunctionBuilder, InstKind,
    LoweredBody, Operand, PassThroughBody, Terminator, Value, ValueDef,
};
use crate::lower::{parse_and_lower, parse_and_lower_with_decider};
use oxc_ast::ast as js;

fn roundtrip(src: &str) -> String {
    let m = parse_and_lower(src).expect("parse_and_lower failed");
    let out = emit(&m);
    parse_and_lower(&out).expect("emitted JS failed to re-parse");
    out
}

/// Full pipeline roundtrip: parse → tree-IR → SSA → tree-IR → emit.
/// Exercises the SSA round-trip which is where the closure resugaring
/// and capture-resolution logic lives. Returns the final emitted source.
fn ssa_roundtrip(src: &str) -> String {
    let tm = parse_and_lower(src).expect("parse_and_lower failed");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    let tm2 = lower_tree::lower_module(&sm);
    let out = emit(&tm2);
    parse_and_lower(&out).expect("SSA-roundtrip emit must re-parse");
    out
}

#[test]
fn fib() {
    let out = roundtrip(DEMO);
    assert!(out.contains("function asd"));
}

#[test]
fn arithmetic() {
    let _ = roundtrip("const x = (1 + 2) * 3;");
}

#[test]
fn member_and_call() {
    let _ = roundtrip("console.log(\"hi\", 42);");
}

#[test]
fn while_loop() {
    let _ = roundtrip("let i = 0; while (i < 10) { i = i + 1; }");
}

#[test]
fn for_loop_lowers_in_function_body() {
    // Body uses `for` + `continue` + compound assignment + `++` —
    // exercises items 4-6 together. Walker accepts; SSA round-trip
    // must produce a `while`-shaped emit (we don't reconstruct
    // `for` from SSA).
    let src = "function f() { let s = 0; for (let i = 0; i < 3; i++) { if (i === 1) continue; s += i; } return s; }";
    let m = parse_and_lower(src).expect("for-loop body must lower");
    // The function must be Lowered (not PassThrough).
    use crate::ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "for-loop function body must lower, not PassThrough; got {:?}",
        m.items[0]
    );
    let out = ssa_roundtrip(src);
    assert!(out.contains("while"), "SSA round-trip emits while: {out}");
}

#[test]
fn ternary_lowers_in_function_body() {
    let src = "function pick(c) { return c ? 1 : 2; }";
    let m = parse_and_lower(src).expect("ternary body must lower");
    use crate::ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let _ = roundtrip(src);
}

#[test]
fn update_lowers_in_function_body() {
    let src = "function inc(x) { return x++; }";
    let m = parse_and_lower(src).expect("update body must lower");
    use crate::ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let _ = roundtrip(src);
}

/// Top-level `class` / `for` and other constructs the lowering can't
/// represent are now preserved as verbatim source slices via
/// `Item::PassThroughStmt` — the same fallback strategy used for
/// un-lowerable function bodies. The "errors loudly on unsupported"
/// guarantee still applies inside Lowered function bodies (via
/// `disposition::decide`); top-level passthrough is the safer default
/// when the alternative is silent data loss (see
/// `top_level_iife_survives_ssa_roundtrip` for the bug this prevents).
#[test]
fn top_level_class_passthrough() {
    let src = "class Foo {}";
    let out = roundtrip(src);
    assert!(out.contains("class Foo"), "class must survive: {out}");
}

#[test]
fn top_level_for_passthrough() {
    let src = "for (let i = 0; i < 10; i++) {}";
    let out = roundtrip(src);
    assert!(out.contains("for"), "for must survive: {out}");
}

/// Function body containing a class — disposition routes the function
/// to PassThrough. The class itself is preserved verbatim in the
/// emitted output via the source-slice round-trip.
#[test]
fn passthrough_class_in_function_body() {
    let src = "function makeFoo() { class Foo { greet() { return 1; } } return new Foo(); }";
    let m = parse_and_lower(src).expect("PassThrough should not error");
    let out = emit(&m);
    assert!(
        out.contains("class Foo"),
        "class body should survive: {out}"
    );
    assert!(out.contains("greet"), "method name should survive: {out}");
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

/// Function body with a generator — also PassThrough.
#[test]
fn passthrough_generator_function() {
    let src = "function gen() { function* inner() { yield 1; yield 2; } return inner(); }";
    let m = parse_and_lower(src).expect("PassThrough should not error");
    let out = emit(&m);
    assert!(out.contains("function*"), "generator should survive: {out}");
    assert!(out.contains("yield"), "yield should survive: {out}");
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

/// Function body with try/catch — PassThrough, since try is unsupported.
#[test]
fn passthrough_try_catch() {
    let src = "function safe(x) { try { return x.foo; } catch (e) { return null; } }";
    let m = parse_and_lower(src).expect("PassThrough should not error");
    let out = emit(&m);
    assert!(out.contains("try"), "try should survive: {out}");
    assert!(out.contains("catch"), "catch should survive: {out}");
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

// === T2 Stream C pin + oxc_codegen tests ===
//
// These tests exercise the capture-by-passthrough pin propagation
// through `lower_module_with_config` and the oxc_codegen-driven
// PassThrough body emit. They construct SSA modules directly (rather
// than going through `parse_and_lower`) for two reasons:
//   1) The front end (Stream B) does not yet emit
//      `Stmt::FuncDecl`/`Expr::Func` from OXC AST, so the source-text
//      route cannot produce inner closures.
//   2) Direct construction lets each test pin down a single behavior
//      (pin-with-collision, synthesized-capture-panic, round-trip)
//      without dragging in the full pipeline's noise.

/// Build a minimal Lowered Function with one block, two named values
/// (both preferring `tmp`), and a return.
fn build_lowered_with_two_tmps(module: &mut ssa::Module, fname: &str) -> (FuncId, Value, Value) {
    let mut b = FunctionBuilder::new(module, fname.to_string(), 0);
    let bb = b.new_block();
    b.switch_to_block(bb);
    let v0 = b
        .add_inst(InstKind::AllocObject)
        .expect("AllocObject produces a value");
    let v1 = b
        .add_inst(InstKind::AllocObject)
        .expect("AllocObject produces a value");
    let tmp = b.intern("tmp");
    b.set_source_name(v0, tmp);
    b.set_source_name(v1, tmp);
    b.set_terminator(Terminator::Return { val: None });
    let fid = b.finish();
    (fid, v0, v1)
}

/// Pin pathology: two Values both prefer the source name `tmp`, but
/// one is captured-by-passthrough. The renamer must route the pinned
/// Value to the bare `tmp`, and disambiguate the other to `tmp_1`.
#[test]
fn passthrough_pin_disambiguates_collision() {
    let mut sm = ssa::Module::new();
    let (_fid, v_pin, _v_other) = build_lowered_with_two_tmps(&mut sm, "host");
    // Mark the first Value as captured-by-passthrough. The wiring in
    // `lower_module_with_config` reads this directly from the
    // Function struct and populates EmitConfig.immovable_names.
    sm.functions[0].captured_by_passthrough.push(v_pin.0);
    sm.validate().expect("module validates");

    let tm = lower_module_with_config(&sm, &EmitConfig::default());
    let out = emit(&tm);

    // The pinned value must keep `tmp`; the other must be suffixed.
    // We don't constrain WHICH let comes first (the renamer is
    // allocation-order-driven), but exactly one of `let tmp;` and
    // `let tmp_1;` must appear, in some order.
    assert!(
        out.contains("let tmp;") && out.contains("let tmp_1;"),
        "expected `tmp` and `tmp_1` lets in output, got:\n{out}"
    );
}

/// Synthesized-capture into PassThrough panics. We hand-build a
/// LoweredBody whose `captured_by_passthrough` references a Value
/// without a `source_name`, then trigger the SSA→tree lowering. The
/// constraint check at `lower_function`'s pin-extraction should fire
/// — but in fact our stricter check runs at SSA build time. So we
/// construct via FunctionBuilder + mark.
#[test]
#[should_panic(expected = "synthesized intermediate cannot be captured")]
fn synthesized_capture_into_passthrough_panics() {
    let mut sm = ssa::Module::new();
    let mut b = FunctionBuilder::new(&mut sm, "host".to_string(), 0);
    let bb = b.new_block();
    b.switch_to_block(bb);
    // AllocObject → produces a Value with NO source_name (synthesized
    // intermediate).
    let v = b
        .add_inst(InstKind::AllocObject)
        .expect("AllocObject produces a value");
    b.mark_value_captured_by_passthrough(v);
    b.set_terminator(Terminator::Return { val: None });
    // finish() runs the constraint check and panics with the spec'd
    // message.
    let _fid = b.finish();
}

/// Round-trip stability: a PassThrough function body, once emitted
/// via the oxc_codegen path, is byte-identical when piped through
/// `parse_and_lower → emit` a second time. Comments are dropped on
/// the first pass; the second pass is a no-op on the comment-stripped
/// form.
#[test]
fn passthrough_roundtrip_byte_identical() {
    // A function with a try/catch routes through PassThrough (try is
    // unsupported by the lowerer) — same path the
    // `passthrough_try_catch` test uses.
    let src = "function safe(x) { try { return x.foo; } catch (e) { return null; } }";
    let m1 = parse_and_lower(src).expect("first lower");
    let out1 = emit(&m1);

    let m2 = parse_and_lower(&out1).expect("re-parse first emit");
    let out2 = emit(&m2);

    assert_eq!(
        out1, out2,
        "PassThrough round-trip must be byte-identical after the first pass.\
        \nout1:\n{out1}\nout2:\n{out2}"
    );
}

/// The wiring test: when a Lowered function's
/// `captured_by_passthrough` set is populated, the matching Value's
/// source name appears unchanged in the emitted JS even with no
/// caller-supplied EmitConfig. Companions
/// `passthrough_pin_disambiguates_collision` above; this one targets
/// the no-collision happy path.
#[test]
fn passthrough_pin_preserves_source_name_with_default_config() {
    let mut sm = ssa::Module::new();
    let mut b = FunctionBuilder::new(&mut sm, "host".to_string(), 0);
    let bb = b.new_block();
    b.switch_to_block(bb);
    let v = b
        .add_inst(InstKind::AllocObject)
        .expect("AllocObject produces a value");
    let pinned_name = b.intern("captured");
    b.set_source_name(v, pinned_name);
    b.mark_value_captured_by_passthrough(v);
    b.set_terminator(Terminator::Return { val: None });
    let _fid = b.finish();
    sm.validate().expect("module validates");

    let tm = lower_module_with_config(&sm, &EmitConfig::default());
    let out = emit(&tm);
    assert!(
        out.contains("captured"),
        "pinned source name must appear in output:\n{out}"
    );
}

// Silence dead-code warnings for the LoweredBody / Function / etc.
// types imported above — at least one path uses each.
#[allow(dead_code)]
fn _silence_unused() {
    let _: Option<&LoweredBody> = None;
    let _: Option<&Function> = None;
    let _: Option<&FunctionBody> = None;
    let _: Option<&PassThroughBody> = None;
    let _: Option<&BasicBlock> = None;
    let _: Option<&Constant> = None;
    let _: Option<&ValueDef> = None;
    let _: Option<&Operand> = None;
    let _: Option<&BlockId> = None;
}

// === T2 Stream D cross-check tests ===
//
// The asymmetric debug cross-check in `lower.rs::lower_top_function`
// fires when the disposition walker says `Lowered` but the lowering
// returns `Error::Unsupported`. These tests inject hand-crafted
// "lying" walkers via the test seam `parse_and_lower_with_decider`.
//
// Release-build behavior: `cfg!(debug_assertions)` is `false`, the
// panic branch is dead code, and the cross-check compiles out — no
// runtime cost. Verified by inspection (the `if cfg!(debug_assertions)`
// block in `lower.rs::lower_top_function_with_decider`); the
// integration test below documents the debug-build behavior.

/// A "lying" walker that returns `Lowered` for everything. Combined
/// with a function body that contains a class declaration, this must
/// trip the cross-check: the lowering will return
/// `Error::Unsupported("class declaration")`, but the walker said
/// `Lowered`, so the asymmetric debug check panics.
fn always_lowered(_p: &js::FormalParameters, _b: &js::FunctionBody) -> Disposition {
    Disposition::Lowered
}

/// A "lying" walker that returns `PassThrough` for everything. Even
/// for fully-lowerable bodies, the function ends up preserved as
/// source text. The cross-check intentionally does NOT catch this
/// direction — see the comment in `lower.rs` and T2_PLAN.md Stream D.
fn always_passthrough(_p: &js::FormalParameters, _b: &js::FunctionBody) -> Disposition {
    Disposition::PassThrough
}

/// Walker too permissive: it claims `class Foo {}` is lowerable. The
/// real lowering can't represent a class declaration, so it returns
/// `Error::Unsupported`. The debug-only cross-check must convert that
/// into a panic naming the construct.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "T2 cross-check")]
fn cross_check_panics_when_walker_too_permissive() {
    // Class declaration inside a function body is the canonical case:
    // the honest walker would route this to PassThrough, but our
    // `always_lowered` liar forces the Lowered path. The lowering hits
    // `Error::Unsupported("class declaration")` and the cross-check
    // panics.
    let src = "function makeFoo() { class Foo {} return new Foo(); }";
    let _ = parse_and_lower_with_decider(src, always_lowered);
}

/// Same lying walker, with a more specific assertion that the panic
/// message names the offending function.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "function `makeFoo`")]
fn cross_check_panic_message_names_function() {
    let src = "function makeFoo() { class Foo {} }";
    let _ = parse_and_lower_with_decider(src, always_lowered);
}

/// Walker too conservative: it claims a fully-lowerable function is
/// PassThrough. This is the intentionally-uncaught direction — no
/// correctness issue, just a missed optimization. Asserting that the
/// pipeline does NOT panic documents the gap. If a future change
/// catches this direction (e.g. via the scratch-module fallback
/// described in T2_PLAN.md), this test will need to be updated.
#[test]
fn cross_check_does_not_fire_when_walker_too_conservative() {
    let src = "function add(a, b) { return a + b; }";
    let m = parse_and_lower_with_decider(src, always_passthrough)
        .expect("too-conservative walker must not error");
    let out = emit(&m);
    // The body survives as PassThrough source text rather than getting
    // optimized — confirms we took the PassThrough path without
    // panicking.
    assert!(out.contains("function add"), "function survives: {out}");
    assert!(out.contains("a + b"), "body survives verbatim: {out}");
}

/// A program with one Lowered function (fully optimizable) and one
/// PassThrough function (contains a class). The pipeline must not
/// abort, and both functions must appear in the output.
#[test]
fn mixed_lowered_and_passthrough_in_module() {
    let src = r#"
        function add(a, b) { return a + b; }
        function makeWidget() { class Widget {} return new Widget(); }
    "#;
    let m = parse_and_lower(src).expect("mixed module should not error");
    let out = emit(&m);
    assert!(out.contains("function add"));
    assert!(out.contains("function makeWidget"));
    assert!(out.contains("class Widget"));
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

// === T2 Stream B closure tests ===
//
// These tests exercise the front-end → SSA → tree-IR → emit round-trip
// for nested function declarations, function expressions, arrow
// functions, and the forward-ref / reassigned-capture rules from
// `closures-disposition-rule`.

/// Top-level IIFE: `(function(x){ return x+1; })(2)`. The function
/// expression must lower to an AllocClosure + Call; the round-trip
/// output must re-parse as valid JS.
#[test]
fn stream_b_iife() {
    let src = "(function(x){ return x+1; })(2);";
    let m = parse_and_lower(src).expect("IIFE should lower");
    let out = emit(&m);
    parse_and_lower(&out).expect("emitted IIFE must re-parse");
}

/// Function expression bound to a let variable.
#[test]
fn stream_b_function_expression_bound() {
    let src = "let f = function(x){return x*2;}; f(5);";
    let m = parse_and_lower(src).expect("function expression should lower");
    let out = emit(&m);
    assert!(
        out.contains("function"),
        "function literal must survive: {out}"
    );
    parse_and_lower(&out).expect("re-parse");
}

/// Arrow function bound to a let.
#[test]
fn stream_b_arrow_function() {
    let src = "let f = (x) => x*2; f(5);";
    let m = parse_and_lower(src).expect("arrow should lower");
    let out = emit(&m);
    parse_and_lower(&out).expect("re-parse");
}

/// Nested function declaration inside an outer function body.
#[test]
fn stream_b_nested_decl() {
    let src = "function outer(){ function inner(x){return x+1;} return inner(2); }";
    let m = parse_and_lower(src).expect("nested decl should lower");
    let out = emit(&m);
    assert!(out.contains("function outer"));
    assert!(out.contains("function inner") || out.contains("inner"));
    parse_and_lower(&out).expect("re-parse");
}

/// A nested arrow that captures an outer let.
#[test]
fn stream_b_capture() {
    let src = "function outer(){ let y = 10; return function(x){ return x + y; }; }";
    let m = parse_and_lower(src).expect("capture should lower");
    let out = emit(&m);
    parse_and_lower(&out).expect("re-parse");
}

/// Forward-ref capture: inner references `y` before it's bound. Must
/// be re-classified to PassThrough.
#[test]
fn stream_b_forward_ref_capture_forces_passthrough() {
    // SSA lowering must not panic on forward-ref captures — the rule
    // re-classifies the inner literal to PassThrough so the
    // AllocClosure capture for `y` is sourced from the outer scope at
    // emit time without being read SSA-style at the function-expression
    // position.
    let src = "function outer(){ let f = function(x){ return x + y; }; let y = 10; return f(1); }";
    let out = ssa_roundtrip(src);
    // The inner body must survive verbatim through PassThrough.
    assert!(
        out.contains("x + y") || out.contains("x+y"),
        "inner body lost: {out}"
    );
}

/// Reassigned capture: outer reassigns `y` after the closure is
/// created. Must be re-classified to PassThrough.
#[test]
fn stream_b_reassigned_capture_forces_passthrough() {
    let src = "function outer(){ let y = 10; let f = function(x){ return x + y; }; y = 20; return f(1); }";
    let out = ssa_roundtrip(src);
    assert!(
        out.contains("x + y") || out.contains("x+y"),
        "inner body lost: {out}"
    );
}

/// Plain (non-violating) capture goes through SSA cleanly.
#[test]
fn stream_b_capture_through_ssa() {
    let src = "function outer(){ let y = 10; return function(x){ return x + y; }; }";
    let _ = ssa_roundtrip(src);
}

/// Top-level IIFE through SSA.
#[test]
fn stream_b_iife_through_ssa() {
    let src = "function main(){ return (function(x){ return x+1; })(2); }";
    let _ = ssa_roundtrip(src);
}

/// Nested function declaration through SSA.
#[test]
fn stream_b_nested_decl_through_ssa() {
    let src = "function outer(){ function inner(x){return x+1;} return inner(2); }";
    let _ = ssa_roundtrip(src);
}

// === Top-level non-function statement preservation through SSA ===
//
// Previously: `Item::Stmt` was discarded by `lower_ssa::discover_module`
// and never re-emitted by `lower_tree::lower_module`, so a top-level
// IIFE produced an empty output. The fix preserves non-function
// top-level statements as source-slice passthrough items that survive
// the SSA round-trip and re-emit verbatim.

/// Top-level IIFE round-trips through SSA without producing empty
/// output. Minimal reproduction of the silent-data-loss bug.
#[test]
fn top_level_iife_survives_ssa_roundtrip() {
    let src = "(function(){ var x = 1; })();";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "SSA round-trip dropped IIFE entirely");
    assert!(
        out.contains("function") && out.contains("var x"),
        "IIFE body must survive: {out}"
    );
}

/// Top-level function expression as a statement.
#[test]
fn top_level_function_expression_statement_survives() {
    let src = "(function foo(){ return 1; });";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "function-expression statement dropped");
    assert!(out.contains("foo"), "function name must survive: {out}");
}

/// Call expression whose callee is a function expression
/// (canonical IIFE shape).
#[test]
fn top_level_call_with_function_callee_survives() {
    let src = "(function(n){ return n*2; })(21);";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "IIFE with arg dropped");
    assert!(out.contains("21"), "argument must survive: {out}");
}

/// Arrow IIFE.
#[test]
fn top_level_arrow_iife_survives() {
    let src = "(() => { return 7; })();";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "arrow IIFE dropped");
    assert!(out.contains("=>"), "arrow shape must survive: {out}");
}

/// Named function expression IIFE.
#[test]
fn top_level_named_funcexpr_iife_survives() {
    let src = "(function foo(){ return 1; })();";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "named funcexpr IIFE dropped");
    assert!(out.contains("foo"), "named funcexpr name must survive: {out}");
}

/// IIFE as an argument to another call.
#[test]
fn top_level_iife_as_argument_survives() {
    let src = "console.log((function(){ return 42; })());";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "IIFE-as-argument dropped");
    assert!(out.contains("42") && out.contains("console.log"));
}

/// Mixed top-level: a function declaration followed by an IIFE call
/// must preserve both, in original order.
#[test]
fn top_level_mixed_function_and_iife_preserves_order() {
    let src = "function asd(){ return 1; }\n(function(){ var x = 1; })();";
    let out = ssa_roundtrip(src);
    let asd_pos = out.find("function asd").expect("function decl missing");
    let iife_pos = out.find("var x").expect("IIFE body missing");
    assert!(
        asd_pos < iife_pos,
        "source order not preserved across SSA round-trip:\n{out}"
    );
}
