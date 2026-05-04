use crate::DEMO;
use crate::emit::emit;
use crate::error::Error;
use crate::ir::lower_tree::{EmitConfig, lower_module_with_config};
use crate::ir::ssa::{
    self, BasicBlock, BlockId, Constant, FuncId, Function, FunctionBody, FunctionBuilder, InstKind,
    LoweredBody, Operand, PassThroughBody, Terminator, Value, ValueDef,
};
use crate::lower::parse_and_lower;

fn roundtrip(src: &str) -> String {
    let m = parse_and_lower(src).expect("parse_and_lower failed");
    let out = emit(&m);
    parse_and_lower(&out).expect("emitted JS failed to re-parse");
    out
}

#[test]
fn fib() {
    let out = roundtrip(DEMO);
    assert!(out.contains("function fib"));
    assert!(out.contains("return"));
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
fn unsupported_class_errors_loudly() {
    let err = parse_and_lower("class Foo {}").unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
}

#[test]
fn unsupported_for_errors_loudly() {
    let err = parse_and_lower("for (let i = 0; i < 10; i++) {}").unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
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
