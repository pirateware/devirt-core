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
    // RestParam (`function f(...args)`) still requires a variadic-call
    // ABI we don't model. Keep it blocked. The destructuring/default
    // shapes lift via parse-time desugaring (issue #29) — we still walk
    // into them here so any *inner* blocker (e.g. `function f({a = b ?? 0})`
    // contains NullishCoalescing) still surfaces.
    if params.rest.is_some() {
        blockers.insert(BlockerKind::RestParam);
    }
    for p in &params.items {
        collect_binding_pattern_blockers(&p.pattern, blockers);
    }
}

/// Walk a binding pattern (used for both function parameters and the
/// catch handler param) and surface any blockers inside default
/// expressions or nested destructuring.
fn collect_binding_pattern_blockers(
    pat: &js::BindingPattern,
    blockers: &mut BTreeSet<BlockerKind>,
) {
    match &pat.kind {
        js::BindingPatternKind::BindingIdentifier(_) => {}
        js::BindingPatternKind::AssignmentPattern(ap) => {
            // Default expression — walk it for any inner blockers.
            collect_expr_blockers(&ap.right, blockers);
            collect_binding_pattern_blockers(&ap.left, blockers);
        }
        js::BindingPatternKind::ObjectPattern(op) => {
            // Object rest in destructuring (`{a, ...rest}`) needs the
            // same variadic-extraction machinery as RestParam — keep
            // blocked under RestParam for consistency.
            if op.rest.is_some() {
                blockers.insert(BlockerKind::RestParam);
            }
            for prop in &op.properties {
                if prop.computed {
                    // Computed destructuring keys are valid JS but the
                    // desugared `LoadIndex(t, key)` form needs the key
                    // expression evaluated at the destructuring site;
                    // surface inner blockers and refuse if the key
                    // shape isn't lowerable.
                    match prop.key.as_expression() {
                        Some(e) => collect_expr_blockers(e, blockers),
                        None => {
                            blockers.insert(BlockerKind::UnsupportedExpr);
                        }
                    }
                } else {
                    match &prop.key {
                        js::PropertyKey::StaticIdentifier(_)
                        | js::PropertyKey::StringLiteral(_)
                        | js::PropertyKey::NumericLiteral(_) => {}
                        _ => {
                            blockers.insert(BlockerKind::UnsupportedExpr);
                        }
                    }
                }
                collect_binding_pattern_blockers(&prop.value, blockers);
            }
        }
        js::BindingPatternKind::ArrayPattern(ap) => {
            if ap.rest.is_some() {
                blockers.insert(BlockerKind::RestParam);
            }
            for el in &ap.elements {
                if let Some(p) = el {
                    collect_binding_pattern_blockers(p, blockers);
                }
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
        ContinueStatement(_) => {
            // Both unlabeled and labeled `continue` are now lowered:
            // labeled `continue label;` lifts to `Stmt::LabeledContinue`,
            // resolved at SSA build time against the matching labeled
            // loop frame. (Unlabeled `continue` was already lowered.)
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
            // `break` lowers to `Stmt::Break`. SSA can't model break
            // targets yet, so containing functions get routed to
            // PassThrough at SSA build time — but the AST-level
            // disposition still says Lowered, which is what the
            // coverage table reads.
        }
        SwitchStatement(s) => {
            // `switch` lowers to `Stmt::Switch`. Same SSA-fallback
            // pattern as `break` / throw / for-in / try — preserved as
            // syntactic source through tree-IR, opaque in SSA.
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
            // `do { body } while (cond);` lowers to `Stmt::DoWhile` in
            // tree-IR and the SSA layer desugars it into a top-test
            // loop with a tail-branch to enforce test-after-body
            // semantics. Out-of-SSA reconstruction relooper does not
            // yet recognize the test-after-body pattern, so the
            // round-trip would not preserve do-while shape; leaving
            // the disposition path as Lowered would panic the
            // relooper. For now we keep DoWhileStmt as a blocker,
            // routing such functions to PassThrough where the
            // verbatim source slice is preserved verbatim. The IR
            // variant `Stmt::DoWhile` is in place so a later pass
            // (relooper extension) can lift this without further IR
            // churn.
            blockers.insert(BlockerKind::DoWhileStmt);
            collect_expr_blockers(&s.test, blockers);
            collect_stmt_blockers(&s.body, blockers);
        }
        ForInStatement(f) => {
            // The for-in loop is preserved as `Stmt::ForIn` with a
            // plain-identifier binding. Destructuring bindings
            // (`for (const {a} of ...)`-style for-in) lift via parse-
            // time desugaring: bind a fresh temp on the left and emit
            // a destructuring prologue at the top of the body. Walk
            // the binding pattern so any *inner* expression blockers
            // (default values, computed keys) still surface.
            let binding_ok = match &f.left {
                js::ForStatementLeft::VariableDeclaration(decl) => {
                    decl.declarations.len() == 1
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
            } else if let js::ForStatementLeft::VariableDeclaration(decl) = &f.left {
                if let Some(d) = decl.declarations.first() {
                    collect_binding_pattern_blockers(&d.id, blockers);
                }
            }
            collect_expr_blockers(&f.right, blockers);
            collect_stmt_blockers(&f.body, blockers);
        }
        LabeledStatement(s) => {
            // `label: stmt` lowers to `Stmt::Labeled`. Labeled
            // break/continue inside the body resolves at SSA build time
            // against the matching labeled frame on the break stack.
            collect_stmt_blockers(&s.body, blockers);
        }
        DebuggerStatement(_) => {
            // `debugger;` is a no-op for our purposes — preserved as
            // `Stmt::Debugger` through tree-IR (round-trip exact); SSA
            // lowering elides it.
        }
        ClassDeclaration(c) => {
            // Plain class shapes desugar at parse time to a `function`
            // + prototype assignments; they do NOT block. Only shapes
            // the desugar can't represent (private members, static
            // blocks, decorators) leave the function PassThrough.
            // `class_blockers` enumerates them.
            //
            // Anonymous class declaration (`class { ... }`) is a parse
            // error in JS, but be defensive.
            if c.id.is_none() {
                blockers.insert(BlockerKind::ClassDecl);
            }
            blockers.extend(crate::class_desugar::class_blockers(c));
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
            // Catch destructuring (`catch ({message})`) lifts via the
            // same parse-time desugaring as DestructuringParam (issue #29).
            // Walk the pattern so inner blockers still surface.
            collect_binding_pattern_blockers(&param.pattern, blockers);
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
    // For-of with single-declarator var/let/const is lowerable. A
    // destructuring binding (`for (const {a} of xs)`) lifts via
    // parse-time desugaring: bind a fresh temp and emit a
    // destructuring prologue at the top of the body. Walk the binding
    // pattern so any *inner* blockers still surface.
    let binding_ok = match &f.left {
        js::ForStatementLeft::VariableDeclaration(decl) => {
            decl.declarations.len() == 1
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
    } else if let js::ForStatementLeft::VariableDeclaration(decl) = &f.left {
        if let Some(d) = decl.declarations.first() {
            collect_binding_pattern_blockers(&d.id, blockers);
        }
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
            // `&&`, `||` and `??` are all lowerable. Walk children
            // for any inner blockers.
            let _ = l.operator;
            collect_expr_blockers(&l.left, blockers);
            collect_expr_blockers(&l.right, blockers);
        }

        UnaryExpression(u) => {
            if matches!(u.operator, js::UnaryOperator::Delete) {
                // `delete obj.k` and `delete obj[k]` are lowerable.
                // Bare-ident `delete x` is sloppy-mode-only and
                // unsupported. Look through parens.
                let target = match &u.argument {
                    js::Expression::ParenthesizedExpression(p) => &p.expression,
                    other => other,
                };
                match target {
                    js::Expression::StaticMemberExpression(_)
                    | js::Expression::ComputedMemberExpression(_) => {}
                    _ => {
                        blockers.insert(BlockerKind::UnsupportedUnaryOp);
                    }
                }
                collect_expr_blockers(&u.argument, blockers);
                return;
            }
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
                        // Elision (`[1,,3]`) lifts at parse time by
                        // substituting `undefined` for the missing
                        // slot. Lowerable.
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
        TaggedTemplateExpression(t) => {
            // `tag`...`` desugars at parse time to a function call:
            // `tag([cooked0, cooked1, ...], expr0, ...)`. The desugar
            // is a simplification — `String.raw` tagging additionally
            // depends on the array's `.raw` property, which we don't
            // populate. For v1 we accept this trade-off.
            collect_expr_blockers(&t.tag, blockers);
            for e in &t.quasi.expressions {
                collect_expr_blockers(e, blockers);
            }
        }
        ChainExpression(c) => {
            // Optional chain (`a?.b`, `a?.()`, `a?.[k]`) is
            // lowerable. Walk children so any inner blockers (e.g.,
            // `a?.b()` where `a` itself contains an unsupported
            // construct) still surface.
            match &c.expression {
                js::ChainElement::CallExpression(ce) => {
                    collect_expr_blockers(&ce.callee, blockers);
                    for a in &ce.arguments {
                        match a {
                            js::Argument::SpreadElement(s) => {
                                collect_expr_blockers(&s.argument, blockers);
                            }
                            other => {
                                if let Some(e) = other.as_expression() {
                                    collect_expr_blockers(e, blockers);
                                }
                            }
                        }
                    }
                }
                js::ChainElement::ComputedMemberExpression(m) => {
                    collect_expr_blockers(&m.object, blockers);
                    collect_expr_blockers(&m.expression, blockers);
                }
                js::ChainElement::StaticMemberExpression(m) => {
                    collect_expr_blockers(&m.object, blockers);
                }
                _ => {
                    blockers.insert(BlockerKind::ChainExpr);
                }
            }
        }
        ImportExpression(_) => {
            blockers.insert(BlockerKind::ImportExpr);
        }
        MetaProperty(_) => {
            blockers.insert(BlockerKind::MetaProperty);
        }
        RegExpLiteral(_) => {
            // Lowers to `Expr::RegExp(<raw source>)`; SSA maps to
            // `InstKind::AllocRegExp` (PURE allocation per evaluation).
        }
        BigIntLiteral(_) => {
            // `42n` lifts to `Expr::BigInt(...)` / `Constant::BigInt(...)`.
            // Pure operations on BigInt aren't const-folded; lowering
            // accepts the literal opaquely.
        }
        ThisExpression(_) => {
            // `this` is preserved syntactically as `Expr::This` and
            // flows through SSA as `Constant::This`. Correct because
            // expressions stay inside their producing function body
            // (no boundary-crossing pass exists today).
        }
        ClassExpression(c) => {
            // Same desugar as `ClassDeclaration`; only un-desugarable
            // shapes block.
            blockers.extend(crate::class_desugar::class_blockers(c));
        }
        Super(_) => {
            // Bare `super` reaches here only as a leaf — the
            // surrounding member-expression / call-expression cases
            // walk into the object/callee, hitting Super here. The
            // super itself is fine; the lowering rewrites it into the
            // captured-superclass binding when it has a SuperCtx, and
            // refuses with `Error::Unsupported` otherwise. The walker
            // can't easily check whether a SuperCtx will be present at
            // lower time, so we permit it here unconditionally and
            // rely on lowering's safety net.
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
            | LogicalAnd
            | LogicalOr
            | LogicalNullish
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
            | In
            | Instanceof
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
