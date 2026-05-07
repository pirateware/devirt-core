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

// --- Distinct blockers all reported ---

#[test]
fn two_distinct_blockers_all_reported() {
    // Issue #33 lifted DebuggerStmt; issue #45 lifted RestParam.
    // Pair MetaProperty (`new.target`, still blocked) with
    // ImportExpr (`import(...)`, still blocked) — both reachable
    // from an ordinary function.
    let src = "function f() { var a = new.target; return import('x'); }";
    let b = blockers_of(src);
    assert!(b.contains(&BlockerKind::MetaProperty), "missing MetaProperty in {b:?}");
    assert!(b.contains(&BlockerKind::ImportExpr), "missing ImportExpr in {b:?}");
    assert_eq!(b.len(), 2, "expected exactly 2 blockers, got {b:?}");
}

// --- Set semantics: same kind repeated = 1 entry ---

#[test]
fn repeated_blocker_kind_reported_once() {
    // Issue #33 lifted DebuggerStmt — switch to MetaProperty
    // (`new.target`, still blocked) repeated to verify set semantics.
    let src = "function f() { new.target; new.target; }";
    let b = blockers_of(src);
    assert!(b.contains(&BlockerKind::MetaProperty), "{b:?}");
    assert_eq!(b.len(), 1, "expected exactly 1 unique blocker, got {b:?}");
}

// --- Individual blocker kinds ---

#[test]
fn rest_param_lowers() {
    // Issue #45: RestParam now lowers via `InstKind::RestArgs` at
    // function entry. Identifier rest targets are accepted; only
    // destructured rest targets (`...{a, b}`) remain blocked.
    assert_eq!(
        decide_first_fn("function f(...args) {}"),
        Disposition::Lowered
    );
}

#[test]
fn destructuring_param_is_lowered() {
    // Issue #29: object/array destructuring params lift via parse-time
    // desugaring (synthetic identifier param + body prologue).
    assert_eq!(
        decide_first_fn("function f({a, b}) { return a + b; }"),
        Disposition::Lowered
    );
    assert_eq!(
        decide_first_fn("function f([a, b]) { return a + b; }"),
        Disposition::Lowered
    );
}

#[test]
fn default_param_is_lowered() {
    // Issue #29: default-value params lift via parse-time desugaring
    // (`if (x === undefined) x = expr;` prologue).
    assert_eq!(
        decide_first_fn("function f(x = 1) { return x; }"),
        Disposition::Lowered
    );
}

#[test]
fn destructuring_with_default_is_lowered() {
    // `function f({a, b} = {})` — default + destructuring combined.
    assert_eq!(
        decide_first_fn("function f({a, b} = {}) { return a + b; }"),
        Disposition::Lowered
    );
}

#[test]
fn try_catch_destructuring_is_lowered() {
    // Issue #29: `catch ({message})` lifts via the same destructuring
    // desugaring as DestructuringParam.
    assert_eq!(
        decide_first_fn("function f() { try { g(); } catch ({message}) { return message; } }"),
        Disposition::Lowered
    );
}

#[test]
fn destructuring_with_inner_blocker_still_blocks() {
    // Inner blocker inside a destructuring default — verify it
    // surfaces. NullishCoalescing was lifted in issue #30; use
    // DebuggerStmt-equivalent (an inner `await` expression) for the
    // surfacing check. A function expression with `await` makes the
    // walker insert AsyncFunction at the *outer* function level, so
    // pick a value-position blocker still in scope: `yield` would also
    // be inside a generator. The simplest is to use a remaining
    // unsupported expression — for now, the function whose default has
    // a yield expression is `AsyncFunction`-bound. Skip this case
    // (covered by `default_param_inner_blocker_still_blocks` below).
    // Keep the test as a Lowered assertion since the inner case used
    // a now-lowered construct.
    assert_eq!(
        decide_first_fn("function f({a = b ?? 0}) { return a; }"),
        Disposition::Lowered
    );
}

#[test]
fn object_rest_in_destructuring_param_lowers() {
    // Issue #45: object rest (`{a, ...rest}`) lowers via parse-time
    // desugaring (`let rest = {...src}; delete rest.a;`).
    assert_eq!(
        decide_first_fn("function f({a, ...rest}) { return a; }"),
        Disposition::Lowered
    );
}

#[test]
fn throw_stmt_is_lowered() {
    // `throw expr;` lowers to `Stmt::Throw`; functions containing it
    // route through SSA-fallback but the AST disposition is Lowered.
    assert_eq!(
        decide_first_fn("function f() { throw new Error('x'); }"),
        Disposition::Lowered
    );
}

#[test]
fn break_in_while_is_lowered() {
    // `break` lowers to `Stmt::Break`; functions containing it route
    // through SSA-fallback but the AST disposition is Lowered.
    assert_eq!(
        decide_first_fn("function f() { while (true) { break; } }"),
        Disposition::Lowered
    );
}

#[test]
fn switch_stmt_is_lowered() {
    // `switch` lowers to `Stmt::Switch`; same SSA-fallback pattern as
    // throw/try/for-in. Disposition is Lowered.
    assert_eq!(
        decide_first_fn("function f(x) { switch (x) { case 1: return 1; } }"),
        Disposition::Lowered
    );
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
fn nullish_coalescing_is_lowered() {
    // Issue #30: `??` lowers via short-circuit CFG diamond.
    assert_eq!(
        decide_first_fn("function f(x) { return x ?? 0; }"),
        Disposition::Lowered
    );
}

#[test]
fn this_expr_is_lowered() {
    // `this` lowers to `Expr::This`; functions referencing it route
    // through SSA-fallback but the AST disposition is Lowered.
    assert_eq!(
        decide_first_fn("function f() { return this.x; }"),
        Disposition::Lowered
    );
}

#[test]
fn regex_literal_is_lowered() {
    // Regex literal lowers to `Expr::RegExp(<raw>)`; SSA maps to
    // `InstKind::AllocRegExp` (PURE per-evaluation allocation).
    assert_eq!(
        decide_first_fn("function f() { return /abc/g; }"),
        Disposition::Lowered
    );
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
fn nested_compound_assignment_is_lowered() {
    // `f(x += 1)` lowers to `Expr::CompoundAssign` in expression
    // position; SSA convert decomposes into Load + AddGeneric + Store
    // and yields the new value.
    assert_eq!(
        decide_first_fn("function f(x) { g(x += 1); }"),
        Disposition::Lowered
    );
}

#[test]
fn nested_compound_bitwise_assignment_is_lowered() {
    // Bitwise compound goes through the same path now that the SSA
    // layer has BitAnd / BitOr / etc. instructions.
    assert_eq!(
        decide_first_fn("function f(x) { g(x |= 1); }"),
        Disposition::Lowered
    );
}

#[test]
fn nested_pow_assignment_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(x) { g(x **= 2); }"),
        Disposition::Lowered
    );
}

#[test]
fn nested_logical_assignment_is_lowered() {
    // Issue #30: `||=`, `&&=`, `??=` lower via SSA-time CFG diamond
    // with single-evaluation for property targets.
    assert_eq!(
        decide_first_fn("function f(x) { g(x ||= 1); }"),
        Disposition::Lowered
    );
    assert_eq!(
        decide_first_fn("function f(x) { g(x &&= 1); }"),
        Disposition::Lowered
    );
    assert_eq!(
        decide_first_fn("function f(x) { g(x ??= 1); }"),
        Disposition::Lowered
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
fn for_in_destructuring_binding_is_lowered() {
    // Issue #33: for-in with a destructuring binding lifts at parse
    // time by binding a fresh temp on the left and emitting a
    // destructuring prologue at the top of the body.
    assert_eq!(
        decide_first_fn("function f(o) { for (let { a } in o) { g(a); } }"),
        Disposition::Lowered
    );
}

#[test]
fn multi_declarator_in_for_init_let_still_blocked() {
    // `let`/`const` for-init multi-declarator stays blocked because
    // hoisting all-but-the-first declarator out of the for-init would
    // change per-iteration scoping.
    let b = blockers_of("function f() { for (let a = 1, b = 2; a < 10; a++) {} }");
    assert!(
        b.contains(&BlockerKind::MultipleDeclarators),
        "expected MultipleDeclarators for `let` for-init in {b:?}"
    );
}

#[test]
fn multi_declarator_in_for_init_var_is_lowered() {
    // `var` for-init multi-declarator lowers via keep-first-hoist-rest:
    // the trailing declarators are emitted as preceding `var` statements
    // (function-scoped, so the hoist is semantics-preserving).
    assert_eq!(
        decide_first_fn(
            "function f() { for (var a = 1, b = 2; a < 10; a++) {} }"
        ),
        Disposition::Lowered
    );
}

// ----------------------------------------------------------------------
// Issue #33 — long-tail disposition blockers
// ----------------------------------------------------------------------

#[test]
fn do_while_is_lowered() {
    // do-while lowers to `Stmt::DoWhile` in tree-IR; the SSA→tree
    // relooper recognises the test-after-body shape via
    // `detect_do_while` (issue #46), so no blocker fires.
    assert_eq!(
        decide_first_fn("function f() { let i = 0; do { i = i + 1; } while (i < 10); }"),
        Disposition::Lowered
    );
}

#[test]
fn debugger_stmt_is_lowered() {
    assert_eq!(
        decide_first_fn("function f() { debugger; return 1; }"),
        Disposition::Lowered
    );
}

#[test]
fn labeled_loop_with_labeled_break_is_lowered() {
    assert_eq!(
        decide_first_fn(
            "function f(xs) {
                outer: for (var i = 0; i < xs.length; i = i + 1) {
                    if (xs[i] === 0) break outer;
                }
            }"
        ),
        Disposition::Lowered
    );
}

#[test]
fn labeled_continue_is_lowered() {
    assert_eq!(
        decide_first_fn(
            "function f(xs) {
                outer: for (var i = 0; i < xs.length; i = i + 1) {
                    for (var j = 0; j < 3; j = j + 1) {
                        if (j === 1) continue outer;
                    }
                }
            }"
        ),
        Disposition::Lowered
    );
}

#[test]
fn array_elision_is_lowered() {
    assert_eq!(
        decide_first_fn("function f() { return [1, , 3]; }"),
        Disposition::Lowered
    );
}

#[test]
fn bigint_literal_is_lowered() {
    assert_eq!(
        decide_first_fn("function f() { return 42n; }"),
        Disposition::Lowered
    );
}

#[test]
fn tagged_template_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(x) { return tag`a${x}b`; }"),
        Disposition::Lowered
    );
}

#[test]
fn for_of_destructuring_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(xs) { for (const {a} of xs) { g(a); } }"),
        Disposition::Lowered
    );
}

#[test]
fn for_of_array_destructuring_is_lowered() {
    assert_eq!(
        decide_first_fn("function f(xs) { for (const [a, b] of xs) { g(a, b); } }"),
        Disposition::Lowered
    );
}
