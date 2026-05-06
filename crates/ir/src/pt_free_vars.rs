//! Free-variable analysis for `FuncBody::PassThrough` source slices.
//!
//! **Architectural rule** (re-stated here because every future closure
//! ticket needs to know): PassThrough is opaque to *passes*, not to
//! *scope analysis*. Optimization passes treat the body as an opaque
//! string they cannot reason about; the lexical references the body
//! depends on, by contrast, MUST be visible to the surrounding
//! lowering — otherwise captures fail to flow into the enclosing
//! function's `AllocClosure` operand list, the outer's bindings lose
//! their SSA uses, DCE drops the constants, and the renamer never
//! gets a pin signal. The result is a `ReferenceError` at runtime on
//! the exact name the PassThrough body lexically references.
//!
//! This module computes the set of free identifiers a PassThrough
//! body references by parsing the source slice with OXC and walking
//! the AST with a scope-aware visitor. A pure lexer pass is
//! insufficient: `x.y` reads `x` (free) but `y` is a property name
//! (not a free var); object shorthand `{x}` binds vs. reads `x`
//! depending on whether it's a pattern or expression; destructuring
//! introduces bindings. Only a parser-aware walk distinguishes these.
//!
//! The body is parsed twice over the run of the pipeline (once here at
//! lowering time, once at emit time for codegen). PassThrough bodies
//! are small and parsing is cheap; caching the AST would reintroduce
//! the lifetime entanglements the pipeline explicitly avoids.


use std::collections::{BTreeSet, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::SourceType;

/// Compute the alphabetically-sorted set of identifier names referenced
/// in `source` (a PassThrough body slice — the text between the
/// function's `{` and `}`, exclusive) that are NOT bound by `params`,
/// `name`, or any binding declared inside the body itself.
///
/// `is_async` and `is_generator` are required because they affect the
/// re-parse wrapper: an `await` or `yield` in the body must be valid
/// JS in the wrapped form, which means the wrapping function must
/// match the body's flavor.
///
/// Returns an empty list on parse failure (treating the body as having
/// no captures is unsound but the alternative — panicking inside
/// lowering — would crash the pipeline; parse failures here mean the
/// body slice was extracted incorrectly upstream, which is a bug we
/// want to surface elsewhere). In practice every body that survived
/// lower.rs's `passthrough_literal` is parseable by construction, so
/// this branch should never fire in production.
pub fn analyze_passthrough(
    source: &str,
    params: &[String],
    name: Option<&str>,
    is_async: bool,
    is_generator: bool,
) -> Vec<String> {
    // Wrapper synthesizes a parseable function expression around the
    // body source. The choice of keyword preserves `await`/`yield`
    // validity. Arrow bodies are normalized to block form upstream
    // (lower.rs wraps arrow expression bodies as `return <expr>;`),
    // so we don't need an arrow wrapper here.
    let params_text = params.join(", ");
    let kw = match (is_async, is_generator) {
        (true, true) => "async function*",
        (true, false) => "async function",
        (false, true) => "function*",
        (false, false) => "function",
    };
    let wrapper = format!("({kw} ({params_text}) {{{source}}});");

    let allocator = Allocator::default();
    let ret = Parser::new(&allocator, &wrapper, SourceType::default()).parse();
    if !ret.errors.is_empty() {
        // See module doc — this should be unreachable in production.
        debug_assert!(
            false,
            "pt_free_vars: failed to parse wrapped PassThrough body:\n{:#?}\n--- wrapper ---\n{}",
            ret.errors, wrapper
        );
        return Vec::new();
    }

    // Locate the function body in `(function (params) { body })`.
    let Some(body) = locate_wrapped_body(&ret.program) else {
        debug_assert!(false, "pt_free_vars: could not locate wrapped function body");
        return Vec::new();
    };

    let mut ctx = Ctx::new();
    // Function-level bindings: params + (named function expression's
    // own name binds inside the body).
    ctx.enter_function_scope();
    if let Some(n) = name {
        ctx.bind(n);
    }
    for p in params {
        ctx.bind(p);
    }
    // Hoist `var` and function declarations into the function scope
    // before walking, matching JS semantics.
    ctx.hoist_function_scope_decls(&body.statements);
    for stmt in &body.statements {
        ctx.walk_stmt(stmt);
    }
    ctx.leave_scope();

    ctx.free.into_iter().collect()
}

fn locate_wrapped_body<'a>(prog: &'a js::Program<'a>) -> Option<&'a js::FunctionBody<'a>> {
    let stmt = prog.body.first()?;
    let js::Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let inner = match &es.expression {
        js::Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    let js::Expression::FunctionExpression(f) = inner else {
        return None;
    };
    f.body.as_deref()
}

struct Ctx {
    /// Scope chain. `scopes[0]` is the function scope; deeper entries
    /// are block scopes. Idents resolve from the innermost outward.
    scopes: Vec<HashSet<String>>,
    /// Result accumulator (sorted on output via BTreeSet).
    free: BTreeSet<String>,
}

impl Ctx {
    fn new() -> Self {
        Self {
            scopes: Vec::new(),
            free: BTreeSet::new(),
        }
    }
    fn enter_function_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }
    fn enter_block_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }
    fn leave_scope(&mut self) {
        self.scopes.pop();
    }
    fn bind(&mut self, name: &str) {
        self.scopes
            .last_mut()
            .expect("no active scope")
            .insert(name.to_string());
    }
    /// Bind in the *function* scope (scopes[0] of the chain we're
    /// currently building). Used for `var` and function-decl hoisting.
    fn bind_function_scope(&mut self, name: &str) {
        // The function scope is the outermost scope this Ctx
        // currently has — we never re-enter the literal's caller.
        if let Some(s) = self.scopes.first_mut() {
            s.insert(name.to_string());
        }
    }
    fn is_bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }
    fn use_name(&mut self, name: &str) {
        if !self.is_bound(name) {
            self.free.insert(name.to_string());
        }
    }

    /// JS hoisting: `var` declarations and function declarations within
    /// the function body are visible throughout the function scope,
    /// regardless of textual position. Walk the statements once
    /// before the main walk, recording these bindings into the
    /// function scope. (`let`/`const`/class are block-scoped and are
    /// handled inline during the main walk.) Recurse into block
    /// statements so a `var` inside an `if` block still hoists.
    fn hoist_function_scope_decls(&mut self, stmts: &[js::Statement]) {
        for s in stmts {
            self.hoist_stmt(s);
        }
    }
    fn hoist_stmt(&mut self, s: &js::Statement) {
        use js::Statement::*;
        match s {
            VariableDeclaration(d) if matches!(d.kind, js::VariableDeclarationKind::Var) => {
                for decl in &d.declarations {
                    self.collect_pattern_bindings_function_scope(&decl.id);
                }
            }
            FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    self.bind_function_scope(&id.name);
                }
            }
            BlockStatement(b) => {
                for inner in &b.body {
                    self.hoist_stmt(inner);
                }
            }
            IfStatement(s) => {
                self.hoist_stmt(&s.consequent);
                if let Some(a) = &s.alternate {
                    self.hoist_stmt(a);
                }
            }
            WhileStatement(s) => self.hoist_stmt(&s.body),
            DoWhileStatement(s) => self.hoist_stmt(&s.body),
            ForStatement(s) => {
                if let Some(js::ForStatementInit::VariableDeclaration(d)) = &s.init
                    && matches!(d.kind, js::VariableDeclarationKind::Var)
                {
                    for decl in &d.declarations {
                        self.collect_pattern_bindings_function_scope(&decl.id);
                    }
                }
                self.hoist_stmt(&s.body);
            }
            ForInStatement(s) => {
                if let js::ForStatementLeft::VariableDeclaration(d) = &s.left
                    && matches!(d.kind, js::VariableDeclarationKind::Var)
                {
                    for decl in &d.declarations {
                        self.collect_pattern_bindings_function_scope(&decl.id);
                    }
                }
                self.hoist_stmt(&s.body);
            }
            ForOfStatement(s) => {
                if let js::ForStatementLeft::VariableDeclaration(d) = &s.left
                    && matches!(d.kind, js::VariableDeclarationKind::Var)
                {
                    for decl in &d.declarations {
                        self.collect_pattern_bindings_function_scope(&decl.id);
                    }
                }
                self.hoist_stmt(&s.body);
            }
            SwitchStatement(s) => {
                for case in &s.cases {
                    for inner in &case.consequent {
                        self.hoist_stmt(inner);
                    }
                }
            }
            TryStatement(s) => {
                for inner in &s.block.body {
                    self.hoist_stmt(inner);
                }
                if let Some(h) = &s.handler {
                    for inner in &h.body.body {
                        self.hoist_stmt(inner);
                    }
                }
                if let Some(f) = &s.finalizer {
                    for inner in &f.body {
                        self.hoist_stmt(inner);
                    }
                }
            }
            LabeledStatement(s) => self.hoist_stmt(&s.body),
            // Other statements cannot host hoisted declarations.
            _ => {}
        }
    }

    fn walk_stmt(&mut self, s: &js::Statement) {
        use js::Statement::*;
        match s {
            VariableDeclaration(d) => {
                let block_scoped = !matches!(d.kind, js::VariableDeclarationKind::Var);
                for decl in &d.declarations {
                    if let Some(init) = &decl.init {
                        self.walk_expr(init);
                    }
                    if block_scoped {
                        self.collect_pattern_bindings_block_scope(&decl.id);
                    }
                    // var is already hoisted; bind already happened.
                }
            }
            ExpressionStatement(es) => self.walk_expr(&es.expression),
            ReturnStatement(r) => {
                if let Some(e) = &r.argument {
                    self.walk_expr(e);
                }
            }
            IfStatement(s) => {
                self.walk_expr(&s.test);
                self.walk_stmt(&s.consequent);
                if let Some(a) = &s.alternate {
                    self.walk_stmt(a);
                }
            }
            WhileStatement(s) => {
                self.walk_expr(&s.test);
                self.walk_stmt(&s.body);
            }
            DoWhileStatement(s) => {
                self.walk_stmt(&s.body);
                self.walk_expr(&s.test);
            }
            ForStatement(s) => {
                self.enter_block_scope();
                if let Some(init) = &s.init {
                    match init {
                        js::ForStatementInit::VariableDeclaration(d) => {
                            let block_scoped =
                                !matches!(d.kind, js::VariableDeclarationKind::Var);
                            for decl in &d.declarations {
                                if let Some(init) = &decl.init {
                                    self.walk_expr(init);
                                }
                                if block_scoped {
                                    self.collect_pattern_bindings_block_scope(&decl.id);
                                }
                            }
                        }
                        e => self.walk_expr_init(e),
                    }
                }
                if let Some(t) = &s.test {
                    self.walk_expr(t);
                }
                if let Some(u) = &s.update {
                    self.walk_expr(u);
                }
                self.walk_stmt(&s.body);
                self.leave_scope();
            }
            ForInStatement(_) | ForOfStatement(_) => {
                self.walk_for_in_or_of(s);
            }
            BlockStatement(b) => {
                self.enter_block_scope();
                // Hoist function decls at this block level into the
                // *function* scope (already done by `hoist_function_scope_decls`).
                // Block-scoped: let/const/class.
                for inner in &b.body {
                    if let LabeledStatement(_) = inner {
                        // labels don't bind names
                    }
                    if let ClassDeclaration(c) = inner
                        && let Some(id) = &c.id
                    {
                        self.bind(&id.name);
                    }
                }
                for inner in &b.body {
                    self.walk_stmt(inner);
                }
                self.leave_scope();
            }
            FunctionDeclaration(f) => {
                // Already hoisted into function scope. Walk the body
                // for free vars (within the inner function's scope).
                self.walk_function(f);
            }
            ClassDeclaration(c) => {
                if let Some(id) = &c.id {
                    self.bind(&id.name);
                }
                self.walk_class(c);
            }
            SwitchStatement(s) => {
                self.walk_expr(&s.discriminant);
                self.enter_block_scope();
                for case in &s.cases {
                    if let Some(t) = &case.test {
                        self.walk_expr(t);
                    }
                    for inner in &case.consequent {
                        if let ClassDeclaration(c) = inner
                            && let Some(id) = &c.id
                        {
                            self.bind(&id.name);
                        }
                        self.walk_stmt(inner);
                    }
                }
                self.leave_scope();
            }
            TryStatement(s) => {
                self.enter_block_scope();
                for inner in &s.block.body {
                    self.walk_stmt(inner);
                }
                self.leave_scope();
                if let Some(h) = &s.handler {
                    self.enter_block_scope();
                    if let Some(p) = &h.param {
                        self.collect_pattern_bindings_block_scope(&p.pattern);
                    }
                    for inner in &h.body.body {
                        self.walk_stmt(inner);
                    }
                    self.leave_scope();
                }
                if let Some(f) = &s.finalizer {
                    self.enter_block_scope();
                    for inner in &f.body {
                        self.walk_stmt(inner);
                    }
                    self.leave_scope();
                }
            }
            ThrowStatement(s) => self.walk_expr(&s.argument),
            LabeledStatement(s) => self.walk_stmt(&s.body),
            BreakStatement(_) | ContinueStatement(_) | EmptyStatement(_) | DebuggerStatement(_) => {}
            // Imports/exports inside a function body are syntactically
            // invalid, but ignore-and-don't-crash if we see them.
            _ => {}
        }
    }

    fn walk_for_in_or_of(&mut self, s: &js::Statement) {
        use js::Statement::*;
        self.enter_block_scope();
        match s {
            ForInStatement(fi) => {
                match &fi.left {
                    js::ForStatementLeft::VariableDeclaration(d) => {
                        let block_scoped = !matches!(d.kind, js::VariableDeclarationKind::Var);
                        for decl in &d.declarations {
                            if block_scoped {
                                self.collect_pattern_bindings_block_scope(&decl.id);
                            }
                        }
                    }
                    js::ForStatementLeft::AssignmentTargetIdentifier(id) => {
                        self.use_name(&id.name);
                    }
                    other => self.walk_assignment_target_general(other),
                }
                self.walk_expr(&fi.right);
                self.walk_stmt(&fi.body);
            }
            ForOfStatement(fo) => {
                match &fo.left {
                    js::ForStatementLeft::VariableDeclaration(d) => {
                        let block_scoped = !matches!(d.kind, js::VariableDeclarationKind::Var);
                        for decl in &d.declarations {
                            if block_scoped {
                                self.collect_pattern_bindings_block_scope(&decl.id);
                            }
                        }
                    }
                    js::ForStatementLeft::AssignmentTargetIdentifier(id) => {
                        self.use_name(&id.name);
                    }
                    other => self.walk_assignment_target_general(other),
                }
                self.walk_expr(&fo.right);
                self.walk_stmt(&fo.body);
            }
            _ => unreachable!(),
        }
        self.leave_scope();
    }

    fn walk_assignment_target_general(&mut self, t: &js::ForStatementLeft) {
        // Catchall — best-effort: treat any non-binding LHS as a use of
        // its leading identifier. Member/index access reads obj.
        // Implementation kept minimal because the for-in/of LHS
        // shapes are rarely the load-bearing case for free-var
        // analysis on PassThrough bodies.
        match t {
            js::ForStatementLeft::ComputedMemberExpression(m) => {
                self.walk_expr(&m.object);
                self.walk_expr(&m.expression);
            }
            js::ForStatementLeft::StaticMemberExpression(m) => {
                self.walk_expr(&m.object);
            }
            _ => {}
        }
    }

    fn walk_function(&mut self, f: &js::Function) {
        self.enter_function_scope();
        // Named function expression: name binds inside.
        if let Some(id) = &f.id {
            self.bind(&id.name);
        }
        for p in &f.params.items {
            self.collect_pattern_bindings_block_scope(&p.pattern);
        }
        if let Some(rest) = &f.params.rest {
            self.collect_pattern_bindings_block_scope(&rest.argument);
        }
        if let Some(body) = &f.body {
            self.hoist_function_scope_decls(&body.statements);
            for stmt in &body.statements {
                self.walk_stmt(stmt);
            }
        }
        self.leave_scope();
    }

    fn walk_arrow(&mut self, a: &js::ArrowFunctionExpression) {
        self.enter_function_scope();
        for p in &a.params.items {
            self.collect_pattern_bindings_block_scope(&p.pattern);
        }
        if let Some(rest) = &a.params.rest {
            self.collect_pattern_bindings_block_scope(&rest.argument);
        }
        // Arrow body is a FunctionBody whose statements are either a
        // real block's statements or a single ExpressionStatement
        // synthesized for `(x) => x+1`. Walk uniformly.
        self.hoist_function_scope_decls(&a.body.statements);
        for stmt in &a.body.statements {
            self.walk_stmt(stmt);
        }
        self.leave_scope();
    }

    fn walk_class(&mut self, c: &js::Class) {
        // Class name binds inside the class body (decl form already
        // bound at the enclosing scope). Method/property bodies are
        // their own function scopes.
        self.enter_block_scope();
        if let Some(id) = &c.id {
            self.bind(&id.name);
        }
        if let Some(s) = &c.super_class {
            self.walk_expr(s);
        }
        for el in &c.body.body {
            match el {
                js::ClassElement::MethodDefinition(m) => {
                    self.walk_property_key(&m.key);
                    self.walk_function(&m.value);
                }
                js::ClassElement::PropertyDefinition(p) => {
                    self.walk_property_key(&p.key);
                    if let Some(v) = &p.value {
                        self.walk_expr(v);
                    }
                }
                js::ClassElement::StaticBlock(b) => {
                    self.enter_block_scope();
                    for s in &b.body {
                        self.walk_stmt(s);
                    }
                    self.leave_scope();
                }
                js::ClassElement::AccessorProperty(_) | js::ClassElement::TSIndexSignature(_) => {}
            }
        }
        self.leave_scope();
    }

    fn walk_property_key(&mut self, k: &js::PropertyKey) {
        if let Some(e) = k.as_expression() {
            // Computed keys read identifiers from scope.
            self.walk_expr(e);
        }
        // Identifier-name keys (`{ foo: ... }` static keys) are not
        // reads of `foo` — skip.
    }

    fn walk_expr(&mut self, e: &js::Expression) {
        use js::Expression::*;
        match e {
            BooleanLiteral(_) | NullLiteral(_) | NumericLiteral(_) | StringLiteral(_)
            | BigIntLiteral(_) | RegExpLiteral(_) | TemplateLiteral(_) | MetaProperty(_)
            | ThisExpression(_) | Super(_) | PrivateInExpression(_) => {
                // TemplateLiteral has expressions — handle below.
                if let TemplateLiteral(t) = e {
                    for ex in &t.expressions {
                        self.walk_expr(ex);
                    }
                }
            }
            Identifier(id) => self.use_name(&id.name),
            ParenthesizedExpression(p) => self.walk_expr(&p.expression),
            ArrayExpression(a) => {
                for el in &a.elements {
                    match el {
                        js::ArrayExpressionElement::SpreadElement(s) => {
                            self.walk_expr(&s.argument);
                        }
                        js::ArrayExpressionElement::Elision(_) => {}
                        other => {
                            if let Some(ex) = other.as_expression() {
                                self.walk_expr(ex);
                            }
                        }
                    }
                }
            }
            ObjectExpression(o) => {
                for p in &o.properties {
                    match p {
                        js::ObjectPropertyKind::ObjectProperty(prop) => {
                            // Shorthand `{x}` is `{x: x}` — the
                            // value's `Identifier` will be picked up
                            // as a read by the recursive walk.
                            self.walk_property_key(&prop.key);
                            self.walk_expr(&prop.value);
                        }
                        js::ObjectPropertyKind::SpreadProperty(s) => {
                            self.walk_expr(&s.argument);
                        }
                    }
                }
            }
            BinaryExpression(b) => {
                self.walk_expr(&b.left);
                self.walk_expr(&b.right);
            }
            LogicalExpression(l) => {
                self.walk_expr(&l.left);
                self.walk_expr(&l.right);
            }
            UnaryExpression(u) => self.walk_expr(&u.argument),
            UpdateExpression(u) => match &u.argument {
                js::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                    self.use_name(&id.name);
                }
                js::SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                    self.walk_expr(&m.object);
                    self.walk_expr(&m.expression);
                }
                js::SimpleAssignmentTarget::StaticMemberExpression(m) => {
                    self.walk_expr(&m.object);
                }
                _ => {}
            },
            AssignmentExpression(a) => {
                self.walk_assignment_target(&a.left);
                self.walk_expr(&a.right);
            }
            ConditionalExpression(c) => {
                self.walk_expr(&c.test);
                self.walk_expr(&c.consequent);
                self.walk_expr(&c.alternate);
            }
            CallExpression(c) => {
                self.walk_expr(&c.callee);
                for a in &c.arguments {
                    if let js::Argument::SpreadElement(s) = a {
                        self.walk_expr(&s.argument);
                    } else if let Some(ex) = a.as_expression() {
                        self.walk_expr(ex);
                    }
                }
            }
            NewExpression(n) => {
                self.walk_expr(&n.callee);
                for a in &n.arguments {
                    if let js::Argument::SpreadElement(s) = a {
                        self.walk_expr(&s.argument);
                    } else if let Some(ex) = a.as_expression() {
                        self.walk_expr(ex);
                    }
                }
            }
            StaticMemberExpression(m) => self.walk_expr(&m.object),
            ComputedMemberExpression(m) => {
                self.walk_expr(&m.object);
                self.walk_expr(&m.expression);
            }
            PrivateFieldExpression(p) => self.walk_expr(&p.object),
            FunctionExpression(f) => self.walk_function(f),
            ArrowFunctionExpression(a) => self.walk_arrow(a),
            ClassExpression(c) => self.walk_class(c),
            YieldExpression(y) => {
                if let Some(arg) = &y.argument {
                    self.walk_expr(arg);
                }
            }
            AwaitExpression(a) => self.walk_expr(&a.argument),
            SequenceExpression(s) => {
                for e in &s.expressions {
                    self.walk_expr(e);
                }
            }
            TaggedTemplateExpression(t) => {
                self.walk_expr(&t.tag);
                for ex in &t.quasi.expressions {
                    self.walk_expr(ex);
                }
            }
            ChainExpression(c) => {
                // Optional chain — walk the inner expression.
                match &c.expression {
                    js::ChainElement::CallExpression(ce) => {
                        self.walk_expr(&ce.callee);
                        for a in &ce.arguments {
                            if let Some(ex) = a.as_expression() {
                                self.walk_expr(ex);
                            }
                        }
                    }
                    js::ChainElement::ComputedMemberExpression(m) => {
                        self.walk_expr(&m.object);
                        self.walk_expr(&m.expression);
                    }
                    js::ChainElement::StaticMemberExpression(m) => {
                        self.walk_expr(&m.object);
                    }
                    js::ChainElement::PrivateFieldExpression(p) => self.walk_expr(&p.object),
                    _ => {}
                }
            }
            ImportExpression(i) => {
                self.walk_expr(&i.source);
                for arg in &i.arguments {
                    self.walk_expr(arg);
                }
            }
            // Fallthrough for anything we missed (TS-only kinds, etc).
            _ => {}
        }
    }

    /// For-statement init that's an Expression (not a VariableDeclaration).
    fn walk_expr_init(&mut self, init: &js::ForStatementInit) {
        if let Some(e) = init.as_expression() {
            self.walk_expr(e);
        }
    }

    fn walk_assignment_target(&mut self, t: &js::AssignmentTarget) {
        use js::AssignmentTarget::*;
        match t {
            AssignmentTargetIdentifier(id) => self.use_name(&id.name),
            StaticMemberExpression(m) => self.walk_expr(&m.object),
            ComputedMemberExpression(m) => {
                self.walk_expr(&m.object);
                self.walk_expr(&m.expression);
            }
            // Destructuring assignment patterns: walk to find both
            // reads (computed keys, defaults) and the LHS targets
            // (recursively assignment targets, never bindings here).
            // Conservative: walk via children to surface free reads.
            ArrayAssignmentTarget(a) => {
                for t in a.elements.iter().flatten() {
                    match t {
                        js::AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(wd) => {
                            self.walk_assignment_target(&wd.binding);
                            self.walk_expr(&wd.init);
                        }
                        other => {
                            if let Some(at) = other.as_assignment_target() {
                                self.walk_assignment_target(at);
                            }
                        }
                    }
                }
                if let Some(rest) = &a.rest {
                    self.walk_assignment_target(&rest.target);
                }
            }
            ObjectAssignmentTarget(o) => {
                for p in &o.properties {
                    match p {
                        js::AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(pi) => {
                            // `({ x } = ...)` — `x` is both target and read.
                            self.use_name(&pi.binding.name);
                            if let Some(init) = &pi.init {
                                self.walk_expr(init);
                            }
                        }
                        js::AssignmentTargetProperty::AssignmentTargetPropertyProperty(pp) => {
                            self.walk_property_key(&pp.name);
                            match &pp.binding {
                                js::AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(
                                    wd,
                                ) => {
                                    self.walk_assignment_target(&wd.binding);
                                    self.walk_expr(&wd.init);
                                }
                                other => {
                                    if let Some(at) = other.as_assignment_target() {
                                        self.walk_assignment_target(at);
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(rest) = &o.rest {
                    self.walk_assignment_target(&rest.target);
                }
            }
            _ => {}
        }
    }

    /// Collect every name introduced by a `let`/`const` (or inner
    /// pattern) into the *current* (block) scope. Walks default
    /// expressions for free reads as it goes.
    fn collect_pattern_bindings_block_scope(&mut self, p: &js::BindingPattern) {
        self.collect_pattern_bindings(p, /*function_scope=*/ false);
    }
    /// Same but binds into the function scope (for `var`).
    fn collect_pattern_bindings_function_scope(&mut self, p: &js::BindingPattern) {
        self.collect_pattern_bindings(p, /*function_scope=*/ true);
    }
    fn collect_pattern_bindings(&mut self, p: &js::BindingPattern, function_scope: bool) {
        match &p.kind {
            js::BindingPatternKind::BindingIdentifier(id) => {
                if function_scope {
                    self.bind_function_scope(&id.name);
                } else {
                    self.bind(&id.name);
                }
            }
            js::BindingPatternKind::ObjectPattern(o) => {
                for p in &o.properties {
                    self.walk_property_key(&p.key);
                    self.collect_pattern_bindings(&p.value, function_scope);
                }
                if let Some(rest) = &o.rest {
                    self.collect_pattern_bindings(&rest.argument, function_scope);
                }
            }
            js::BindingPatternKind::ArrayPattern(a) => {
                for p in a.elements.iter().flatten() {
                    self.collect_pattern_bindings(p, function_scope);
                }
                if let Some(rest) = &a.rest {
                    self.collect_pattern_bindings(&rest.argument, function_scope);
                }
            }
            js::BindingPatternKind::AssignmentPattern(a) => {
                // `{ x = default }` — walk the default expr (may
                // reference free names) and recurse into the inner
                // pattern.
                self.walk_expr(&a.right);
                self.collect_pattern_bindings(&a.left, function_scope);
            }
        }
    }
}
