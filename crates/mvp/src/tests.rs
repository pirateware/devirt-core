use crate::DEMO;
use crate::emit::emit;
use crate::error::Error;
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
