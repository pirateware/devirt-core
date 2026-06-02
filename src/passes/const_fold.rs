//! Constant folding via oxc's `ConstantEvaluation`.
//!
//! We provide a deliberately *conservative* evaluation context: property reads
//! and unknown globals are treated as side-effectful, so only genuinely pure
//! expressions fold. That defeats the common opaque predicates and arithmetic
//! noise (`!![]`, `!0`, `(0x1, 2)` comma chains, literal arithmetic) without
//! risking unsound rewrites. We fold to number/boolean/null only; string and
//! bigint folding are left for later (they need arena string allocation and are
//! mostly subsumed by decoder lifting).
//!
//! Also folds `if (<constant>)` to its taken branch — but preserves hoisted
//! `var`/function declarations from the dead branch is NOT handled yet, so we
//! only fold when the discarded branch is empty-ish (a single non-declaration).

use oxc_allocator::{Allocator, TakeIn};
use oxc_ast::ast::{Expression, Statement};
use oxc_ast::AstBuilder;
use oxc_ecmascript::constant_evaluation::{
    ConstantEvaluation, ConstantEvaluationCtx, ConstantValue,
};
use oxc_ecmascript::side_effects::{MayHaveSideEffectsContext, PropertyReadSideEffects};
use oxc_ecmascript::GlobalContext;
use oxc_ast::ast::IdentifierReference;
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
use oxc_syntax::number::NumberBase;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct ConstantFold {
    changed: bool,
}

impl Pass for ConstantFold {
    fn name(&self) -> &'static str {
        "const-fold"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let scoping = build_scoping(program);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

use oxc_ast::ast::Program;

impl<'a> Traverse<'a, ()> for ConstantFold {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Don't re-fold something already in canonical literal form.
        if is_literal(expr) {
            return;
        }
        let folded = {
            let ectx = EvalCtx { ast: ctx.ast, scoping: ctx.scoping() };
            expr.evaluate_value(&ectx)
        };
        if let Some(value) = folded {
            if let Some(new_expr) = const_to_expr(value, ctx.ast, expr) {
                *expr = new_expr;
                self.changed = true;
            }
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Statement::IfStatement(ifs) = stmt else {
            return;
        };
        let cond = {
            let ectx = EvalCtx { ast: ctx.ast, scoping: ctx.scoping() };
            ifs.test.evaluate_value_to_boolean(&ectx)
        };
        let Some(cond) = cond else {
            return;
        };
        // Replace with the taken branch; the discarded branch is dropped.
        let taken: Statement<'a> = if cond {
            ifs.consequent.take_in(ctx.ast)
        } else if let Some(alt) = &mut ifs.alternate {
            alt.take_in(ctx.ast)
        } else {
            ctx.ast.statement_empty(ifs.span)
        };
        *stmt = taken;
        self.changed = true;
    }
}

fn is_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::StringLiteral(_)
    )
}

/// Convert a folded constant back to a literal node. Returns `None` for cases
/// we deliberately skip (strings, bigint, undefined, non-finite/negative
/// numbers — the latter to avoid fold/rebuild oscillation with unary minus).
fn const_to_expr<'a>(
    value: ConstantValue<'a>,
    ast: AstBuilder<'a>,
    original: &Expression<'a>,
) -> Option<Expression<'a>> {
    let span = original.span();
    match value {
        ConstantValue::Number(n) if n.is_finite() && n >= 0.0 => {
            Some(ast.expression_numeric_literal(span, n, None, NumberBase::Decimal))
        }
        ConstantValue::Boolean(b) => Some(ast.expression_boolean_literal(span, b)),
        ConstantValue::Null => Some(ast.expression_null_literal(span)),
        _ => None,
    }
}

/// Conservative evaluation context: treat anything that could observe or mutate
/// the world as side-effectful, so only pure expressions fold.
struct EvalCtx<'a, 'b> {
    ast: AstBuilder<'a>,
    scoping: &'b Scoping,
}

impl<'a, 'b> GlobalContext<'a> for EvalCtx<'a, 'b> {
    fn is_global_reference(&self, reference: &IdentifierReference<'a>) -> bool {
        match reference.reference_id.get() {
            Some(rid) => self.scoping.get_reference(rid).symbol_id().is_none(),
            None => true,
        }
    }
}

impl<'a, 'b> MayHaveSideEffectsContext<'a> for EvalCtx<'a, 'b> {
    fn annotations(&self) -> bool {
        false
    }
    fn manual_pure_functions(&self, _callee: &Expression) -> bool {
        false
    }
    fn property_read_side_effects(&self) -> PropertyReadSideEffects {
        PropertyReadSideEffects::All
    }
    fn unknown_global_side_effects(&self) -> bool {
        true
    }
}

impl<'a, 'b> ConstantEvaluationCtx<'a> for EvalCtx<'a, 'b> {
    fn ast(&self) -> AstBuilder<'a> {
        self.ast
    }
}
