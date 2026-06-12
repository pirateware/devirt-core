//! Shared helpers for passes.

use oxc_ast::ast::{ArrayExpressionElement, Expression, ObjectPropertyKind, Program, PropertyKind};
use oxc_semantic::{Scoping, SemanticBuilder};

/// Run `f` on a dedicated worker thread with an explicit `stack` size, reporting
/// whether it panicked.
///
/// Boa's recursive parser/VM and oxc's traversal lean hard on the native stack,
/// and a stack overflow *aborts* (it cannot be caught), so the depth ceiling is
/// made large and independent of the caller's stack. A panic inside `f` is
/// contained — `join()` returns `Err(payload)` — so callers can degrade
/// gracefully instead of crashing the host.
pub fn run_isolated<R: Send>(
    name: &str,
    stack: usize,
    f: impl FnOnce() -> R + Send,
) -> std::thread::Result<R> {
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .name(name.to_string())
            .stack_size(stack)
            .spawn_scoped(scope, f)
            .expect("spawn isolated worker thread")
            .join()
    })
}

/// Build `Scoping` for the current tree. Side effect: `SemanticBuilder` writes
/// `symbol_id`/`reference_id` back into the AST's interior-mutable cells, so
/// after this call binding identifiers and identifier references are annotated
/// and can be resolved during traversal.
pub fn build_scoping(program: &Program<'_>) -> Scoping {
    SemanticBuilder::new()
        .build(program)
        .semantic
        .into_scoping()
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
/// unused. Conservative: literals, bare identifier reads, function values
/// (defining a function the program never references has no effect), and
/// object/array literals built entirely from such pure parts (a spent
/// operator-proxy table or decoder array left dead after inlining/lifting).
pub fn is_pure_simple(expr: &Expression<'_>) -> bool {
    if is_inlineable_literal(expr) {
        return true;
    }
    match expr {
        Expression::Identifier(_)
        | Expression::FunctionExpression(_)
        | Expression::ArrowFunctionExpression(_) => true,
        Expression::ObjectExpression(o) => o.properties.iter().all(pure_object_property),
        Expression::ArrayExpression(a) => a.elements.iter().all(pure_array_element),
        _ => false,
    }
}

/// A property whose mere construction has no side effect: a non-computed
/// data/accessor property with a pure value. Computed keys (could evaluate a
/// side-effecting expression) and spreads (read another object, possibly a
/// getter) are rejected.
fn pure_object_property(prop: &ObjectPropertyKind<'_>) -> bool {
    let ObjectPropertyKind::ObjectProperty(p) = prop else {
        return false; // SpreadProperty
    };
    if p.computed {
        return false;
    }
    // Getter/setter definitions are themselves side-effect-free to install; a
    // data property is pure iff its value is. In both cases the value is a
    // function or a pure expression, so the same predicate applies.
    matches!(
        p.kind,
        PropertyKind::Init | PropertyKind::Get | PropertyKind::Set
    ) && is_pure_simple(&p.value)
}

fn pure_array_element(elem: &ArrayExpressionElement<'_>) -> bool {
    match elem {
        ArrayExpressionElement::SpreadElement(_) => false,
        ArrayExpressionElement::Elision(_) => true,
        other => other.as_expression().is_some_and(is_pure_simple),
    }
}
