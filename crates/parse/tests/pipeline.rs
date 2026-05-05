const DEMO: &str = r#"
function asd() {
    var numbers = [1, 2, 3, 4, 5];

    var doubled = numbers.map(function(n) {
        return n * 2;
    });

    var sum = numbers.reduce(function(acc, n) {
        return acc + n;
    }, 0);

    console.log('Original:', numbers);
    console.log('Doubled:', doubled);
    console.log('Sum:', sum);
}
"#;
use emit::emit;
use parse::disposition::Disposition;
use parse::lower_ssa;
use parse::lower_tree;
use parse::lower_tree::{EmitConfig, lower_module_with_config};
use ir::ssa::{
    self, BasicBlock, BlockId, Constant, FuncId, Function, FunctionBody, FunctionBuilder, InstKind,
    LoweredBody, Operand, PassThroughBody, Terminator, Value, ValueDef,
};
use parse::{parse_and_lower, parse_and_lower_with_decider};
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
    use ir::tree::Item;
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
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let _ = roundtrip(src);
}

#[test]
fn update_lowers_in_function_body() {
    let src = "function inc(x) { return x++; }";
    let m = parse_and_lower(src).expect("update body must lower");
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let _ = roundtrip(src);
}

/// Regression: previously caused infinite recursion in `lower_tree::walk`.
///
/// Shape: a `while`/`for` loop whose body contains a nested if whose
/// branches return on one side and continue (back-edge) on the other.
/// `find_diamond_join` would pick a non-header join (an early-return
/// block) as `inner_stop`, the recursive `walk` on the back-edge branch
/// would then re-enter the loop header *without* the header being its
/// `stop`, hit the cached `loop_info`, and recursively re-emit the loop
/// — eating the stack. Fixed by tracking `loop_headers_open` and
/// emitting `continue` when re-entering an open header.
///
/// Discovered on `lodash_cloneDeep.high.js` (assocIndexOf). This is the
/// minimal hand-written reproducer.
#[test]
fn nested_if_with_continue_in_loop_does_not_recurse_forever() {
    let src = r#"
        function find(arr, target) {
            for (var i = 0; i < arr.length; i++) {
                if (arr[i] === target) {
                    if (i > 10) {
                        return i;
                    } else {
                        return -1;
                    }
                }
            }
            return -1;
        }
    "#;
    let out = ssa_roundtrip(src);
    assert!(out.contains("while"), "SSA round-trip must emit a loop: {out}");
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

// === Corpus parse-only regression check ===

#[test]
fn tier1_corpus_parse_and_lower_no_overflow() {
    // Check that the full pipeline doesn't overflow for any tier1 corpus file.
    // This catches regressions where newly-Lowered functions overflow the
    // recursive descent in either the parse or SSA lowering phase.
    const FILES: &[(&str, &str)] = &[
        ("datefns_addDays", include_str!("../../../corpus/realworld/tier1/datefns_addDays.js")),
        ("datefns_addMonths", include_str!("../../../corpus/realworld/tier1/datefns_addMonths.js")),
        ("datefns_differenceInDays", include_str!("../../../corpus/realworld/tier1/datefns_differenceInDays.js")),
        ("datefns_endOfMonth", include_str!("../../../corpus/realworld/tier1/datefns_endOfMonth.js")),
        ("datefns_format", include_str!("../../../corpus/realworld/tier1/datefns_format.js")),
        ("datefns_formatDistance", include_str!("../../../corpus/realworld/tier1/datefns_formatDistance.js")),
        ("datefns_getDay", include_str!("../../../corpus/realworld/tier1/datefns_getDay.js")),
        ("datefns_isAfter", include_str!("../../../corpus/realworld/tier1/datefns_isAfter.js")),
        ("datefns_isBefore", include_str!("../../../corpus/realworld/tier1/datefns_isBefore.js")),
        ("datefns_parseISO", include_str!("../../../corpus/realworld/tier1/datefns_parseISO.js")),
        ("datefns_startOfDay", include_str!("../../../corpus/realworld/tier1/datefns_startOfDay.js")),
        ("datefns_subDays", include_str!("../../../corpus/realworld/tier1/datefns_subDays.js")),
        ("lodash_camelCase", include_str!("../../../corpus/realworld/tier1/lodash_camelCase.js")),
        ("lodash_capitalize", include_str!("../../../corpus/realworld/tier1/lodash_capitalize.js")),
        ("lodash_chunk", include_str!("../../../corpus/realworld/tier1/lodash_chunk.js")),
        ("lodash_cloneDeep", include_str!("../../../corpus/realworld/tier1/lodash_cloneDeep.js")),
        ("lodash_compact", include_str!("../../../corpus/realworld/tier1/lodash_compact.js")),
        ("lodash_constant", include_str!("../../../corpus/realworld/tier1/lodash_constant.js")),
        ("lodash_curry", include_str!("../../../corpus/realworld/tier1/lodash_curry.js")),
        ("lodash_debounce", include_str!("../../../corpus/realworld/tier1/lodash_debounce.js")),
        ("lodash_difference", include_str!("../../../corpus/realworld/tier1/lodash_difference.js")),
        ("lodash_escape", include_str!("../../../corpus/realworld/tier1/lodash_escape.js")),
        ("lodash_flatten", include_str!("../../../corpus/realworld/tier1/lodash_flatten.js")),
        ("lodash_flow", include_str!("../../../corpus/realworld/tier1/lodash_flow.js")),
        ("lodash_get", include_str!("../../../corpus/realworld/tier1/lodash_get.js")),
        ("lodash_groupBy", include_str!("../../../corpus/realworld/tier1/lodash_groupBy.js")),
        ("lodash_has", include_str!("../../../corpus/realworld/tier1/lodash_has.js")),
        ("lodash_identity", include_str!("../../../corpus/realworld/tier1/lodash_identity.js")),
        ("lodash_intersection", include_str!("../../../corpus/realworld/tier1/lodash_intersection.js")),
        ("lodash_invert", include_str!("../../../corpus/realworld/tier1/lodash_invert.js")),
        ("lodash_isEqual", include_str!("../../../corpus/realworld/tier1/lodash_isEqual.js")),
        ("lodash_isPlainObject", include_str!("../../../corpus/realworld/tier1/lodash_isPlainObject.js")),
        ("lodash_kebabCase", include_str!("../../../corpus/realworld/tier1/lodash_kebabCase.js")),
        ("lodash_keyBy", include_str!("../../../corpus/realworld/tier1/lodash_keyBy.js")),
        ("lodash_mapKeys", include_str!("../../../corpus/realworld/tier1/lodash_mapKeys.js")),
        ("lodash_mapValues", include_str!("../../../corpus/realworld/tier1/lodash_mapValues.js")),
        ("lodash_memoize", include_str!("../../../corpus/realworld/tier1/lodash_memoize.js")),
        ("lodash_merge", include_str!("../../../corpus/realworld/tier1/lodash_merge.js")),
        ("lodash_noop", include_str!("../../../corpus/realworld/tier1/lodash_noop.js")),
        ("lodash_omit", include_str!("../../../corpus/realworld/tier1/lodash_omit.js")),
        ("lodash_orderBy", include_str!("../../../corpus/realworld/tier1/lodash_orderBy.js")),
        ("lodash_partial", include_str!("../../../corpus/realworld/tier1/lodash_partial.js")),
        ("lodash_partition", include_str!("../../../corpus/realworld/tier1/lodash_partition.js")),
        ("lodash_pick", include_str!("../../../corpus/realworld/tier1/lodash_pick.js")),
        ("lodash_range", include_str!("../../../corpus/realworld/tier1/lodash_range.js")),
        ("lodash_set", include_str!("../../../corpus/realworld/tier1/lodash_set.js")),
        ("lodash_snakeCase", include_str!("../../../corpus/realworld/tier1/lodash_snakeCase.js")),
        ("lodash_sortBy", include_str!("../../../corpus/realworld/tier1/lodash_sortBy.js")),
        ("lodash_startCase", include_str!("../../../corpus/realworld/tier1/lodash_startCase.js")),
        ("lodash_throttle", include_str!("../../../corpus/realworld/tier1/lodash_throttle.js")),
        ("lodash_times", include_str!("../../../corpus/realworld/tier1/lodash_times.js")),
        ("lodash_union", include_str!("../../../corpus/realworld/tier1/lodash_union.js")),
        ("lodash_uniq", include_str!("../../../corpus/realworld/tier1/lodash_uniq.js")),
        ("lodash_uniqBy", include_str!("../../../corpus/realworld/tier1/lodash_uniqBy.js")),
        ("lodash_without", include_str!("../../../corpus/realworld/tier1/lodash_without.js")),
        ("lodash_zip", include_str!("../../../corpus/realworld/tier1/lodash_zip.js")),
    ];
    for (name, src) in FILES {
        let tm = parse_and_lower(src).unwrap_or_else(|e| panic!("{name}: parse_and_lower failed: {e:?}"));
        let sm = lower_ssa::lower_module(&tm);
        sm.validate().unwrap_or_else(|e| panic!("{name}: SSA validation failed: {e:?}"));
        let tm2 = lower_tree::lower_module(&sm);
        let out = emit::emit(&tm2);
        parse_and_lower(&out).unwrap_or_else(|e| panic!("{name}: re-parse failed: {e:?}"));
    }
}

// === MultipleDeclarators lowering tests ===

#[test]
fn multi_declarator_var_lowers_to_separate_stmts() {
    let src = "function f() { var a = 1, b = 2; return a + b; }";
    let m = parse_and_lower(src).expect("multi-declarator var must lower");
    assert!(
        matches!(m.items[0], ir::tree::Item::Function(_)),
        "multi-declarator var function must be Lowered: {:?}",
        m.items[0]
    );
    let out = roundtrip(src);
    assert!(out.contains("let a"), "expected `let a` in: {out}");
    assert!(out.contains("let b"), "expected `let b` in: {out}");
}

#[test]
fn multi_declarator_let_no_init_no_spurious_assignment() {
    // `let x, y = 1;` — x has no initializer. The lowering must NOT emit
    // `x = undefined`, which would be observable via property descriptors.
    let src = "function f() { let x, y = 1; return y; }";
    let m = parse_and_lower(src).expect("multi-declarator let must lower");
    assert!(matches!(m.items[0], ir::tree::Item::Function(_)));
    let out = roundtrip(src);
    assert!(
        !out.contains("x = undefined"),
        "spurious `x = undefined` found in: {out}"
    );
    assert!(out.contains("let x"), "expected bare `let x` in: {out}");
    assert!(out.contains("let y"), "expected `let y` in: {out}");
}

#[test]
fn multi_declarator_let_second_can_reference_first() {
    // `let x = 1, y = x + 1;` — after splitting into two separate `let` statements,
    // the second initializer still sees `x` because it's evaluated left-to-right.
    let src = "function f() { let x = 1, y = x + 1; return y; }";
    let m = parse_and_lower(src).expect("must lower");
    assert!(matches!(m.items[0], ir::tree::Item::Function(_)));
    let out = roundtrip(src);
    // Both bindings must appear as separate let statements in declaration order.
    let x_pos = out.find("let x").expect("`let x` missing in: {out}");
    let y_pos = out.find("let y").expect("`let y` missing in: {out}");
    assert!(x_pos < y_pos, "`let x` must precede `let y` in: {out}");
}

#[test]
fn multi_declarator_const_lowers() {
    let src = "function f() { const p = 1, q = 2; return p + q; }";
    let m = parse_and_lower(src).expect("multi-declarator const must lower");
    assert!(matches!(m.items[0], ir::tree::Item::Function(_)));
    let out = roundtrip(src);
    assert!(out.contains("const p"), "expected `const p` in: {out}");
    assert!(out.contains("const q"), "expected `const q` in: {out}");
}

#[test]
fn multi_declarator_destructuring_stays_pass_through() {
    // Each declarator with a destructuring pattern is still a blocker (DestructuringBinding).
    // The function must be PassThrough, but the source must survive verbatim.
    let src = "function f(x, y) { let {a} = x, [b] = y; return a + b; }";
    let m = parse_and_lower(src).expect("PassThrough must not error");
    assert!(
        matches!(m.items[0], ir::tree::Item::PassThroughFunction(_)),
        "destructuring multi-declarator must remain PassThrough: {:?}",
        m.items[0]
    );
    let out = emit(&m);
    assert!(out.contains("let {a}") || out.contains("let {"), "source must survive: {out}");
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

#[test]
fn multi_declarator_sole_blocker_becomes_lowered() {
    // A function whose only blocker was MultipleDeclarators is now fully Lowered.
    let src = "function onlyMultiDecl() { var a = 1, b = 2, c = 3; return a + b + c; }";
    let m = parse_and_lower(src).expect("sole-blocker function must lower");
    assert!(
        matches!(m.items[0], ir::tree::Item::Function(_)),
        "sole-MultipleDeclarators function must become Lowered: {:?}",
        m.items[0]
    );
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
    Disposition::PassThrough(std::collections::BTreeSet::new())
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
    assert!(
        !out.trim().is_empty(),
        "SSA round-trip dropped IIFE entirely"
    );
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
    assert!(
        !out.trim().is_empty(),
        "function-expression statement dropped"
    );
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
    assert!(
        out.contains("foo"),
        "named funcexpr name must survive: {out}"
    );
}

/// IIFE as an argument to another call.
#[test]
fn top_level_iife_as_argument_survives() {
    let src = "console.log((function(){ return 42; })());";
    let out = ssa_roundtrip(src);
    assert!(!out.trim().is_empty(), "IIFE-as-argument dropped");
    assert!(out.contains("42") && out.contains("console.log"));
}

// === for-of loop tests ===

/// Basic `for (const x of arr)` — the function must be Lowered (not
/// PassThrough) and survive the full SSA round-trip.
#[test]
fn for_of_basic() {
    let src = "function f(arr) { for (const x of arr) { use(x); } }";
    let m = parse_and_lower(src).expect("for-of must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "for-of function must be Lowered, got: {:?}",
        m.items[0]
    );
    // Tree-IR emit: the for-of form is preserved.
    let out = emit(&m);
    assert!(out.contains("function f"), "function survives: {out}");
    // SSA round-trip: desugars to while.
    let out2 = ssa_roundtrip(src);
    assert!(out2.contains("while"), "SSA round-trip emits while: {out2}");
    parse_and_lower(&out2).expect("SSA-roundtrip emit must re-parse");
}

/// `for (let x of arr)` binding — let is also accepted.
#[test]
fn for_of_let_binding() {
    let src = "function f(arr) { for (let x of arr) { use(x); } }";
    let m = parse_and_lower(src).expect("for-of let must lower");
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let _ = ssa_roundtrip(src);
}

/// `for (var x of arr)` binding — var is also accepted.
#[test]
fn for_of_var_binding() {
    let src = "function f(arr) { for (var x of arr) { use(x); } }";
    let m = parse_and_lower(src).expect("for-of var must lower");
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let _ = ssa_roundtrip(src);
}

/// `for await...of` must remain PassThrough (not supported).
#[test]
fn for_of_await_passthrough() {
    let src = "async function f(arr) { for await (const x of arr) { use(x); } }";
    let m = parse_and_lower(src).expect("for-await-of must not error");
    use ir::tree::Item;
    // async function is already PassThrough due to the `async` flag,
    // but the for-await-of check should also trigger PassThrough.
    assert!(
        matches!(m.items[0], Item::PassThroughFunction(_)),
        "for-await-of must be PassThrough"
    );
}

/// Destructuring binding in for-of — must remain PassThrough.
#[test]
fn for_of_destructuring_passthrough() {
    let src = "function f(arr) { for (const {x} of arr) { use(x); } }";
    let m = parse_and_lower(src).expect("for-of destructuring must not error");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::PassThroughFunction(_)),
        "for-of destructuring must be PassThrough"
    );
}

/// for-of loop accessing the bound variable in the body.
#[test]
fn for_of_uses_binding_in_body() {
    let src = "function f(arr) { let sum = 0; for (const x of arr) { sum = sum + x; } return sum; }";
    let m = parse_and_lower(src).expect("for-of with body use must lower");
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    let out = ssa_roundtrip(src);
    parse_and_lower(&out).expect("round-trip must re-parse");
}

/// for-of with `return` inside must stay PassThrough.
/// The desugared iterator protocol does not call iterator.return() on abrupt
/// completion, so we conservatively reject these to avoid leaking resources.
#[test]
fn for_of_with_return_is_passthrough() {
    let src = "function f(arr) { for (const x of arr) { if (x > 5) return x; } }";
    let m = parse_and_lower(src).expect("must not error");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::PassThroughFunction(_)),
        "for-of with bare return must be PassThrough (iterator.return() not called on desugaring)"
    );
}

// === Spread operator tests ===

/// Spread in a call argument position.
#[test]
fn spread_in_call() {
    roundtrip("function f(a, b) { return foo(...a); }");
}

/// Spread in array literal.
#[test]
fn spread_in_array() {
    roundtrip("function f(a) { return [...a, 1, 2]; }");
}

/// Mixed spread and non-spread arguments in a call.
#[test]
fn spread_mixed_args() {
    roundtrip("function f(a, b) { return foo(1, ...a, b); }");
}

/// Spread in object literal.
#[test]
fn object_spread() {
    roundtrip("function f(a) { return {...a, x: 1}; }");
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

/// try/catch basic — function with a try/catch block should be Lowered
/// (not PassThrough) and round-trip through the full pipeline.
#[test]
fn try_catch_basic() {
    roundtrip("function f() { try { use(); } catch (e) { handle(e); } }");
}

/// try/finally — function with a try/finally block (no catch clause).
#[test]
fn try_finally() {
    roundtrip("function f() { try { use(); } finally { cleanup(); } }");
}

/// try/catch/finally — all three clauses present.
#[test]
fn try_catch_finally() {
    roundtrip("function f() { try { use(); } catch (e) { handle(e); } finally { cleanup(); } }");
}

/// try/catch without a catch parameter binding (`catch { }` — ES2019+).
#[test]
fn try_catch_no_param() {
    roundtrip("function f() { try { use(); } catch { handle(); } }");
}

// === Template literal tests ===

/// Simple template literal with one interpolation is lowered (not PassThrough).
#[test]
fn template_literal_simple() {
    use ir::tree::Item;
    let src = r#"function f(name) { return `Hello ${name}`; }"#;
    let m = parse_and_lower(src).expect("parse_and_lower failed");
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "template literal function must be Lowered, not PassThrough"
    );
    let out = roundtrip(src);
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

/// Template literal with no interpolations (pure string) is lowered.
#[test]
fn template_literal_no_interp() {
    use ir::tree::Item;
    let src = r#"function f() { return `hello world`; }"#;
    let m = parse_and_lower(src).expect("parse_and_lower failed");
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "no-interp template literal function must be Lowered"
    );
    let out = roundtrip(src);
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

/// Template literal with multiple interpolations is lowered.
#[test]
fn template_literal_multi_expr() {
    use ir::tree::Item;
    let src = r#"function f(a, b) { return `${a} and ${b}`; }"#;
    let m = parse_and_lower(src).expect("parse_and_lower failed");
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "multi-expr template literal function must be Lowered"
    );
    let out = roundtrip(src);
    parse_and_lower(&out).expect("emitted JS must re-parse");
}

/// Tagged template literals must remain PassThrough (not panic).
#[test]
fn tagged_template_passthrough() {
    use ir::tree::Item;
    let src = r#"function f(content) { return html`<div>${content}</div>`; }"#;
    let m = parse_and_lower(src).expect("tagged template must not error");
    assert!(
        matches!(m.items[0], Item::PassThroughFunction(_)),
        "tagged template must be PassThrough"
    );
}

/// Template literal SSA round-trip: semantics survive the full pipeline.
#[test]
fn template_literal_ssa_roundtrip() {
    let src = r#"function f(name, age) { return `Hello ${name}, you are ${age} years old`; }"#;
    let out = ssa_roundtrip(src);
    parse_and_lower(&out).expect("SSA-roundtrip emit must re-parse");
}

/// Regression: template literal uses ToString (not ToPrimitive) for interpolated
/// values. For an object with both toString and valueOf, `${x}` must call
/// toString, not valueOf. The SSA desugaring through AddGeneric uses the `+`
/// operator which calls ToPrimitive (preferring valueOf), so this test documents
/// the known semantic gap.
///
/// TODO: Fix by introducing a dedicated `TemplateConcat` SSA instruction (or a
/// `ToString` coercion node) so template interpolation uses ToString rather than
/// the ToPrimitive path that `+` follows. Until then, functions whose templates
/// interpolate objects with custom valueOf (a rare obfuscation pattern) may
/// produce incorrect output after the SSA round-trip.
#[test]
fn template_literal_tostring_vs_toprimitive_roundtrip() {
    // After SSA roundtrip, `${x}` becomes `"" + x` which uses ToPrimitive.
    // The tree-IR direct path emits backtick syntax (correct ToString).
    // This test documents that the tree-IR path preserves the template form.
    let src = r#"function f(x) { return `value:${x}`; }"#;
    let m = parse_and_lower(src).expect("must lower");
    let tree_out = emit(&m);
    // Tree-IR emit must preserve backtick syntax (spec-correct path).
    assert!(
        tree_out.contains('`'),
        "tree-IR emit must preserve template literal backtick syntax, got: {tree_out}"
    );
    // SSA round-trip: documents that the form degrades to string concat.
    let ssa_out = ssa_roundtrip(src);
    // SSA path loses backtick form — this is the known ToString/ToPrimitive gap.
    // When TemplateConcat is implemented, update this assertion to expect backticks.
    assert!(
        !ssa_out.contains('`'),
        "SSA round-trip currently degrades to + concat (known gap): {ssa_out}"
    );
}

// ---------------------------------------------------------------------------
// Compound assignment in expression position. Covers each of the 12 lowerable
// operators across the three LHS forms (ident, static-property, computed-
// property) and the three positions where the assignment's value flows into
// surrounding code (call argument, while condition, for-update).
//
// Behavioral correctness rests on the SSA decomposition emitting LHS reads
// and writes in spec order: receiver, key, load, rhs, op, store. The tests
// here check that each shape survives both the tree-only and full SSA
// round-trips and that the emitted output re-parses.
// ---------------------------------------------------------------------------

const COMPOUND_OPS: &[&str] = &[
    "+=", "-=", "*=", "/=", "%=", "**=", "&=", "|=", "^=", "<<=", ">>=", ">>>=",
];

fn assert_ssa_roundtrip(src: &str) {
    let _ = ssa_roundtrip(src);
}

#[test]
fn compound_assign_ident_in_call_arg_all_ops() {
    for op in COMPOUND_OPS {
        let src = format!("function f(x) {{ g(x {op} 1); return x; }}");
        assert_ssa_roundtrip(&src);
    }
}

#[test]
fn compound_assign_static_property_in_call_arg_all_ops() {
    for op in COMPOUND_OPS {
        let src = format!("function f(o) {{ g(o.x {op} 1); return o.x; }}");
        assert_ssa_roundtrip(&src);
    }
}

#[test]
fn compound_assign_computed_property_in_call_arg_all_ops() {
    for op in COMPOUND_OPS {
        let src = format!(
            "function f(o, k) {{ g(o[k] {op} 1); return o[k]; }}"
        );
        assert_ssa_roundtrip(&src);
    }
}

#[test]
fn compound_assign_in_while_condition() {
    let src = "function f(n) { while (n -= 1) { g(n); } return n; }";
    let out = ssa_roundtrip(src);
    assert!(out.contains("while"), "must keep while loop: {out}");
}

#[test]
fn compound_assign_in_for_update() {
    let src = "
        function f(n) {
            let s = 0;
            for (let i = 0; i < n; i += 2) { s = s + i; }
            return s;
        }
    ";
    assert_ssa_roundtrip(src);
}

#[test]
fn compound_assign_chained_nested() {
    // a += (b += 1): inner evaluates first, contributes its computed
    // value to the outer add. SSA decomposition emits two read-modify
    // -write sequences in source order.
    let src = "function f(a, b) { return a += (b += 1); }";
    assert_ssa_roundtrip(src);
}

#[test]
fn compound_assign_with_side_effecting_lhs_index() {
    // arr[i++] += 1 — the index expression must be evaluated exactly
    // once. The SSA convert reads `i++` as a single Update instruction
    // that emits one increment; the result is bound to a fresh value
    // and reused for both load and store.
    let src = "
        function f(arr, i) {
            arr[i++] += 1;
            return [arr, i];
        }
    ";
    assert_ssa_roundtrip(src);
}

#[test]
fn compound_assign_with_call_lhs_obj_and_key() {
    // obj()[k()] += rhs — receiver and key are each side-effecting
    // calls. Both must be evaluated exactly once before the load.
    let src = "
        function f(obj, k, rhs) {
            return obj()[k()] += rhs;
        }
    ";
    assert_ssa_roundtrip(src);
}

#[test]
fn compound_assign_string_plus_eq() {
    // `+=` does ToPrimitive (string concat path).
    let src = "function f(s, t) { s += t; return s; }";
    assert_ssa_roundtrip(src);
}

#[test]
fn pow_binary_in_expression_position() {
    // The Pow lowering also unblocks plain `**`.
    let src = "function f(x, y) { return x ** y; }";
    assert_ssa_roundtrip(src);
}

#[test]
fn bitwise_binary_in_expression_position() {
    for op in ["&", "|", "^", "<<", ">>", ">>>"] {
        let src = format!("function f(x, y) {{ return x {op} y; }}");
        assert_ssa_roundtrip(&src);
    }
}

#[test]
fn compound_assign_ident_statement_position_still_lowers() {
    // Statement-position compound stays on the existing decomposition
    // path (lib.rs:lower_compound_assign). Covered here to pin it.
    for op in COMPOUND_OPS {
        let src = format!("function f(x) {{ x {op} 1; return x; }}");
        assert_ssa_roundtrip(&src);
    }
}

// ---------------------------------------------------------------------------
// For-init multi-declarator (var only). Keep first declarator in the for-init
// slot, hoist the rest as preceding `var` statements wrapped in a Block.
// `var` is function-scoped so the hoist preserves observable behavior.
// ---------------------------------------------------------------------------

#[test]
fn for_init_two_declarators_var_lowers() {
    let src = "function f(arr) {
        var s = 0;
        for (var i = 0, n = arr.length; i < n; i++) { s = s + arr[i]; }
        return s;
    }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "for-init multi-declarator (var) must lower, got {:?}",
        m.items[0]
    );
    assert_ssa_roundtrip(src);
}

#[test]
fn for_init_three_declarators_var_lowers() {
    let src = "function f(arr) {
        var s = 0;
        for (var i = 0, n = arr.length, step = 1; i < n; i += step) {
            s = s + arr[i];
        }
        return s;
    }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    assert_ssa_roundtrip(src);
}

#[test]
fn for_init_four_declarators_uninitialized_mix_lowers() {
    // Obfuscated base64 decode loop shape: some declarators have
    // initializers, some don't.
    let src = "function f(s, table) {
        var out = '';
        for (var i = 0, acc, ch, j = 0; (ch = s.charAt(j++)); i++) {
            acc = table[ch];
            out = out + String.fromCharCode(acc);
        }
        return out;
    }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(matches!(m.items[0], Item::Function(_)));
    assert_ssa_roundtrip(src);
}

// ---------------------------------------------------------------------------
// ThrowStmt and ThisExpr. Both lower at the disposition level (functions
// containing them count as Lowered for coverage), and route through the
// SSA-fallback path (preserve as text). The roundtrip survives, the emit
// re-parses, and the syntactic forms come back through.
// ---------------------------------------------------------------------------

#[test]
fn throw_stmt_lowers_and_roundtrips() {
    let src = "function f() { throw new Error('x'); }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "throw must lower to Item::Function, got {:?}",
        m.items[0]
    );
    let out = ssa_roundtrip(src);
    assert!(out.contains("throw"), "throw must survive: {out}");
}

#[test]
fn throw_with_object_argument_roundtrips() {
    let src = "function f(msg) { throw { code: 500, message: msg }; }";
    let _ = ssa_roundtrip(src);
}

#[test]
fn throw_in_branch_roundtrips() {
    let src = "function f(x) { if (x < 0) { throw new RangeError('neg'); } return x; }";
    let _ = ssa_roundtrip(src);
}

#[test]
fn this_expr_lowers_and_roundtrips() {
    let src = "function f() { return this.x; }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "this must lower to Item::Function, got {:?}",
        m.items[0]
    );
    let out = ssa_roundtrip(src);
    assert!(out.contains("this"), "this must survive: {out}");
}

#[test]
fn this_method_call_roundtrips() {
    let src = "function f(arg) { return this.fn(arg); }";
    let _ = ssa_roundtrip(src);
}

#[test]
fn this_in_nested_arrow_roundtrips() {
    // Arrow captures `this` lexically. Outer body has no direct `this`
    // (so no SSA fallback for outer); inner arrow has `this` and gets
    // routed through fallback. Both halves emit correctly.
    let src = "function f() { return [1, 2, 3].map(x => this.scale * x); }";
    let _ = ssa_roundtrip(src);
}

#[test]
fn for_init_let_multi_declarator_stays_blocked() {
    // `let`/`const` in for-init multi-declarator must stay PassThrough.
    // Hoisting them would change per-iteration scope semantics.
    let src = "function f(arr) {
        for (let i = 0, n = arr.length; i < n; i++) {}
    }";
    let m = parse_and_lower(src).expect("must parse");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::PassThroughFunction(_)),
        "let multi-declarator for-init must stay PassThrough, got {:?}",
        m.items[0]
    );
}

// ---------------------------------------------------------------------------
// IR-shape canonicalization. The parser folds `Unary(Neg, NumericLiteral(0))`
// to `Number(-0.0)` and bare `Identifier("NaN" / "Infinity")` to the
// corresponding `Number` literal so the post-SSA tree-IR and re-parsed emit
// converge on the same canonical form. Without these folds the structural
// roundtrip check reports Number-vs-Unary / Number-vs-Ident divergences
// (33 cases across the corpus before the fix landed).
// ---------------------------------------------------------------------------

fn ret_expr(m: &ir::tree::Module) -> &ir::tree::Expr {
    use ir::tree::{Item, Stmt};
    let f = match &m.items[0] {
        Item::Function(f) => f,
        other => panic!("expected Item::Function, got {other:?}"),
    };
    match f.body.last().expect("non-empty body") {
        Stmt::Return(Some(e)) => e,
        other => panic!("expected Stmt::Return(Some), got {other:?}"),
    }
}

#[test]
fn negative_zero_folds_to_number_literal() {
    use ir::tree::Expr;
    let m = parse_and_lower("function f() { return -0; }").expect("must lower");
    match ret_expr(&m) {
        Expr::Number(n) => {
            assert_eq!(*n, 0.0, "value must be zero");
            assert!(
                n.is_sign_negative(),
                "must preserve IEEE -0 sign bit, got {}",
                n
            );
        }
        other => panic!("expected Expr::Number(-0.0), got {other:?}"),
    }
}

#[test]
fn positive_zero_does_not_fold_to_negative_zero() {
    // Sanity: the fold only triggers under UnaryNegation, not over a
    // bare positive zero.
    use ir::tree::Expr;
    let m = parse_and_lower("function f() { return 0; }").expect("must lower");
    match ret_expr(&m) {
        Expr::Number(n) => {
            assert_eq!(*n, 0.0);
            assert!(!n.is_sign_negative(), "must be +0, got {}", n);
        }
        other => panic!("expected Expr::Number(0.0), got {other:?}"),
    }
}

#[test]
fn nan_identifier_folds_to_number_nan() {
    use ir::tree::Expr;
    let m = parse_and_lower("function f() { return NaN; }").expect("must lower");
    match ret_expr(&m) {
        Expr::Number(n) => assert!(n.is_nan(), "expected NaN, got {n}"),
        other => panic!("expected Expr::Number(NaN), got {other:?}"),
    }
}

#[test]
fn infinity_identifier_folds_to_number_infinity() {
    use ir::tree::Expr;
    let m = parse_and_lower("function f() { return Infinity; }").expect("must lower");
    match ret_expr(&m) {
        Expr::Number(n) => assert_eq!(*n, f64::INFINITY),
        other => panic!("expected Expr::Number(Infinity), got {other:?}"),
    }
}

#[test]
fn negative_infinity_folds_through_unary() {
    // `-Infinity` is `UnaryExpression(Neg, Identifier("Infinity"))`.
    // The identifier folds to `Number(INFINITY)`, then the surrounding
    // unary preserves shape — same on both sides of the roundtrip.
    use ir::tree::{Expr, UnOp};
    let m = parse_and_lower("function f() { return -Infinity; }").expect("must lower");
    match ret_expr(&m) {
        Expr::Unary { op: UnOp::Neg, operand } => match operand.as_ref() {
            Expr::Number(n) => assert_eq!(*n, f64::INFINITY),
            other => panic!("expected Number(Infinity) inside unary, got {other:?}"),
        },
        other => panic!("expected Unary(Neg, Number(Infinity)), got {other:?}"),
    }
}

#[test]
fn negative_zero_roundtrip_preserves_sign() {
    // End-to-end: source `-0` survives parse → SSA → emit → re-parse
    // with the negative-zero bit pattern intact.
    let src = "function f() { return -0; }";
    let _ = ssa_roundtrip(src);
    // Tree-IR direct path also folds correctly.
    let m = parse_and_lower(src).expect("must lower");
    let m2 = parse_and_lower(&emit(&m)).expect("emit must re-parse");
    use ir::tree::Expr;
    match (ret_expr(&m), ret_expr(&m2)) {
        (Expr::Number(a), Expr::Number(b)) => {
            assert!(a.is_sign_negative());
            assert!(
                b.is_sign_negative(),
                "re-parsed -0 must preserve sign, got {b}"
            );
        }
        (a, b) => panic!("expected matching Number(-0), got {a:?} vs {b:?}"),
    }
}

// ---------------------------------------------------------------------------
// `in` and `instanceof`. The dump on UnsupportedBinOp showed these as the
// only two operators rejected by the disposition walker; both lower as
// preserve-as-syntactic `Expr::Binary` with their respective `BinOp` variant
// and emit through the existing `(lhs op rhs)` formatter (the leading/trailing
// space around the keyword is supplied by `binop_str`'s wrapping).
// Tests are intentionally separate so a regression in one doesn't mask the
// other (the `Instanceof` SSA InstKind is older and was never exercised
// end-to-end before this ticket).
// ---------------------------------------------------------------------------

#[test]
fn in_operator_in_expression_lowers() {
    let src = "function f(k, o) { return k in o; }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "`in` must lower, got {:?}",
        m.items[0]
    );
    let out = ssa_roundtrip(src);
    assert!(out.contains(" in "), "`in` keyword must survive: {out}");
}

#[test]
fn in_operator_in_branch_condition_lowers() {
    let src = "function f(k, o) { if (k in o) { return 1; } return 0; }";
    let _ = ssa_roundtrip(src);
}

#[test]
fn instanceof_operator_in_expression_lowers() {
    let src = "function f(o, C) { return o instanceof C; }";
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "`instanceof` must lower, got {:?}",
        m.items[0]
    );
    let out = ssa_roundtrip(src);
    assert!(
        out.contains(" instanceof "),
        "`instanceof` keyword must survive: {out}"
    );
}

#[test]
fn instanceof_operator_in_branch_condition_lowers() {
    let src = "function f(o, C) { if (o instanceof C) { return 1; } return 0; }";
    let _ = ssa_roundtrip(src);
}

// ---------------------------------------------------------------------------
// RegExp literal. Preserved as the verbatim source slice (`/pattern/flags`);
// SSA maps to `InstKind::AllocRegExp` (PURE allocation, fresh identity per
// evaluation per ES2015+).
// ---------------------------------------------------------------------------

#[test]
fn regex_literal_lowers_and_roundtrips() {
    let src = r#"function f() { return /abc/g; }"#;
    let m = parse_and_lower(src).expect("must lower");
    use ir::tree::Item;
    assert!(
        matches!(m.items[0], Item::Function(_)),
        "regex literal must lower, got {:?}",
        m.items[0]
    );
    let out = ssa_roundtrip(src);
    assert!(out.contains("/abc/g"), "regex slice must survive: {out}");
}

#[test]
fn regex_literal_with_no_flags_roundtrips() {
    let src = r#"function f() { return /\d+/.exec("hi42"); }"#;
    let _ = ssa_roundtrip(src);
}

#[test]
fn regex_literal_in_call_arg_roundtrips() {
    let src = r#"function f(s) { return s.replace(/[\s]/g, ""); }"#;
    let _ = ssa_roundtrip(src);
}

#[test]
fn in_and_instanceof_combined_lower() {
    // The lodash_isPlainObject shape: nested instanceof inside a
    // larger expression. Confirms both operators co-exist cleanly.
    let src = r#"
        function f(o, C) {
            return ('key' in o) && (o instanceof C);
        }
    "#;
    let _ = ssa_roundtrip(src);
}
