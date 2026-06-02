//! Split statement-level comma (sequence) expressions into separate statements.
//!
//! Obfuscators (notably Cloudflare's challenge) collapse a whole prologue into
//! one giant comma chain:
//!
//! ```js
//! a = f, (function(){...})(), b = this, c = b.document, d = 30, ...;
//! ```
//!
//! Each comma operand is evaluated left-to-right for its side effects and only
//! the last value is the expression's result. At *statement* position the
//! result is discarded, so `(e1, e2, ..., en);` is exactly `e1; e2; ...; en;` —
//! a purely syntactic, always-sound expansion that restores one-effect-per-line.
//!
//! We also split the operand list of `return`/`throw`: `return e1, e2, en;`
//! becomes `e1; e2; return en;` (the thrown/returned value is the last operand,
//! the rest are leading side effects). All three live in a `Vec<Statement>`, so
//! a single `enter_statements` hook covers them.

use oxc_allocator::{Allocator, TakeIn, Vec as ArenaVec};
use oxc_ast::ast::{Expression, Program, Statement};
use oxc_span::GetSpan;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct SequenceSplit {
    changed: bool,
}

impl Pass for SequenceSplit {
    fn name(&self) -> &'static str {
        "sequence-split"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let scoping = build_scoping(program);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

/// Does this statement contain a sequence we would expand? Cheap pre-check so we
/// only rebuild a statement list when there is actually work to do.
fn splittable(stmt: &Statement) -> bool {
    match stmt {
        Statement::ExpressionStatement(es) => {
            matches!(es.expression, Expression::SequenceExpression(_))
        }
        Statement::ReturnStatement(rs) => {
            matches!(rs.argument, Some(Expression::SequenceExpression(_)))
        }
        Statement::ThrowStatement(ts) => {
            matches!(ts.argument, Expression::SequenceExpression(_))
        }
        _ => false,
    }
}

/// Append `expr` (flattened if it is itself a sequence) as one expression
/// statement per operand into `out`.
fn push_as_statements<'a>(
    expr: Expression<'a>,
    out: &mut ArenaVec<'a, Statement<'a>>,
    ctx: &mut TraverseCtx<'a, ()>,
) {
    match expr {
        Expression::SequenceExpression(mut seq) => {
            let exprs = std::mem::replace(&mut seq.expressions, ctx.ast.vec());
            for e in exprs {
                push_as_statements(e, out, ctx);
            }
        }
        other => {
            let span = other.span();
            out.push(ctx.ast.statement_expression(span, other));
        }
    }
}

/// Move the operands of a sequence into `out` as leading statements and return
/// the final operand (the value position). For a non-sequence, returns it
/// unchanged with nothing pushed.
fn split_off_last<'a>(
    expr: Expression<'a>,
    out: &mut ArenaVec<'a, Statement<'a>>,
    ctx: &mut TraverseCtx<'a, ()>,
) -> Expression<'a> {
    let Expression::SequenceExpression(mut seq) = expr else {
        return expr;
    };
    let exprs = std::mem::replace(&mut seq.expressions, ctx.ast.vec());
    let mut iter = exprs.into_iter().peekable();
    let mut last = ctx.ast.expression_null_literal(seq.span);
    while let Some(e) = iter.next() {
        if iter.peek().is_none() {
            last = e; // value position — keep flattening it in the caller
        } else {
            push_as_statements(e, out, ctx);
        }
    }
    last
}

impl<'a> Traverse<'a, ()> for SequenceSplit {
    fn enter_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        if !stmts.iter().any(splittable) {
            return;
        }

        let old = std::mem::replace(stmts, ctx.ast.vec_with_capacity(stmts.len()));
        for stmt in old {
            match stmt {
                Statement::ExpressionStatement(mut es) => {
                    let expr = es.expression.take_in(ctx.ast);
                    push_as_statements(expr, stmts, ctx);
                }
                Statement::ReturnStatement(mut rs) => match rs.argument.take() {
                    Some(arg) => {
                        let span = rs.span;
                        let last = split_off_last(arg, stmts, ctx);
                        stmts.push(ctx.ast.statement_return(span, Some(last)));
                    }
                    None => stmts.push(Statement::ReturnStatement(rs)),
                },
                Statement::ThrowStatement(mut ts) => {
                    let span = ts.span;
                    let arg = ts.argument.take_in(ctx.ast);
                    let last = split_off_last(arg, stmts, ctx);
                    stmts.push(ctx.ast.statement_throw(span, last));
                }
                other => stmts.push(other),
            }
        }
        self.changed = true;
    }
}
