use crate::DEMO;
use crate::emit::emit;
use crate::error::Error;
use crate::ir::disposition::Disposition;
use crate::lower::{parse_and_lower, parse_and_lower_with_decider};
use oxc_ast::ast as js;

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
