//! Per-function disposition decision: Lowered vs PassThrough.
//!
//! T2 (closures-and-pass-through) introduces the rule that every valid JS
//! function flows through the pipeline: bodies whose constructs are all
//! fully lowerable get optimized; bodies whose constructs include
//! anything we can't represent in our IR are preserved as
//! source-text (PassThrough).
//!
//! `decide` walks an OXC `FunctionBody` (plus its parameter list) and
//! returns the appropriate disposition. It stops descent at nested
//! function boundaries — those get their own decision when lowering
//! reaches them.
//!
//! The walker MUST track the lowering. If it disagrees, the asymmetric
//! debug cross-check in lower_ssa fires (catches "walker too permissive"
//! — i.e., walker said Lowered but lowering returned `Unsupported`,
//! which is the dangerous direction). Walker-too-conservative is
//! recoverable (a function we could optimize stays pass-through, no
//! correctness issue).
//!
//! The set of lowerable nodes is "as of this commit." Every expansion of
//! coverage is a separate ticket. Resist any temptation to add a node
//! kind here without a corresponding lowering implementation.

// Wired into the lowering by stage 4; suppress unused-symbol warnings
// until then.
#![allow(dead_code)]

use oxc_ast::ast as js;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    Lowered,
    PassThrough,
}

/// Decide a function's disposition. Walks `params` and `body`,
/// stopping at nested function boundaries.
pub fn decide(params: &js::FormalParameters, body: &js::FunctionBody) -> Disposition {
    if !params_lowerable(params) {
        return Disposition::PassThrough;
    }
    for stmt in &body.statements {
        if !stmt_lowerable(stmt) {
            return Disposition::PassThrough;
        }
    }
    Disposition::Lowered
}

/// Convenience: decide an arrow function. Arrows can have either an
/// expression body or a block body; the walker handles both via
/// `js::FunctionBody` for blocks and a single-expression check for
/// expression bodies.
pub fn decide_arrow(arrow: &js::ArrowFunctionExpression) -> Disposition {
    if !params_lowerable(&arrow.params) {
        return Disposition::PassThrough;
    }
    // Arrow body in OXC is always a `FunctionBody` whose statements
    // are either a real block's statements or a single
    // `ExpressionStatement` synthesized for `(x) => x+1`. Walk uniformly.
    for stmt in &arrow.body.statements {
        if !stmt_lowerable(stmt) {
            return Disposition::PassThrough;
        }
    }
    Disposition::Lowered
}

// ----------------------------------------------------------------------
// Parameters
// ----------------------------------------------------------------------

fn params_lowerable(params: &js::FormalParameters) -> bool {
    if params.rest.is_some() {
        return false;
    }
    for p in &params.items {
        // Plain identifier parameters only — anything else (defaults,
        // destructuring) blocks lowering.
        if !matches!(p.pattern.kind, js::BindingPatternKind::BindingIdentifier(_)) {
            return false;
        }
    }
    true
}

// ----------------------------------------------------------------------
// Statements
// ----------------------------------------------------------------------

fn stmt_lowerable(stmt: &js::Statement) -> bool {
    use js::Statement::*;
    match stmt {
        VariableDeclaration(decl) => var_decl_lowerable(decl),
        ExpressionStatement(es) => expr_stmt_lowerable(&es.expression),
        ReturnStatement(r) => match &r.argument {
            Some(e) => expr_lowerable(e),
            None => true,
        },
        IfStatement(s) => {
            expr_lowerable(&s.test)
                && stmt_lowerable(&s.consequent)
                && match &s.alternate {
                    Some(a) => stmt_lowerable(a),
                    None => true,
                }
        }
        WhileStatement(s) => expr_lowerable(&s.test) && stmt_lowerable(&s.body),
        BlockStatement(b) => b.body.iter().all(stmt_lowerable),
        FunctionDeclaration(f) => function_decl_lowerable(f),
        ForStatement(f) => for_stmt_lowerable(f),
        ContinueStatement(c) => c.label.is_none(),
        // Anything else: not currently lowered. (BreakStatement is
        // unsupported this round — see docs/DECISIONS.md.)
        _ => false,
    }
}

/// C-style `for (init; cond; step) body`. Each header part is
/// optional; we accept all three optional and require the body and any
/// present part to be lowerable. `for-in` / `for-of` are different AST
/// nodes and stay unsupported.
fn for_stmt_lowerable(f: &js::ForStatement) -> bool {
    if let Some(init) = &f.init {
        if !for_init_lowerable(init) {
            return false;
        }
    }
    if let Some(test) = &f.test
        && !expr_lowerable(test)
    {
        return false;
    }
    if let Some(update) = &f.update
        && !expr_lowerable(update)
    {
        return false;
    }
    stmt_lowerable(&f.body)
}

fn for_init_lowerable(init: &js::ForStatementInit) -> bool {
    match init {
        js::ForStatementInit::VariableDeclaration(decl) => var_decl_lowerable(decl),
        other => match other.as_expression() {
            Some(e) => expr_lowerable(e),
            None => false,
        },
    }
}

fn var_decl_lowerable(decl: &js::VariableDeclaration) -> bool {
    if decl.declarations.len() != 1 {
        return false;
    }
    let d = &decl.declarations[0];
    if !matches!(d.id.kind, js::BindingPatternKind::BindingIdentifier(_)) {
        return false;
    }
    match decl.kind {
        js::VariableDeclarationKind::Let | js::VariableDeclarationKind::Var => {
            // `var` is currently treated as `let` by the lowering. TDZ
            // and var-hoisting are out of scope per the T2 ticket. The
            // accepted simplification for now.
            match &d.init {
                Some(e) => expr_lowerable(e),
                None => true,
            }
        }
        js::VariableDeclarationKind::Const => match &d.init {
            Some(e) => expr_lowerable(e),
            None => false, // const without initializer is a syntax error
        },
        _ => false,
    }
}

fn expr_stmt_lowerable(expr: &js::Expression) -> bool {
    if let js::Expression::AssignmentExpression(a) = expr {
        // Plain `=` and compound (`+=`, `-=`, etc.) accepted; the
        // unsupported set is the logical-assignment family (`&&=`,
        // `||=`, `??=`) and any operator without a corresponding
        // BinOp. Compound assignment desugars at lower.rs time to a
        // sequence of let-tempos + binary + plain assign.
        if !assignment_op_lowerable(a.operator) {
            return false;
        }
        return assignment_target_lowerable(&a.left) && expr_lowerable(&a.right);
    }
    expr_lowerable(expr)
}

fn assignment_op_lowerable(op: js::AssignmentOperator) -> bool {
    use js::AssignmentOperator::*;
    matches!(
        op,
        Assign
            | Addition
            | Subtraction
            | Multiplication
            | Division
            | Remainder
            | Exponential
            | ShiftLeft
            | ShiftRight
            | ShiftRightZeroFill
            | BitwiseAnd
            | BitwiseOR
            | BitwiseXOR
    )
}

fn assignment_target_lowerable(t: &js::AssignmentTarget) -> bool {
    use js::AssignmentTarget::*;
    match t {
        AssignmentTargetIdentifier(_) => true,
        StaticMemberExpression(m) => expr_lowerable(&m.object),
        ComputedMemberExpression(m) => expr_lowerable(&m.object) && expr_lowerable(&m.expression),
        _ => false,
    }
}

/// `++`/`--` apply to a SimpleAssignmentTarget — identifier, static
/// member, or computed member. Same shape as a plain assignment
/// target.
fn update_target_lowerable(target: &js::SimpleAssignmentTarget) -> bool {
    use js::SimpleAssignmentTarget::*;
    match target {
        AssignmentTargetIdentifier(_) => true,
        StaticMemberExpression(m) => expr_lowerable(&m.object),
        ComputedMemberExpression(m) => expr_lowerable(&m.object) && expr_lowerable(&m.expression),
        _ => false,
    }
}

// ----------------------------------------------------------------------
// Function declarations / expressions / arrows
// ----------------------------------------------------------------------

fn function_decl_lowerable(f: &js::Function) -> bool {
    // We accept nested function declarations regardless of their inner
    // body's disposition — the inner body gets its own decision when
    // lowering recurses into it. The outer function being Lowered does
    // NOT require the inner function being Lowered.
    //
    // What we DO need to reject here: the load-bearing rule from
    // memory `closures-disposition-rule` — captures cannot be
    // forward-references and cannot be reassigned in the outer scope.
    // Resolving that requires symbol-resolution on the AST, which the
    // walker doesn't have access to in isolation. The lowering performs
    // the actual capture analysis and routes inner functions to
    // PassThrough when forward-ref / reassign rules are violated; the
    // walker conservatively accepts any nested function decl whose
    // local shape is otherwise lowerable.
    //
    // Reject features the lowering doesn't model:
    if f.r#async || f.generator {
        return false;
    }
    if !params_lowerable(&f.params) {
        return false;
    }
    f.id.is_some() // anonymous function declarations aren't legal JS anyway
}

fn function_expr_lowerable(f: &js::Function) -> bool {
    // Same shape as function decls; just no name requirement.
    if f.r#async || f.generator {
        return false;
    }
    params_lowerable(&f.params)
}

fn arrow_lowerable(a: &js::ArrowFunctionExpression) -> bool {
    if a.r#async {
        return false;
    }
    params_lowerable(&a.params)
}

// ----------------------------------------------------------------------
// Expressions
// ----------------------------------------------------------------------

fn expr_lowerable(expr: &js::Expression) -> bool {
    use js::Expression::*;
    match expr {
        NumericLiteral(_) | StringLiteral(_) | BooleanLiteral(_) | NullLiteral(_) => true,
        Identifier(_) => true,
        ParenthesizedExpression(p) => expr_lowerable(&p.expression),

        BinaryExpression(b) => {
            binop_lowerable(b.operator) && expr_lowerable(&b.left) && expr_lowerable(&b.right)
        }
        LogicalExpression(l) => match l.operator {
            js::LogicalOperator::And | js::LogicalOperator::Or => {
                expr_lowerable(&l.left) && expr_lowerable(&l.right)
            }
            // ?? not lowered.
            _ => false,
        },
        UnaryExpression(u) => unop_lowerable(u.operator) && expr_lowerable(&u.argument),

        CallExpression(c) => {
            if !expr_lowerable(&c.callee) {
                return false;
            }
            for a in &c.arguments {
                match a {
                    js::Argument::SpreadElement(_) => return false,
                    other => match other.as_expression() {
                        Some(e) => {
                            if !expr_lowerable(e) {
                                return false;
                            }
                        }
                        None => return false,
                    },
                }
            }
            true
        }

        StaticMemberExpression(m) => expr_lowerable(&m.object),
        ComputedMemberExpression(m) => expr_lowerable(&m.object) && expr_lowerable(&m.expression),

        // Closure expressions: the body's lowerability is the inner
        // function's own decision, NOT this walker's concern. We only
        // need to make sure the literal itself is shape-acceptable.
        FunctionExpression(f) => function_expr_lowerable(f),
        ArrowFunctionExpression(a) => arrow_lowerable(a),

        NewExpression(n) => {
            if !expr_lowerable(&n.callee) {
                return false;
            }
            for a in &n.arguments {
                match a {
                    js::Argument::SpreadElement(_) => return false,
                    other => match other.as_expression() {
                        Some(e) => {
                            if !expr_lowerable(e) {
                                return false;
                            }
                        }
                        None => return false,
                    },
                }
            }
            true
        }

        ObjectExpression(obj) => {
            for prop in &obj.properties {
                match prop {
                    js::ObjectPropertyKind::ObjectProperty(p) => {
                        if p.method || p.kind != js::PropertyKind::Init {
                            return false;
                        }
                        if !expr_lowerable(&p.value) {
                            return false;
                        }
                        if p.computed {
                            match p.key.as_expression() {
                                Some(e) => {
                                    if !expr_lowerable(e) {
                                        return false;
                                    }
                                }
                                None => return false,
                            }
                        } else {
                            match &p.key {
                                js::PropertyKey::StaticIdentifier(_)
                                | js::PropertyKey::StringLiteral(_)
                                | js::PropertyKey::NumericLiteral(_) => {}
                                _ => return false,
                            }
                        }
                    }
                    js::ObjectPropertyKind::SpreadProperty(_) => return false,
                }
            }
            true
        }

        ConditionalExpression(c) => {
            expr_lowerable(&c.test)
                && expr_lowerable(&c.consequent)
                && expr_lowerable(&c.alternate)
        }

        UpdateExpression(u) => update_target_lowerable(&u.argument),

        ArrayExpression(arr) => {
            for el in &arr.elements {
                match el {
                    js::ArrayExpressionElement::SpreadElement(_) => return false,
                    js::ArrayExpressionElement::Elision(_) => return false,
                    other => match other.as_expression() {
                        Some(e) => {
                            if !expr_lowerable(e) {
                                return false;
                            }
                        }
                        None => return false,
                    },
                }
            }
            true
        }

        _ => false,
    }
}

fn binop_lowerable(op: js::BinaryOperator) -> bool {
    use js::BinaryOperator::*;
    matches!(
        op,
        Addition
            | Subtraction
            | Multiplication
            | Division
            | Remainder
            | Equality
            | StrictEquality
            | Inequality
            | StrictInequality
            | LessThan
            | LessEqualThan
            | GreaterThan
            | GreaterEqualThan
            | BitwiseAnd
            | BitwiseOR
            | BitwiseXOR
            | ShiftLeft
            | ShiftRight
            | ShiftRightZeroFill
    )
}

fn unop_lowerable(op: js::UnaryOperator) -> bool {
    use js::UnaryOperator::*;
    matches!(
        op,
        UnaryNegation | UnaryPlus | LogicalNot | BitwiseNot | Typeof | Void
    )
}
