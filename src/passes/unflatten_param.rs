//! Param-state control-flow unflattening (interprocedural).
//!
//! The dominant flattening shape in samples 1 & 4 puts the dispatch state in a
//! *function parameter* and selects the entry point at the call site:
//! ```text
//! var f = function self(s, p) { while (s != 105) { switch (s) { … s = …; break; } } };
//! f(56, args);          // external entry 56
//! …self(2, other)…      // recursive entry 2
//! ```
//! The plain (statement-list) unflattener can't touch this: there is no local
//! `var s = <int>` initializer — the entry is whatever the caller passes.
//!
//! We *specialize* such a function per distinct constant entry. For each entry `C`
//! we simulate the dispatch machine from `C` (reusing the engine in `unflatten`)
//! and emit `function __pf_<f>_<C>(p) { <linearized body> }` — the slot's first
//! parameter (the dispatch state) dropped — then rewrite every `f(C, args)` to
//! `__pf_<f>_<C>(args)`. Recursive calls inside the body rewrite the same way, so
//! a self-recursive machine stays correct. The original `f` is left for DCE.
//!
//! Soundness gates (all required, else the function is left untouched):
//!   * the dispatch loop is the *first* statement; parameter 0 is dispatch-only
//!     (`parse_cases` rejects any case body that references it) and any trailing
//!     statements after the loop never reference it;
//!   * params are plain identifiers (no rest/pattern/default) and the body does
//!     not use `arguments` (dropping param 0 must not shift `arguments` indices);
//!   * every binding name of the function (the outer `var f` and any inner
//!     function-expression name) is **globally unique**, and *every* reference to
//!     those names is the callee of a call whose first argument is an integer
//!     literal — so the function never escapes and every entry is a known constant;
//!   * every distinct entry simulates to a finite linear sequence.
//!
//! Because param 0 is dispatch-only and each call passes a constant entry, the
//! specialized body runs exactly the statements the original would, in order, with
//! the remaining parameters bound identically (bare calls ⇒ `this` is unchanged).

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{
    BindingPattern, Expression, Function, FunctionType, Program, Statement, VariableDeclarator,
};
use oxc_ast::AstBuilder;
use oxc_ast_visit::Visit;
use oxc_span::SPAN;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use super::unflatten::{detect_loop, parse_cases, simulate, ExitMode};
use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct UnflattenParam {
    /// `(callee-name, entry-const)` → specialized function name. Filled by the
    /// build pass, consumed by the rewrite pass.
    plan: HashMap<(String, i64), String>,
}

impl Pass for UnflattenParam {
    fn name(&self) -> &'static str {
        "unflatten-param"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // 1. Whole-program census: binding/reference counts and call entries.
        let mut census = Census::default();
        census.visit_program(program);

        // 2. Build pass: find candidate functions, specialize per entry, record the
        //    call-rewrite plan. We insert via a hand-written recursion (NOT
        //    `traverse_mut`): inserting a freshly-built `Function` mid-traversal
        //    panics oxc's walker (the node has no `scope_id` yet). Our recursion
        //    descends into each list's *existing* children first, then inserts the
        //    specializations, which it never re-visits.
        self.plan.clear();
        let existing = census.binding_names();
        let builder = Builder {
            ast: AstBuilder::new(allocator),
            census: &census,
            existing: &existing,
        };
        builder.specialize_lists(&mut program.body, &mut self.plan);
        if self.plan.is_empty() {
            return false;
        }

        // 3. Rewrite pass: route every constant-entry call to its specialization.
        //    In-place name mutation only (no new nodes) so the walker is safe; the
        //    stale reference id is re-resolved when the next pass builds scoping.
        let mut rewriter = Rewriter {
            ast: AstBuilder::new(allocator),
            plan: &self.plan,
            changed: false,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut rewriter, allocator, program, scoping, ());
        rewriter.changed
    }
}

// ---------------------------------------------------------------------------
// Census
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Census {
    bindings: HashMap<String, usize>,
    refs: HashMap<String, usize>,
    /// Per callee name: one entry per call, `Some(C)` for a constant integer first
    /// argument, `None` otherwise (or no args).
    calls: HashMap<String, Vec<Option<i64>>>,
}

impl Census {
    fn binding_names(&self) -> HashSet<String> {
        self.bindings.keys().cloned().collect()
    }
}

impl<'a> Visit<'a> for Census {
    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        *self.bindings.entry(it.name.to_string()).or_default() += 1;
    }
    fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
        *self.refs.entry(it.name.to_string()).or_default() += 1;
    }
    fn visit_call_expression(&mut self, node: &oxc_ast::ast::CallExpression<'a>) {
        if let Expression::Identifier(callee) = &node.callee {
            let entry = node
                .arguments
                .first()
                .and_then(|a| a.as_expression())
                .and_then(int_literal);
            self.calls
                .entry(callee.name.to_string())
                .or_default()
                .push(entry);
        }
        oxc_ast_visit::walk::walk_call_expression(self, node);
    }
}

/// A non-negative or negative integer-valued numeric literal.
fn int_literal(expr: &Expression<'_>) -> Option<i64> {
    match expr {
        Expression::NumericLiteral(n) if n.value.fract() == 0.0 && n.value.is_finite() => {
            Some(n.value as i64)
        }
        Expression::UnaryExpression(u)
            if matches!(
                u.operator,
                oxc_syntax::operator::UnaryOperator::UnaryNegation
            ) =>
        {
            int_literal(&u.argument).map(|v| -v)
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Build pass
// ---------------------------------------------------------------------------

struct Builder<'a, 'c> {
    ast: AstBuilder<'a>,
    census: &'c Census,
    existing: &'c HashSet<String>,
}

/// A function that is a param-state dispatcher: its binding names and the
/// `Function` node itself.
struct Candidate<'b, 'a> {
    func: &'b Function<'a>,
    /// All globally-visible binding names of the function (outer var + inner name).
    names: Vec<String>,
}

impl<'a, 'c> Builder<'a, 'c> {
    /// Recurse a statement list: descend into existing children first, then insert
    /// each candidate's specializations right after it. The inserted nodes are
    /// never re-visited.
    fn specialize_lists(
        &self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        plan: &mut HashMap<(String, i64), String>,
    ) {
        let old = std::mem::replace(stmts, self.ast.vec());
        for mut stmt in old {
            self.recurse_stmt(&mut stmt, plan);
            let specials = self.specials_for(&stmt, plan);
            stmts.push(stmt);
            for sp in specials {
                stmts.push(sp);
            }
        }
    }

    /// Build the specializations for `stmt` if it binds a valid param-state
    /// dispatcher, recording the rewrite plan. Empty otherwise.
    fn specials_for(
        &self,
        stmt: &Statement<'a>,
        plan: &mut HashMap<(String, i64), String>,
    ) -> Vec<Statement<'a>> {
        let Some(cand) = candidate(stmt) else {
            return Vec::new();
        };
        let Some(entries) = self.validate(&cand) else {
            return Vec::new();
        };
        let outer = &cand.names[0];
        let mut specials: Vec<Statement<'a>> = Vec::new();
        let mut plan_adds: Vec<((String, i64), String)> = Vec::new();
        for &c in &entries {
            let spec_name = special_name(outer, c);
            // Bail the whole function if any entry collides or fails to simulate —
            // a partial plan would leave a recursive call routed to a missing name.
            if self.existing.contains(&spec_name) {
                return Vec::new();
            }
            let Some(func) = self.specialize(&cand, c, &spec_name) else {
                return Vec::new();
            };
            specials.push(Statement::FunctionDeclaration(func));
            for name in &cand.names {
                plan_adds.push(((name.clone(), c), spec_name.clone()));
            }
        }
        for (k, v) in plan_adds {
            plan.insert(k, v);
        }
        specials
    }

    /// Descend into every statement list reachable from `stmt` without crossing
    /// into a freshly-inserted node. Best-effort: containers not handled here are
    /// simply not searched (sound — just no transform there).
    fn recurse_stmt(&self, stmt: &mut Statement<'a>, plan: &mut HashMap<(String, i64), String>) {
        match stmt {
            Statement::BlockStatement(b) => self.specialize_lists(&mut b.body, plan),
            Statement::FunctionDeclaration(f) => {
                if let Some(body) = &mut f.body {
                    self.specialize_lists(&mut body.statements, plan);
                }
            }
            Statement::VariableDeclaration(vd) => {
                for d in &mut vd.declarations {
                    if let Some(init) = &mut d.init {
                        self.recurse_expr_fn(init, plan);
                    }
                }
            }
            Statement::ExpressionStatement(es) => self.recurse_expr_fn(&mut es.expression, plan),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &mut r.argument {
                    self.recurse_expr_fn(arg, plan);
                }
            }
            Statement::IfStatement(i) => {
                self.recurse_stmt(&mut i.consequent, plan);
                if let Some(alt) = &mut i.alternate {
                    self.recurse_stmt(alt, plan);
                }
            }
            Statement::ForStatement(f) => self.recurse_stmt(&mut f.body, plan),
            Statement::ForInStatement(f) => self.recurse_stmt(&mut f.body, plan),
            Statement::ForOfStatement(f) => self.recurse_stmt(&mut f.body, plan),
            Statement::WhileStatement(w) => self.recurse_stmt(&mut w.body, plan),
            Statement::DoWhileStatement(d) => self.recurse_stmt(&mut d.body, plan),
            Statement::LabeledStatement(l) => self.recurse_stmt(&mut l.body, plan),
            Statement::TryStatement(t) => {
                self.specialize_lists(&mut t.block.body, plan);
                if let Some(h) = &mut t.handler {
                    self.specialize_lists(&mut h.body.body, plan);
                }
                if let Some(f) = &mut t.finalizer {
                    self.specialize_lists(&mut f.body, plan);
                }
            }
            Statement::SwitchStatement(sw) => {
                for case in &mut sw.cases {
                    self.specialize_lists(&mut case.consequent, plan);
                }
            }
            _ => {}
        }
    }

    /// Recurse into function bodies embedded in an expression (IIFEs, `var f =
    /// function(){…}`, `~(function(){…})()`, comma/assignment chains).
    fn recurse_expr_fn(
        &self,
        expr: &mut Expression<'a>,
        plan: &mut HashMap<(String, i64), String>,
    ) {
        match expr {
            Expression::FunctionExpression(f) => {
                if let Some(body) = &mut f.body {
                    self.specialize_lists(&mut body.statements, plan);
                }
            }
            Expression::ArrowFunctionExpression(f) => {
                self.specialize_lists(&mut f.body.statements, plan);
            }
            Expression::CallExpression(c) => {
                self.recurse_expr_fn(&mut c.callee, plan);
                for arg in &mut c.arguments {
                    if let Some(e) = arg.as_expression_mut() {
                        self.recurse_expr_fn(e, plan);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => self.recurse_expr_fn(&mut p.expression, plan),
            Expression::UnaryExpression(u) => self.recurse_expr_fn(&mut u.argument, plan),
            Expression::SequenceExpression(s) => {
                for e in &mut s.expressions {
                    self.recurse_expr_fn(e, plan);
                }
            }
            Expression::AssignmentExpression(a) => self.recurse_expr_fn(&mut a.right, plan),
            _ => {}
        }
    }

    /// Validate a candidate against the census and return its distinct entries
    /// (sorted, deduped). `None` if any gate fails.
    fn validate(&self, cand: &Candidate<'_, '_>) -> Option<Vec<i64>> {
        let mut consts: HashSet<i64> = HashSet::new();
        for name in &cand.names {
            // Unique binding.
            if self.census.bindings.get(name).copied() != Some(1) {
                return None;
            }
            let calls = self
                .census
                .calls
                .get(name)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let refs = self.census.refs.get(name).copied().unwrap_or(0);
            // Every reference must be a call callee (callee refs == total refs)…
            if refs != calls.len() {
                return None;
            }
            // …and every such call must pass a constant integer entry.
            for entry in calls {
                match entry {
                    Some(c) => {
                        consts.insert(*c);
                    }
                    None => return None,
                }
            }
        }
        if consts.is_empty() {
            return None; // never called → nothing to do
        }
        let mut out: Vec<i64> = consts.into_iter().collect();
        out.sort_unstable();
        Some(out)
    }

    /// Build the specialized function for entry `c`. Clones the original, drops
    /// param 0, renames, and replaces the body with the simulated linear sequence.
    fn specialize(
        &self,
        cand: &Candidate<'_, 'a>,
        c: i64,
        spec_name: &str,
    ) -> Option<oxc_allocator::Box<'a, Function<'a>>> {
        let alloc = self.ast.allocator;
        // Re-derive the dispatch machine from the (still-borrowed) original body.
        let body = cand.func.body.as_ref()?;
        let loop_stmt = body.statements.first()?;
        let state = param0_name(cand.func)?;
        let (switch, kind, test) = detect_loop(loop_stmt, &state)?;
        let cases = parse_cases(switch, &state)?;
        let (mut seq, exit) = simulate(c, &cases, &test, kind, alloc)?;
        // Statements after the loop run only when the machine exits via the loop
        // test; a terminal (`return`/`throw`) case makes them unreachable.
        if exit == ExitMode::Test {
            for st in body.statements.iter().skip(1) {
                seq.push(st.clone_in(alloc));
            }
        }

        let mut func: Function<'a> = cand.func.clone_in(alloc);
        func.r#type = FunctionType::FunctionDeclaration;
        func.id = Some(self.ast.binding_identifier(SPAN, self.ast.str(spec_name)));

        // Drop parameter 0.
        let old_items = std::mem::replace(&mut func.params.items, self.ast.vec());
        let mut items = self.ast.vec();
        for (i, p) in old_items.into_iter().enumerate() {
            if i != 0 {
                items.push(p);
            }
        }
        func.params.items = items;

        // Replace the body with the linearized sequence.
        if let Some(fb) = &mut func.body {
            let mut stmts = self.ast.vec();
            for st in seq {
                stmts.push(st);
            }
            fb.statements = stmts;
        } else {
            return None;
        }
        Some(oxc_allocator::Box::new_in(func, alloc))
    }
}

/// A statement that binds a param-state dispatcher function, if any. Handles a
/// `function f(s, …) {…}` declaration and a `var f = function [name](s, …) {…}`.
fn candidate<'b, 'a>(stmt: &'b Statement<'a>) -> Option<Candidate<'b, 'a>> {
    match stmt {
        Statement::FunctionDeclaration(f) => {
            let id = f.id.as_ref()?;
            let cand = Candidate {
                func: f,
                names: vec![id.name.to_string()],
            };
            shape_ok(&cand).then_some(cand)
        }
        Statement::VariableDeclaration(vd) if vd.declarations.len() == 1 => {
            let d: &VariableDeclarator = &vd.declarations[0];
            let BindingPattern::BindingIdentifier(outer) = &d.id else {
                return None;
            };
            let Some(Expression::FunctionExpression(f)) = &d.init else {
                return None;
            };
            let mut names = vec![outer.name.to_string()];
            if let Some(inner) = &f.id {
                names.push(inner.name.to_string());
            }
            let cand = Candidate { func: f, names };
            shape_ok(&cand).then_some(cand)
        }
        _ => None,
    }
}

/// Structural gate: the body is exactly the dispatch loop over a plain first
/// parameter, params are simple, and the body does not use `arguments`.
fn shape_ok(cand: &Candidate<'_, '_>) -> bool {
    let f = cand.func;
    if f.params.rest.is_some() || f.params.items.is_empty() {
        return false;
    }
    // All params must be plain identifiers (no patterns/defaults).
    for p in &f.params.items {
        if !matches!(p.pattern, BindingPattern::BindingIdentifier(_)) {
            return false;
        }
    }
    let Some(state) = param0_name(f) else {
        return false;
    };
    let Some(body) = f.body.as_ref() else {
        return false;
    };
    if uses_arguments(body) {
        return false;
    }
    // The dispatch loop must be the *first* statement (no prefix that runs before
    // the machine). Statements after it (trailing, run on a test-exit) must not
    // reference the dispatch parameter — it is dropped in the specialization.
    let Some(loop_stmt) = body.statements.first() else {
        return false;
    };
    for trailing in body.statements.iter().skip(1) {
        if mentions_name(trailing, &state) {
            return false;
        }
    }
    let Some((switch, _, _)) = detect_loop(loop_stmt, &state) else {
        return false;
    };
    parse_cases(switch, &state).is_some()
}

/// Does `stmt` reference `name` (read or write) anywhere, not descending into
/// nested functions (which would have their own binding)?
fn mentions_name(stmt: &Statement<'_>, name: &str) -> bool {
    struct M<'r> {
        name: &'r str,
        found: bool,
    }
    impl<'a> Visit<'a> for M<'_> {
        fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
            if it.name == self.name {
                self.found = true;
            }
        }
        fn visit_function(&mut self, _f: &Function<'a>, _flags: oxc_syntax::scope::ScopeFlags) {}
    }
    let mut m = M { name, found: false };
    m.visit_statement(stmt);
    m.found
}

fn param0_name(f: &Function<'_>) -> Option<String> {
    match &f.params.items.first()?.pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
        _ => None,
    }
}

fn uses_arguments(body: &oxc_ast::ast::FunctionBody<'_>) -> bool {
    struct A {
        found: bool,
    }
    impl<'a> Visit<'a> for A {
        fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
            if it.name == "arguments" {
                self.found = true;
            }
        }
        // Do not descend into nested functions: their `arguments` is their own.
        fn visit_function(&mut self, _f: &Function<'a>, _flags: oxc_syntax::scope::ScopeFlags) {}
    }
    let mut a = A { found: false };
    a.visit_function_body(body);
    a.found
}

fn special_name(outer: &str, c: i64) -> String {
    if c < 0 {
        format!("__pf_{outer}_m{}", -c)
    } else {
        format!("__pf_{outer}_{c}")
    }
}

// ---------------------------------------------------------------------------
// Rewrite pass
// ---------------------------------------------------------------------------

struct Rewriter<'a, 'c> {
    ast: AstBuilder<'a>,
    plan: &'c HashMap<(String, i64), String>,
    changed: bool,
}

impl<'a, 'c> Traverse<'a, ()> for Rewriter<'a, 'c> {
    fn enter_call_expression(
        &mut self,
        node: &mut oxc_ast::ast::CallExpression<'a>,
        _ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let Expression::Identifier(callee) = &node.callee else {
            return;
        };
        let name = callee.name.to_string();
        let Some(entry) = node
            .arguments
            .first()
            .and_then(|a| a.as_expression())
            .and_then(int_literal)
        else {
            return;
        };
        let Some(spec) = self.plan.get(&(name, entry)) else {
            return;
        };
        let spec = self.ast.ident(spec);
        // Re-route to the specialization by renaming the callee *in place* (keeps
        // the node's ids so the walker stays happy) and drop the constant entry arg.
        if let Expression::Identifier(callee) = &mut node.callee {
            callee.name = spec;
        }
        let old = std::mem::replace(&mut node.arguments, self.ast.vec());
        let mut args = self.ast.vec();
        for (i, a) in old.into_iter().enumerate() {
            if i != 0 {
                args.push(a);
            }
        }
        node.arguments = args;
        self.changed = true;
    }
}
