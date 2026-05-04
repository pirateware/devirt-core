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
