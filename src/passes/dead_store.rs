//! Dead-store elimination.
//!
//! After decoder/invariant lifting, the arithmetic index chains that fed the
//! decoder (`a = 0, b = a + a, … , idx = …`) become *read-never*: nothing
//! observes their values anymore. They're assignments inside comma sequences,
//! so ordinary DCE (which works on declarations) leaves them — sample_4 keeps a
//! single 23 KB line of them. Here we delete `x = <pure expr>` whenever `x` has
//! zero read references. Pure RHS + never read ⇒ removing the store changes
//! nothing. Chains peel leaf-first across fixpoint rounds (removing the last use
//! of `b` makes `a` read-never next round).
//!
//! Guards: only non-root (non-global) symbols, and only side-effect-free RHS.

use std::collections::HashSet;

use oxc_allocator::{Allocator, TakeIn};
use oxc_ast::ast::{ArrayExpressionElement, AssignmentTarget, Expression, Program, Statement};
use oxc_semantic::Scoping;
use oxc_syntax::operator::{AssignmentOperator, UnaryOperator};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct DeadStore {
    changed: bool,
}

impl Pass for DeadStore {
    fn name(&self) -> &'static str {
        "dead-store"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);
        let root = scoping.root_scope_id();
        let dead: HashSet<SymbolId> = scoping
            .symbol_ids()
            .filter(|s| {
                scoping.symbol_scope_id(*s) != root
                    && !scoping.get_resolved_references(*s).any(|r| r.is_read())
            })
            .collect();
        if dead.is_empty() {
            return false;
        }

        let mut elim = Eliminator { dead: &dead, changed: false, _marker: std::marker::PhantomData };
        traverse_mut(&mut elim, allocator, program, scoping, ());
        self.changed = elim.changed;
        self.changed
    }
}

struct Eliminator<'a, 'r> {
    dead: &'r HashSet<SymbolId>,
    changed: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'r> Traverse<'a, ()> for Eliminator<'a, 'r> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Expression::SequenceExpression(seq) = expr else {
            return;
        };
        let n = seq.expressions.len();
        if n < 2 {
            return;
        }
        // Drop pure dead stores, but always keep the final element (the
        // sequence's value may be observed).
        let mut idx = 0usize;
        let before = seq.expressions.len();
        let dead = self.dead;
        let scoping: &Scoping = ctx.scoping();
        seq.expressions.retain(|e| {
            let last = idx == n - 1;
            idx += 1;
            last || !is_dead_pure_store(e, dead, scoping)
        });
        if seq.expressions.len() != before {
            self.changed = true;
        }
        // Collapse a single-element sequence to that element.
        if seq.expressions.len() == 1 {
            let only = seq.expressions[0].take_in(ctx.ast);
            *expr = only;
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Statement::ExpressionStatement(es) = stmt else {
            return;
        };
        if is_dead_pure_store(&es.expression, self.dead, ctx.scoping()) {
            *stmt = ctx.ast.statement_empty(es.span);
            self.changed = true;
        }
    }
}

fn is_dead_pure_store(e: &Expression<'_>, dead: &HashSet<SymbolId>, scoping: &Scoping) -> bool {
    let Expression::AssignmentExpression(a) = e else {
        return false;
    };
    if !matches!(a.operator, AssignmentOperator::Assign) {
        return false;
    }
    let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
        return false;
    };
    let Some(rid) = id.reference_id.get() else {
        return false;
    };
    let Some(sym) = scoping.get_reference(rid).symbol_id() else {
        return false;
    };
    dead.contains(&sym) && is_pure_expr(&a.right)
}

/// Side-effect-free expression: removing it observes nothing. Excludes calls,
/// member access (possible getters), object/new/update/assignment.
fn is_pure_expr(e: &Expression<'_>) -> bool {
    match e {
        Expression::Identifier(_)
        | Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_) => true,
        Expression::UnaryExpression(u) => {
            !matches!(u.operator, UnaryOperator::Delete) && is_pure_expr(&u.argument)
        }
        Expression::BinaryExpression(b) => is_pure_expr(&b.left) && is_pure_expr(&b.right),
        Expression::LogicalExpression(l) => is_pure_expr(&l.left) && is_pure_expr(&l.right),
        Expression::ConditionalExpression(c) => {
            is_pure_expr(&c.test) && is_pure_expr(&c.consequent) && is_pure_expr(&c.alternate)
        }
        Expression::SequenceExpression(s) => s.expressions.iter().all(is_pure_expr),
        Expression::ArrayExpression(a) => a.elements.iter().all(|el| match el {
            ArrayExpressionElement::Elision(_) => true,
            ArrayExpressionElement::SpreadElement(_) => false,
            other => other.as_expression().is_some_and(is_pure_expr),
        }),
        _ => false,
    }
}
