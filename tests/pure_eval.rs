//! Pure-call constant evaluation (src/passes/pure_eval.rs). Structural shape only;
//! value preservation (incl. the impure-receiver decline) lives in
//! tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    r.code
}

#[test]
fn pure_method_call_on_literal_folds() {
    assert_eq!(deob("x(['H','e','l','l','o'].join(''));").trim(), r#"x("Hello");"#);
    assert_eq!(deob("x('hello'.toUpperCase());").trim(), r#"x("HELLO");"#);
}

#[test]
fn single_use_string_receiver_folds() {
    // Single-use inlining feeds the literal receiver, which then collapses.
    let out = deob("var s = 'abc'; sink(s.toUpperCase());");
    assert!(out.contains(r#""ABC""#), "method call not folded: {out}");
    assert!(!out.contains("toUpperCase"), "call survived: {out}");
}

#[test]
fn unknown_receiver_method_call_is_kept() {
    // The receiver is an unknown host value (`window`, a free name outside the
    // side-effect-free globals), so the gate declines: the call survives untouched.
    let out = deob("sink(window.parts.join(','));");
    assert!(out.contains("join"), "unknown-receiver call wrongly folded: {out}");
}

#[test]
fn immutable_aggregate_receiver_folds() {
    // A variable-bound const array receiver resolves via sandbox materialization.
    let out = deob("var a = ['x','y','z']; sink(a.join('-'));");
    assert!(out.contains(r#""x-y-z""#), "aggregate method not folded: {out}");
    // Nested const-object member chain.
    assert_eq!(deob("var o = { a: { b: 7 } }; sink(o.a.b);").trim(), "sink(7);");
}

#[test]
fn mutated_or_escaped_aggregate_receiver_is_kept() {
    // `.push` mutates the array, so its value is not its literal — must not fold.
    let out = deob("var a = [1,2]; a.push(3); sink(a.join(','));");
    assert!(out.contains("a.join") || out.contains("push"), "mutated array wrongly folded: {out}");
}

#[test]
fn const_template_literal_folds_to_string() {
    assert_eq!(deob("sink(`a${1 + 1}b`);").trim(), r#"sink("a2b");"#);
    // Tagged templates are not folded (the tag observes the raw parts).
    let out = deob("sink(String.raw`a${1}b`);");
    assert!(out.contains("String.raw"), "tagged template wrongly folded: {out}");
}

#[test]
fn pure_iife_returning_array_materializes_and_cleans_up() {
    // A pure IIFE that returns a small array splices to the literal, so the dead
    // scaffolding binding is then removed by DCE.
    let out = deob("var t = (function(){ return ['a','b','c']; })(); sink(t.join('-'));");
    assert!(out.contains(r#""a-b-c""#), "array IIFE not materialized/folded: {out}");
    assert!(!out.contains("function"), "scaffolding IIFE survived: {out}");
}

#[test]
fn split_to_array_folds() {
    assert_eq!(deob("sink('a,b,c'.split(',').length);").trim(), "sink(3);");
}

#[test]
fn pure_coercion_globals_fold() {
    assert_eq!(deob("sink(Boolean(1) && Number('5') + 2);").trim(), "sink(7);");
    assert_eq!(deob("sink(String(42) + Number('0x10'));").trim(), r#"sink("4216");"#);
}

#[test]
fn shadowed_coercion_global_is_kept() {
    // A local binding named `Number` means a bare `Number(...)` may be the shadow,
    // so it must not be folded as the global.
    let out = deob("function g(){ var Number = function(x){ return ext(x); }; return Number(5); } sink(g());");
    assert!(out.contains("Number(5)") || out.contains("ext("), "shadowed coercion wrongly folded: {out}");
}
