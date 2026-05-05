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

use std::collections::BTreeSet;

use ir::blocker::BlockerKind;
use oxc_ast::ast as js;

/// Disposition of a function body: either fully lowered into our IR,
/// or preserved as source text because it contains one or more constructs
/// the lowering cannot represent.
///
/// `PassThrough` carries the full set of blocking construct kinds found
/// in the walk — non-empty by construction. Callers use `is_pass_through()`
/// rather than pattern-matching directly when they only need the yes/no.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Disposition {
    Lowered,
    PassThrough(BTreeSet<BlockerKind>),
}

impl Disposition {
    pub fn is_pass_through(&self) -> bool {
        matches!(self, Disposition::PassThrough(_))
    }
}

/// Decide a function's disposition. Walks `params` and `body`,
/// stopping at nested function boundaries.
pub fn decide(params: &js::FormalParameters, body: &js::FunctionBody) -> Disposition {
    let mut blockers = BTreeSet::new();
    collect_param_blockers(params, &mut blockers);
    for stmt in &body.statements {
        collect_stmt_blockers(stmt, &mut blockers);
    }
    if blockers.is_empty() {
        Disposition::Lowered
    } else {
        Disposition::PassThrough(blockers)
    }
}

/// Convenience: decide an arrow function. Arrows can have either an
/// expression body or a block body; OXC wraps both in `FunctionBody`.
pub fn decide_arrow(arrow: &js::ArrowFunctionExpression) -> Disposition {
    let mut blockers = BTreeSet::new();
    collect_param_blockers(&arrow.params, &mut blockers);
    for stmt in &arrow.body.statements {
        collect_stmt_blockers(stmt, &mut blockers);
    }
    if blockers.is_empty() {
        Disposition::Lowered
    } else {
        Disposition::PassThrough(blockers)
    }
}

// ----------------------------------------------------------------------
// Parameters
// ----------------------------------------------------------------------

fn collect_param_blockers(params: &js::FormalParameters, blockers: &mut BTreeSet<BlockerKind>) {
    if params.rest.is_some() {
        blockers.insert(BlockerKind::RestParam);
    }
    for p in &params.items {
        match &p.pattern.kind {
            js::BindingPatternKind::BindingIdentifier(_) => {}
            js::BindingPatternKind::ObjectPattern(_) | js::BindingPatternKind::ArrayPattern(_) => {
                blockers.insert(BlockerKind::DestructuringParam);
            }
            js::BindingPatternKind::AssignmentPattern(_) => {
                blockers.insert(BlockerKind::DefaultParam);
            }
        }
    }
}

// ----------------------------------------------------------------------
// Statements
// ----------------------------------------------------------------------

fn collect_stmt_blockers(stmt: &js::Statement, blockers: &mut BTreeSet<BlockerKind>) {
    use js::Statement::*;
    match stmt {
        VariableDeclaration(decl) => collect_var_decl_blockers(decl, blockers),
        ExpressionStatement(es) => collect_expr_stmt_blockers(&es.expression, blockers),
        ReturnStatement(r) => {
            if let Some(e) = &r.argument {
                collect_expr_blockers(e, blockers);
            }
        }
        IfStatement(s) => {
            collect_expr_blockers(&s.test, blockers);
            collect_stmt_blockers(&s.consequent, blockers);
            if let Some(a) = &s.alternate {
                collect_stmt_blockers(a, blockers);
            }
        }
        WhileStatement(s) => {
            collect_expr_blockers(&s.test, blockers);
            collect_stmt_blockers(&s.body, blockers);
        }
        BlockStatement(b) => {
            for s in &b.body {
                collect_stmt_blockers(s, blockers);
            }
        }
        FunctionDeclaration(f) => collect_func_decl_blockers(f, blockers),
        ForStatement(f) => collect_for_stmt_blockers(f, blockers),
        ForOfStatement(f) => collect_for_of_stmt_blockers(f, blockers),
        ContinueStatement(c) => {
            if c.label.is_some() {
                blockers.insert(BlockerKind::LabeledContinue);
            }
        }
        TryStatement(t) => collect_try_stmt_blockers(t, blockers),
        ThrowStatement(t) => {
            // `throw expr;` lowers to `Stmt::Throw`. SSA can't model
            // exception flow yet, so containing functions get routed
            // to PassThrough at SSA build time — but the AST-level
            // disposition still says Lowered, which is what the
            // coverage table reads.
            collect_expr_blockers(&t.argument, blockers);
        }
        BreakStatement(_) => {
            blockers.insert(BlockerKind::BreakStmt);
        }
        SwitchStatement(s) => {
            blockers.insert(BlockerKind::SwitchStmt);
            collect_expr_blockers(&s.discriminant, blockers);
            for case in &s.cases {
                if let Some(test) = &case.test {
                    collect_expr_blockers(test, blockers);
                }
                for stmt in &case.consequent {
                    collect_stmt_blockers(stmt, blockers);
                }
            }
        }
        DoWhileStatement(s) => {
            blockers.insert(BlockerKind::DoWhileStmt);
            collect_expr_blockers(&s.test, blockers);
            collect_stmt_blockers(&s.body, blockers);
        }
        ForInStatement(f) => {
            // The for-in loop is preserved as `Stmt::ForIn`. The IR
            // only represents the simple form: single-declarator
            // var/let/const with a plain-identifier binding. Fall back
            // to the blocker for exotic shapes (multiple declarators,
            // destructuring, no declaration on the left, etc.).
            let binding_ok = match &f.left {
                js::ForStatementLeft::VariableDeclaration(decl) => {
                    decl.declarations.len() == 1
                        && matches!(
                            decl.declarations[0].id.kind,
                            js::BindingPatternKind::BindingIdentifier(_)
                        )
                        && matches!(
                            decl.kind,
                            js::VariableDeclarationKind::Const
                                | js::VariableDeclarationKind::Let
                                | js::VariableDeclarationKind::Var
                        )
                }
                _ => false,
            };
            if !binding_ok {
                blockers.insert(BlockerKind::ForInStmt);
            }
            collect_expr_blockers(&f.right, blockers);
            collect_stmt_blockers(&f.body, blockers);
        }
        LabeledStatement(s) => {
            blockers.insert(BlockerKind::LabeledStmt);
            collect_stmt_blockers(&s.body, blockers);
        }
        DebuggerStatement(_) => {
            blockers.insert(BlockerKind::DebuggerStmt);
        }
        ClassDeclaration(_) => {
            blockers.insert(BlockerKind::ClassDecl);
        }
        _ => {
            blockers.insert(BlockerKind::UnsupportedStmt);
        }
    }
}

fn collect_try_stmt_blockers(t: &js::TryStatement, blockers: &mut BTreeSet<BlockerKind>) {
    for stmt in &t.block.body {
        collect_stmt_blockers(stmt, blockers);
    }
    if let Some(handler) = &t.handler {
        if let Some(param) = &handler.param {
            if !matches!(param.pattern.kind, js::BindingPatternKind::BindingIdentifier(_)) {
                blockers.insert(BlockerKind::TryCatchDestructuringParam);
            }
        }
        for stmt in &handler.body.body {
            collect_stmt_blockers(stmt, blockers);
        }
    }
    if let Some(finalizer) = &t.finalizer {
        for stmt in &finalizer.body {
            collect_stmt_blockers(stmt, blockers);
        }
    }
}

fn collect_for_stmt_blockers(f: &js::ForStatement, blockers: &mut BTreeSet<BlockerKind>) {
    if let Some(init) = &f.init {
        collect_for_init_blockers(init, blockers);
    }
    if let Some(test) = &f.test {
        collect_expr_blockers(test, blockers);
    }
    if let Some(update) = &f.update {
        collect_expr_blockers(update, blockers);
    }
    collect_stmt_blockers(&f.body, blockers);
}

fn collect_for_of_stmt_blockers(f: &js::ForOfStatement, blockers: &mut BTreeSet<BlockerKind>) {
    if f.r#await {
        blockers.insert(BlockerKind::ForAwaitOf);
    }
    let binding_ok = match &f.left {
        js::ForStatementLeft::VariableDeclaration(decl) => {
            decl.declarations.len() == 1
                && matches!(
                    decl.declarations[0].id.kind,
                    js::BindingPatternKind::BindingIdentifier(_)
                )
                && matches!(
                    decl.kind,
                    js::VariableDeclarationKind::Const
                        | js::VariableDeclarationKind::Let
                        | js::VariableDeclarationKind::Var
                )
        }
        _ => false,
    };
    if !binding_ok {
        blockers.insert(BlockerKind::ForOfDestructuringBinding);
    }
    // Reject if the body contains a bare return. The desugared iterator
    // protocol does not call iterator.return() on abrupt completion.
    if stmt_has_bare_return(&f.body) {
        blockers.insert(BlockerKind::ForOfBareReturn);
    }
    collect_expr_blockers(&f.right, blockers);
    collect_stmt_blockers(&f.body, blockers);
}

fn collect_for_init_blockers(init: &js::ForStatementInit, blockers: &mut BTreeSet<BlockerKind>) {
    match init {
        js::ForStatementInit::VariableDeclaration(decl) => {
            // For-init multi-declarator: lowerable for `var` (function-
            // scoped, so hoisting all-but-the-first declarator out of
            // the for-init is semantics-preserving). For `let`/`const`,
            // the per-iteration scoping changes if we hoist out, so
            // we keep them blocked. (Zero corpus occurrences as of
            // this writing.)
            if decl.declarations.len() != 1
                && !matches!(decl.kind, js::VariableDeclarationKind::Var)
            {
                blockers.insert(BlockerKind::MultipleDeclarators);
                for d in &decl.declarations {
                    if let Some(e) = &d.init {
                        collect_expr_blockers(e, blockers);
                    }
                }
                return;
            }
            collect_var_decl_blockers(decl, blockers);
        }
        other => match other.as_expression() {
            Some(e) => collect_expr_blockers(e, blockers),
            None => {
                blockers.insert(BlockerKind::UnsupportedStmt);
            }
        },
    }
}

fn collect_var_decl_blockers(decl: &js::VariableDeclaration, blockers: &mut BTreeSet<BlockerKind>) {
    // Statement-level multi-declarator declarations are lowered by splitting into N
    // separate single-declarator statements, so they are no longer a blocker.
    // Check each declarator independently.
    match decl.kind {
        js::VariableDeclarationKind::Let
        | js::VariableDeclarationKind::Var
        | js::VariableDeclarationKind::Const => {}
        _ => {
            blockers.insert(BlockerKind::UnsupportedVarKind);
        }
    }
    for d in &decl.declarations {
        if !matches!(d.id.kind, js::BindingPatternKind::BindingIdentifier(_)) {
            blockers.insert(BlockerKind::DestructuringBinding);
        }
        if let Some(e) = &d.init {
            collect_expr_blockers(e, blockers);
        }
    }
}

fn collect_expr_stmt_blockers(expr: &js::Expression, blockers: &mut BTreeSet<BlockerKind>) {
    if let js::Expression::AssignmentExpression(a) = expr {
        if !assignment_op_lowerable(a.operator) {
            blockers.insert(BlockerKind::LogicalAssignOp);
        }
        collect_assignment_target_blockers(&a.left, blockers);
        collect_expr_blockers(&a.right, blockers);
        return;
    }
    collect_expr_blockers(expr, blockers);
}

fn collect_assignment_target_blockers(
    t: &js::AssignmentTarget,
    blockers: &mut BTreeSet<BlockerKind>,
) {
    use js::AssignmentTarget::*;
    match t {
        AssignmentTargetIdentifier(_) => {}
        StaticMemberExpression(m) => collect_expr_blockers(&m.object, blockers),
        ComputedMemberExpression(m) => {
            collect_expr_blockers(&m.object, blockers);
            collect_expr_blockers(&m.expression, blockers);
        }
        _ => {
            blockers.insert(BlockerKind::UnsupportedExpr);
        }
    }
}

fn collect_update_target_blockers(
    target: &js::SimpleAssignmentTarget,
    blockers: &mut BTreeSet<BlockerKind>,
) {
    use js::SimpleAssignmentTarget::*;
    match target {
        AssignmentTargetIdentifier(_) => {}
        StaticMemberExpression(m) => collect_expr_blockers(&m.object, blockers),
        ComputedMemberExpression(m) => {
            collect_expr_blockers(&m.object, blockers);
            collect_expr_blockers(&m.expression, blockers);
        }
        _ => {
            blockers.insert(BlockerKind::UnsupportedExpr);
        }
    }
}

// ----------------------------------------------------------------------
// Function declarations / expressions / arrows (nested — stop descent)
// ----------------------------------------------------------------------

fn collect_func_decl_blockers(f: &js::Function, blockers: &mut BTreeSet<BlockerKind>) {
    if f.r#async {
        blockers.insert(BlockerKind::AsyncFunction);
    }
    if f.generator {
        blockers.insert(BlockerKind::GeneratorFunction);
    }
    if f.id.is_none() {
        blockers.insert(BlockerKind::AnonymousFunctionDecl);
    }
    collect_param_blockers(&f.params, blockers);
    // Do NOT recurse into body — nested functions have their own disposition.
}

fn collect_func_expr_blockers(f: &js::Function, blockers: &mut BTreeSet<BlockerKind>) {
    if f.r#async {
        blockers.insert(BlockerKind::AsyncFunction);
    }
    if f.generator {
        blockers.insert(BlockerKind::GeneratorFunction);
    }
    collect_param_blockers(&f.params, blockers);
    // Do NOT recurse into body.
}

fn collect_arrow_blockers(a: &js::ArrowFunctionExpression, blockers: &mut BTreeSet<BlockerKind>) {
    if a.r#async {
        blockers.insert(BlockerKind::AsyncFunction);
    }
    collect_param_blockers(&a.params, blockers);
    // Do NOT recurse into body.
}

// ----------------------------------------------------------------------
// Expressions
// ----------------------------------------------------------------------

fn collect_expr_blockers(expr: &js::Expression, blockers: &mut BTreeSet<BlockerKind>) {
    use js::Expression::*;
    match expr {
        NumericLiteral(_) | StringLiteral(_) | BooleanLiteral(_) | NullLiteral(_) => {}
        Identifier(_) => {}
        ParenthesizedExpression(p) => collect_expr_blockers(&p.expression, blockers),

        BinaryExpression(b) => {
            if !binop_lowerable(b.operator) {
                blockers.insert(BlockerKind::UnsupportedBinOp);
            }
            collect_expr_blockers(&b.left, blockers);
            collect_expr_blockers(&b.right, blockers);
        }

        LogicalExpression(l) => {
            match l.operator {
                js::LogicalOperator::And | js::LogicalOperator::Or => {}
                _ => {
                    blockers.insert(BlockerKind::NullishCoalescing);
                }
            }
            collect_expr_blockers(&l.left, blockers);
            collect_expr_blockers(&l.right, blockers);
        }

        UnaryExpression(u) => {
            if !unop_lowerable(u.operator) {
                blockers.insert(BlockerKind::UnsupportedUnaryOp);
            }
            collect_expr_blockers(&u.argument, blockers);
        }

        CallExpression(c) => {
            collect_expr_blockers(&c.callee, blockers);
            for a in &c.arguments {
                match a {
                    js::Argument::SpreadElement(s) => collect_expr_blockers(&s.argument, blockers),
                    other => {
                        if let Some(e) = other.as_expression() {
                            collect_expr_blockers(e, blockers);
                        }
                    }
                }
            }
        }

        StaticMemberExpression(m) => collect_expr_blockers(&m.object, blockers),
        ComputedMemberExpression(m) => {
            collect_expr_blockers(&m.object, blockers);
            collect_expr_blockers(&m.expression, blockers);
        }

        FunctionExpression(f) => collect_func_expr_blockers(f, blockers),
        ArrowFunctionExpression(a) => collect_arrow_blockers(a, blockers),

        NewExpression(n) => {
            collect_expr_blockers(&n.callee, blockers);
            for a in &n.arguments {
                match a {
                    js::Argument::SpreadElement(s) => collect_expr_blockers(&s.argument, blockers),
                    other => {
                        if let Some(e) = other.as_expression() {
                            collect_expr_blockers(e, blockers);
                        }
                    }
                }
            }
        }

        ObjectExpression(obj) => {
            for prop in &obj.properties {
                match prop {
                    js::ObjectPropertyKind::ObjectProperty(p) => {
                        if p.method || p.kind != js::PropertyKind::Init {
                            blockers.insert(BlockerKind::UnsupportedExpr);
                        } else {
                            collect_expr_blockers(&p.value, blockers);
                            if p.computed {
                                match p.key.as_expression() {
                                    Some(e) => collect_expr_blockers(e, blockers),
                                    None => {
                                        blockers.insert(BlockerKind::UnsupportedExpr);
                                    }
                                }
                            } else {
                                match &p.key {
                                    js::PropertyKey::StaticIdentifier(_)
                                    | js::PropertyKey::StringLiteral(_)
                                    | js::PropertyKey::NumericLiteral(_) => {}
                                    _ => {
                                        blockers.insert(BlockerKind::UnsupportedExpr);
                                    }
                                }
                            }
                        }
                    }
                    js::ObjectPropertyKind::SpreadProperty(s) => {
                        collect_expr_blockers(&s.argument, blockers);
                    }
                }
            }
        }

        ConditionalExpression(c) => {
            collect_expr_blockers(&c.test, blockers);
            collect_expr_blockers(&c.consequent, blockers);
            collect_expr_blockers(&c.alternate, blockers);
        }

        UpdateExpression(u) => collect_update_target_blockers(&u.argument, blockers),

        TemplateLiteral(tpl) => {
            for e in &tpl.expressions {
                collect_expr_blockers(e, blockers);
            }
        }

        ArrayExpression(arr) => {
            for el in &arr.elements {
                match el {
                    js::ArrayExpressionElement::SpreadElement(s) => {
                        collect_expr_blockers(&s.argument, blockers);
                    }
                    js::ArrayExpressionElement::Elision(_) => {
                        blockers.insert(BlockerKind::ArrayElision);
                    }
                    other => {
                        if let Some(e) = other.as_expression() {
                            collect_expr_blockers(e, blockers);
                        }
                    }
                }
            }
        }

        // Named unsupported kinds — each gets its own variant so the
        // frequency table is actionable.
        SequenceExpression(s) => {
            // Sequence is now lowered to `Expr::Sequence`. Walk
            // children so other blockers in the operands are still
            // surfaced.
            for e in &s.expressions {
                collect_expr_blockers(e, blockers);
            }
        }
        AwaitExpression(a) => {
            blockers.insert(BlockerKind::AwaitExpr);
            collect_expr_blockers(&a.argument, blockers);
        }
        YieldExpression(y) => {
            blockers.insert(BlockerKind::YieldExpr);
            if let Some(arg) = &y.argument {
                collect_expr_blockers(arg, blockers);
            }
        }
        TaggedTemplateExpression(_) => {
            blockers.insert(BlockerKind::TaggedTemplateExpr);
        }
        ChainExpression(_) => {
            blockers.insert(BlockerKind::ChainExpr);
        }
        ImportExpression(_) => {
            blockers.insert(BlockerKind::ImportExpr);
        }
        MetaProperty(_) => {
            blockers.insert(BlockerKind::MetaProperty);
        }
        RegExpLiteral(_) => {
            blockers.insert(BlockerKind::RegExpLiteral);
        }
        BigIntLiteral(_) => {
            blockers.insert(BlockerKind::BigIntLiteral);
        }
        ThisExpression(_) => {
            // `this` is preserved syntactically as `Expr::This` and
            // flows through SSA as `Constant::This`. Correct because
            // expressions stay inside their producing function body
            // (no boundary-crossing pass exists today).
        }
        AssignmentExpression(a) => {
            // Plain `=` and arithmetic/bitwise compound (+=, -=, *=,
            // /=, %=, **=, &=, |=, ^=, <<=, >>=, >>>=) are all lowered
            // in expression position. Only logical-assign (`||=`,
            // `&&=`, `??=`) remains blocked.
            if !assignment_op_lowerable(a.operator) {
                blockers.insert(BlockerKind::LogicalAssignOp);
            }
            // Walk children so blockers inside target/value are surfaced.
            collect_assignment_target_blockers(&a.left, blockers);
            collect_expr_blockers(&a.right, blockers);
        }
        _ => {
            blockers.insert(BlockerKind::UnsupportedExpr);
        }
    }
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
            | Exponential
    )
}

fn unop_lowerable(op: js::UnaryOperator) -> bool {
    use js::UnaryOperator::*;
    matches!(
        op,
        UnaryNegation | UnaryPlus | LogicalNot | BitwiseNot | Typeof | Void
    )
}

/// Returns true if `stmt` contains a ReturnStatement not inside a nested
/// function boundary. Unchanged from original — needed by `collect_for_of_stmt_blockers`.
fn stmt_has_bare_return(stmt: &js::Statement) -> bool {
    use js::Statement::*;
    match stmt {
        ReturnStatement(_) => true,
        BlockStatement(b) => b.body.iter().any(stmt_has_bare_return),
        IfStatement(s) => {
            stmt_has_bare_return(&s.consequent)
                || s.alternate.as_ref().is_some_and(|a| stmt_has_bare_return(a))
        }
        WhileStatement(s) => stmt_has_bare_return(&s.body),
        ForStatement(s) => stmt_has_bare_return(&s.body),
        ForOfStatement(s) => stmt_has_bare_return(&s.body),
        TryStatement(t) => {
            t.block.body.iter().any(stmt_has_bare_return)
                || t.handler
                    .as_ref()
                    .is_some_and(|h| h.body.body.iter().any(stmt_has_bare_return))
                || t.finalizer
                    .as_ref()
                    .is_some_and(|f| f.body.iter().any(stmt_has_bare_return))
        }
        // Nested function declarations have their own scope.
        FunctionDeclaration(_) => false,
        ExpressionStatement(_)
        | VariableDeclaration(_)
        | ContinueStatement(_)
        | BreakStatement(_)
        | LabeledStatement(_)
        | SwitchStatement(_)
        | ThrowStatement(_)
        | DoWhileStatement(_)
        | ForInStatement(_)
        | EmptyStatement(_)
        | DebuggerStatement(_) => false,
        _ => false,
    }
}
