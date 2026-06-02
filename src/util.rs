//! Shared helpers for passes.

use oxc_ast::ast::{Expression, Program};
use oxc_semantic::{Scoping, SemanticBuilder};

/// Build `Scoping` for the current tree. Side effect: `SemanticBuilder` writes
/// `symbol_id`/`reference_id` back into the AST's interior-mutable cells, so
/// after this call binding identifiers and identifier references are annotated
/// and can be resolved during traversal.
pub fn build_scoping(program: &Program<'_>) -> Scoping {
    SemanticBuilder::new().build(program).semantic.into_scoping()
}

/// A literal that is pure and freely duplicable, so inlining it at any number of
/// read sites is always semantics-preserving.
pub fn is_inlineable_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
    )
}

/// An initializer with no side effects — safe to drop when its binding is
/// unused. Conservative: literals, bare identifier reads, and function values
/// (defining a function the program never references has no effect).
pub fn is_pure_simple(expr: &Expression<'_>) -> bool {
    is_inlineable_literal(expr)
        || matches!(
            expr,
            Expression::Identifier(_)
                | Expression::FunctionExpression(_)
                | Expression::ArrowFunctionExpression(_)
        )
}
