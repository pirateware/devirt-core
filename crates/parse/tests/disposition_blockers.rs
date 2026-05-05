//! Unit tests for the collect-all blocker walker in `disposition.rs`.

use std::collections::BTreeSet;

use ir::blocker::BlockerKind;
use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::SourceType;
use parse::disposition::{decide, Disposition};

/// Parse `src`, find the first top-level function declaration, and run
/// `decide` on it. Panics if no function declaration is found or parse fails.
fn decide_first_fn(src: &str) -> Disposition {
    let alloc = Allocator::default();
    let ret = Parser::new(&alloc, src, SourceType::default()).parse();
    assert!(ret.errors.is_empty(), "parse errors: {:?}", ret.errors);
    for stmt in &ret.program.body {
        if let js::Statement::FunctionDeclaration(f) = stmt {
            let body = f.body.as_ref().expect("function body");
            return decide(&f.params, body);
        }
    }
    panic!("no function declaration found in: {src}");
}

fn blockers_of(src: &str) -> BTreeSet<BlockerKind> {
    match decide_first_fn(src) {
        Disposition::PassThrough(b) => b,
        Disposition::Lowered => panic!("expected PassThrough, got Lowered for:\n{src}"),
    }
}

// --- Zero blockers → Lowered ---

#[test]
fn empty_body_is_lowered() {
    assert_eq!(decide_first_fn("function f() {}"), Disposition::Lowered);
}

#[test]
fn arithmetic_and_return_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(x, y) { let z = x + y; return z; }"),
        Disposition::Lowered
    );
}

// --- Three distinct blockers all reported ---

#[test]
fn three_distinct_blockers_all_reported() {
    // ThrowStmt + SequenceExpr (comma in parens) + NullishCoalescing
    let src = "function f(a) { throw new Error(); (a, a); let x = a ?? 1; }";
    let b = blockers_of(src);
    assert!(b.contains(&BlockerKind::ThrowStmt), "missing ThrowStmt in {b:?}");
    assert!(b.contains(&BlockerKind::SequenceExpr), "missing SequenceExpr in {b:?}");
    assert!(b.contains(&BlockerKind::NullishCoalescing), "missing NullishCoalescing in {b:?}");
    assert_eq!(b.len(), 3, "expected exactly 3 blockers, got {b:?}");
}

// --- Set semantics: same kind 5 times = 1 entry ---

#[test]
fn repeated_blocker_kind_reported_once() {
    // Five throw statements — all ThrowStmt, only one entry in the set.
    let src = "function f() {
        throw 1; throw 2; throw 3; throw 4; throw 5;
    }";
    let b = blockers_of(src);
    assert!(b.contains(&BlockerKind::ThrowStmt), "{b:?}");
    assert_eq!(b.len(), 1, "expected exactly 1 unique blocker, got {b:?}");
}

// --- Individual blocker kinds ---

#[test]
fn rest_param_blocked() {
    let b = blockers_of("function f(...args) {}");
    assert!(b.contains(&BlockerKind::RestParam), "{b:?}");
}

#[test]
fn destructuring_param_blocked() {
    let b = blockers_of("function f({a, b}) { return a + b; }");
    assert!(b.contains(&BlockerKind::DestructuringParam), "{b:?}");
}

#[test]
fn default_param_blocked() {
    let b = blockers_of("function f(x = 1) { return x; }");
    assert!(b.contains(&BlockerKind::DefaultParam), "{b:?}");
}

#[test]
fn throw_stmt_blocked() {
    let b = blockers_of("function f() { throw new Error('x'); }");
    assert!(b.contains(&BlockerKind::ThrowStmt), "{b:?}");
}

#[test]
fn break_in_while_blocked() {
    let b = blockers_of("function f() { while (true) { break; } }");
    assert!(b.contains(&BlockerKind::BreakStmt), "{b:?}");
}

#[test]
fn switch_stmt_blocked() {
    let b = blockers_of("function f(x) { switch (x) { case 1: return 1; } }");
    assert!(b.contains(&BlockerKind::SwitchStmt), "{b:?}");
}

#[test]
fn sequence_expr_blocked() {
    let b = blockers_of("function f(a, b) { (a, b); }");
    assert!(b.contains(&BlockerKind::SequenceExpr), "{b:?}");
}

#[test]
fn nullish_coalescing_blocked() {
    let b = blockers_of("function f(x) { return x ?? 0; }");
    assert!(b.contains(&BlockerKind::NullishCoalescing), "{b:?}");
}

#[test]
fn this_expr_blocked() {
    let b = blockers_of("function f() { return this.x; }");
    assert!(b.contains(&BlockerKind::ThisExpr), "{b:?}");
}

#[test]
fn regex_literal_blocked() {
    let b = blockers_of("function f() { return /abc/g; }");
    assert!(b.contains(&BlockerKind::RegExpLiteral), "{b:?}");
}

#[test]
fn generator_function_decl_inside_function_blocked() {
    let b = blockers_of("function f() { function* g() {} }");
    assert!(b.contains(&BlockerKind::GeneratorFunction), "{b:?}");
}

#[test]
fn async_function_expr_inside_function_blocked() {
    let b = blockers_of("function f() { let g = async function() {}; }");
    assert!(b.contains(&BlockerKind::AsyncFunction), "{b:?}");
}
