use crate::DEMO;
use crate::emit::emit;
use crate::error::Error;
use crate::ir::lower_ssa;
use crate::ir::lower_tree;
use crate::lower::parse_and_lower;

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
