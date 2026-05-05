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
    // ThrowStmt + BreakStmt + NullishCoalescing — three constructs that
    // remain blocked after the SequenceExpr / AssignmentExpr / ForInStmt
    // ticket (those are no longer blockers in their simple forms).
    let src = "function f(a) { throw new Error(); while (a) { break; } let x = a ?? 1; }";
    let b = blockers_of(src);
    assert!(b.contains(&BlockerKind::ThrowStmt), "missing ThrowStmt in {b:?}");
    assert!(b.contains(&BlockerKind::BreakStmt), "missing BreakStmt in {b:?}");
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
fn sequence_expr_is_lowered() {
    // Sequence expressions are now lowered as `Expr::Sequence`; no blocker.
    assert_eq!(
        decide_first_fn("function f(a, b) { (a, b); }"),
        Disposition::Lowered
    );
}

#[test]
fn sequence_expr_in_call_callee_is_lowered() {
    // The bundler-interop `(0, foo.bar)(args)` pattern must lower
    // (the `this`-binding correctness is verified by a behavioral test
    // in the corpus, not here).
    assert_eq!(
        decide_first_fn("function f(o) { return (0, o.m)(); }"),
        Disposition::Lowered
    );
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

// --- MultipleDeclarators: no longer a blocker for statement-level declarations ---

#[test]
fn multi_declarator_var_is_lowered() {
    assert_eq!(
        decide_first_fn("function f() { var a = 1, b = 2; return a + b; }"),
        Disposition::Lowered
    );
}

#[test]
fn multi_declarator_let_no_init_is_lowered() {
    assert_eq!(
        decide_first_fn("function f() { let x, y = 1; return y; }"),
        Disposition::Lowered
    );
}

#[test]
fn multi_declarator_const_is_lowered() {
    assert_eq!(
        decide_first_fn("function f() { const p = 1, q = 2; return p + q; }"),
        Disposition::Lowered
    );
}

#[test]
fn multi_declarator_destructuring_reports_destructuring_not_multiple() {
    // Each declarator with a destructuring pattern should report DestructuringBinding,
    // not MultipleDeclarators on the whole declaration.
    let b = blockers_of("function f(x, y) { let {a} = x, [b] = y; return a + b; }");
    assert!(
        b.contains(&BlockerKind::DestructuringBinding),
        "expected DestructuringBinding in {b:?}"
    );
    assert!(
        !b.contains(&BlockerKind::MultipleDeclarators),
        "MultipleDeclarators should not appear for destructuring split: {b:?}"
    );
}

#[test]
fn nested_assignment_expr_is_lowered() {
    // Plain `=` in expression position is now lowered as `Expr::Assign`.
    assert_eq!(
        decide_first_fn("function f(x) { g(x = 1); }"),
        Disposition::Lowered
    );
}

#[test]
fn nested_assignment_expr_in_if_test_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(a, b) { if (a = b) { return a; } }"),
        Disposition::Lowered
    );
}

#[test]
fn statement_level_assignment_is_lowered() {
    // Top-level `x = 1;` is a statement, not a nested expression — no blocker.
    assert_eq!(
        decide_first_fn("function f(x) { x = 1; return x; }"),
        Disposition::Lowered
    );
}

#[test]
fn nested_compound_assignment_blocked() {
    // `f(x += 1)` is a compound assignment in expression position.
    let b = blockers_of("function f(x) { g(x += 1); }");
    assert!(b.contains(&BlockerKind::CompoundAssignmentExpr), "{b:?}");
    assert!(
        !b.contains(&BlockerKind::AssignmentExpr),
        "compound should not also report plain AssignmentExpr: {b:?}"
    );
}

#[test]
fn nested_logical_assignment_reports_logical_op_only() {
    // `f(x ||= 1)` reports LogicalAssignOp (already its own variant) and
    // does NOT additionally report AssignmentExpr — the operator-class split
    // is exclusive so the table doesn't double-count.
    let b = blockers_of("function f(x) { g(x ||= 1); }");
    assert!(b.contains(&BlockerKind::LogicalAssignOp), "{b:?}");
    assert!(
        !b.contains(&BlockerKind::AssignmentExpr),
        "should not double-count: {b:?}"
    );
    assert!(
        !b.contains(&BlockerKind::CompoundAssignmentExpr),
        "should not double-count: {b:?}"
    );
}

#[test]
fn for_in_simple_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(o) { for (let k in o) { g(k); } }"),
        Disposition::Lowered
    );
}

#[test]
fn for_in_var_kind_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(o) { for (var k in o) { g(k); } }"),
        Disposition::Lowered
    );
}

#[test]
fn for_in_destructuring_binding_still_blocked() {
    // Disposition still rejects exotic for-in left sides.
    let b = blockers_of("function f(o) { for (let { a } in o) { g(a); } }");
    assert!(b.contains(&BlockerKind::ForInStmt), "{b:?}");
}

#[test]
fn multi_declarator_in_for_init_still_blocked() {
    // For-init multi-declarator cannot be split into multiple statements, so
    // it stays as a MultipleDeclarators blocker until the for-init ticket lands.
    let b = blockers_of("function f() { for (let a = 1, b = 2; a < 10; a++) {} }");
    assert!(
        b.contains(&BlockerKind::MultipleDeclarators),
        "expected MultipleDeclarators for for-init in {b:?}"
    );
}
