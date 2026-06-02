//! Proxy-operator inlining.
//!
//! Obfuscators replace operators with tiny wrapper functions —
//! `function p(a, b) { return a === b; }` — then call them thousands of times
//! (`p(typeof x, "undefined")`). We detect such pure proxies and substitute the
//! call with the body, replacing parameters by the argument expressions. The
//! function then becomes unused and DCE removes it.
//!
//! We also handle the *object-method* form of the same trick, where the proxies
//! are properties of one table object — `var O = { wDLfP: function(a,b){return
//! b != a }, ... }` — and called as `O.wDLfP(x, y)`. Each property body is held
//! to the identical purity grammar, and the method receiver `this` is never used
//! (the grammar forbids it), so `O.m(x,y)` is equivalent to substituting the
//! body. These tables drive the operator-virtualised VM decoders (Cloudflare,
//! PerimeterX) and account for the bulk of remaining `O.x(a,b)` call noise.
//!
//! Soundness:
//!  * Proxy body is a *pure* expression over the parameters only (no free
//!    locals, no `this`/`arguments`, no calls/assignments) — so substitution is
//!    semantics-preserving.
//!  * If a parameter is used more than once, its argument must be side-effect
//!    free (literal/identifier), so we never duplicate or reorder effects.
//!  * The proxy name — the function name, or the table object's name — must be
//!    globally unique AND never reassigned/mutated, so a call always binds to
//!    the proxy we analyzed and never to a rebound value.
//!  * Operator precedence is preserved automatically: we splice AST nodes and
//!    the printer re-parenthesizes by precedence.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::{
    BindingPattern, Expression, FormalParameters, FunctionBody, Program, Statement,
};
use oxc_ast_visit::Visit;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct ProxyInline {
    changed: bool,
}

impl Pass for ProxyInline {
    fn name(&self) -> &'static str {
        "proxy-inline"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let proxies = collect_proxies(program, allocator);
        if proxies.is_empty() {
            return false;
        }
        let mut inl = Inliner { proxies, changed: false };
        let scoping = build_scoping(program);
        traverse_mut(&mut inl, allocator, program, scoping, ());
        self.changed = inl.changed;
        self.changed
    }
}

struct Proxy<'a> {
    params: Vec<String>,
    multi_use: HashSet<String>, // params referenced more than once in the body
    body: Expression<'a>,       // owned clone of the return expression
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

fn collect_proxies<'a>(program: &Program<'a>, allocator: &'a Allocator) -> HashMap<String, Proxy<'a>> {
    let mut c = ProxyCollector {
        allocator,
        counts: HashMap::new(),
        mutated: HashSet::new(),
        candidates: Vec::new(),
    };
    c.visit_program(program);

    // Each candidate carries the *guard* name that must be unique and unmutated
    // for the inline to be sound: the function's own name for a standalone
    // proxy, or the table object's name for an object-method proxy.
    let mut map: HashMap<String, Proxy<'a>> = HashMap::new();
    let mut guard_of: HashMap<String, String> = HashMap::new();
    let mut dup: HashSet<String> = HashSet::new();
    for (key, guard, proxy) in c.candidates {
        if map.contains_key(&key) {
            dup.insert(key.clone());
        }
        guard_of.insert(key.clone(), guard);
        map.insert(key, proxy);
    }
    map.retain(|key, _| {
        let guard = &guard_of[key];
        c.counts.get(guard).copied() == Some(1) && !dup.contains(key) && !c.mutated.contains(guard)
    });
    map
}

struct ProxyCollector<'a> {
    allocator: &'a Allocator,
    counts: HashMap<String, usize>,
    /// Names that are reassigned or have a member/element written or deleted —
    /// unsafe to treat as a fixed proxy.
    mutated: HashSet<String>,
    /// (key, guard-name, proxy). Key is the call form: `"name"` for a standalone
    /// proxy, `"obj.prop"` for an object-method proxy.
    candidates: Vec<(String, String, Proxy<'a>)>,
}

impl<'a> ProxyCollector<'a> {
    /// Build a `Proxy` from a parameter list and a single-return body, if it fits
    /// the pure-operator grammar. `key`/`guard` identify the call form and the
    /// name that must stay unique+unmutated.
    fn try_record(
        &mut self,
        key: String,
        guard: String,
        params: &FormalParameters<'a>,
        body: &FunctionBody<'a>,
        arrow_expr: bool,
    ) {
        let Some(param_names) = simple_param_names(params) else {
            return;
        };
        let Some(ret) = single_return_expr(body, arrow_expr) else {
            return;
        };
        let set: HashSet<&str> = param_names.iter().map(String::as_str).collect();
        if !body_supported(ret, &set) {
            return;
        }
        let mut counts: HashMap<String, usize> = HashMap::new();
        count_idents(ret, &mut counts);
        let multi_use = param_names
            .iter()
            .filter(|p| counts.get(*p).copied().unwrap_or(0) > 1)
            .cloned()
            .collect();
        self.candidates.push((
            key,
            guard,
            Proxy { params: param_names, multi_use, body: ret.clone_in(self.allocator) },
        ));
    }

    /// Record each operator-proxy method of an object-literal table bound to
    /// `obj`: `obj = { prop: function(a,b){ return <pure expr> }, ... }`.
    fn try_record_object(&mut self, obj: &str, object: &oxc_ast::ast::ObjectExpression<'a>) {
        use oxc_ast::ast::ObjectPropertyKind;
        for prop in &object.properties {
            let ObjectPropertyKind::ObjectProperty(p) = prop else {
                continue;
            };
            // Only plain data/method properties with a static name; never a
            // getter/setter (kind != Init) or a computed key.
            if p.computed || p.kind != oxc_ast::ast::PropertyKind::Init {
                continue;
            }
            let Some(prop_name) = static_key_name(&p.key) else {
                continue;
            };
            let key = format!("{obj}.{prop_name}");
            match &p.value {
                Expression::FunctionExpression(f) if f.id.is_none() => {
                    if let Some(body) = &f.body {
                        self.try_record(key, obj.to_string(), &f.params, body, false);
                    }
                }
                Expression::ArrowFunctionExpression(f) => {
                    self.try_record(key, obj.to_string(), &f.params, &f.body, f.expression);
                }
                _ => {}
            }
        }
    }
}

/// The textual name of a non-computed property key, if it is a plain identifier
/// or string literal.
fn static_key_name(key: &oxc_ast::ast::PropertyKey<'_>) -> Option<String> {
    use oxc_ast::ast::PropertyKey as K;
    match key {
        K::StaticIdentifier(id) => Some(id.name.to_string()),
        K::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

impl<'a> Visit<'a> for ProxyCollector<'a> {
    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        *self.counts.entry(it.name.to_string()).or_default() += 1;
    }

    fn visit_function(&mut self, func: &oxc_ast::ast::Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if let (Some(id), Some(body)) = (&func.id, &func.body) {
            let name = id.name.to_string();
            self.try_record(name.clone(), name, &func.params, body, false);
        }
        oxc_ast_visit::walk::walk_function(self, func, flags);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            let name = id.name.to_string();
            match &node.init {
                Some(Expression::FunctionExpression(f)) if f.id.is_none() => {
                    if let Some(body) = &f.body {
                        self.try_record(name.clone(), name, &f.params, body, false);
                    }
                }
                Some(Expression::ArrowFunctionExpression(f)) => {
                    self.try_record(name.clone(), name, &f.params, &f.body, f.expression);
                }
                Some(Expression::ObjectExpression(obj)) => {
                    self.try_record_object(&name, obj);
                }
                _ => {}
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }

    /// Track writes that would invalidate a proxy: `name = ...`, `name.x = ...`,
    /// `name[i] = ...`, and `name op= ...`. The mutated name is then never
    /// treated as a fixed proxy/table.
    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        use oxc_ast::ast::AssignmentTarget as T;
        match &node.left {
            T::AssignmentTargetIdentifier(id) => {
                self.mutated.insert(id.name.to_string());
            }
            T::StaticMemberExpression(m) => {
                if let Expression::Identifier(obj) = &m.object {
                    self.mutated.insert(obj.name.to_string());
                }
            }
            T::ComputedMemberExpression(m) => {
                if let Expression::Identifier(obj) = &m.object {
                    self.mutated.insert(obj.name.to_string());
                }
            }
            _ => {}
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }
}

fn simple_param_names(params: &FormalParameters<'_>) -> Option<Vec<String>> {
    if params.rest.is_some() {
        return None;
    }
    let mut out = Vec::new();
    for item in &params.items {
        let BindingPattern::BindingIdentifier(id) = &item.pattern else {
            return None;
        };
        out.push(id.name.to_string());
    }
    Some(out)
}

fn single_return_expr<'b, 'a>(body: &'b FunctionBody<'a>, arrow_expr: bool) -> Option<&'b Expression<'a>> {
    if body.statements.len() != 1 {
        return None;
    }
    match &body.statements[0] {
        Statement::ReturnStatement(r) => r.argument.as_ref(),
        Statement::ExpressionStatement(e) if arrow_expr => Some(&e.expression),
        _ => None,
    }
}

/// Body grammar we can substitute soundly: pure operator/value expressions whose
/// only identifiers are parameters.
fn body_supported(expr: &Expression<'_>, params: &HashSet<&str>) -> bool {
    match expr {
        Expression::Identifier(id) => params.contains(id.name.as_str()),
        Expression::NumericLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_) => true,
        Expression::UnaryExpression(u) => body_supported(&u.argument, params),
        Expression::BinaryExpression(b) => {
            body_supported(&b.left, params) && body_supported(&b.right, params)
        }
        Expression::LogicalExpression(l) => {
            body_supported(&l.left, params) && body_supported(&l.right, params)
        }
        Expression::ConditionalExpression(c) => {
            body_supported(&c.test, params)
                && body_supported(&c.consequent, params)
                && body_supported(&c.alternate, params)
        }
        Expression::StaticMemberExpression(m) => body_supported(&m.object, params),
        Expression::ComputedMemberExpression(m) => {
            body_supported(&m.object, params) && body_supported(&m.expression, params)
        }
        _ => false,
    }
}

fn count_idents(expr: &Expression<'_>, acc: &mut HashMap<String, usize>) {
    match expr {
        Expression::Identifier(id) => *acc.entry(id.name.to_string()).or_default() += 1,
        Expression::UnaryExpression(u) => count_idents(&u.argument, acc),
        Expression::BinaryExpression(b) => {
            count_idents(&b.left, acc);
            count_idents(&b.right, acc);
        }
        Expression::LogicalExpression(l) => {
            count_idents(&l.left, acc);
            count_idents(&l.right, acc);
        }
        Expression::ConditionalExpression(c) => {
            count_idents(&c.test, acc);
            count_idents(&c.consequent, acc);
            count_idents(&c.alternate, acc);
        }
        Expression::StaticMemberExpression(m) => count_idents(&m.object, acc),
        Expression::ComputedMemberExpression(m) => {
            count_idents(&m.object, acc);
            count_idents(&m.expression, acc);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Inlining
// ---------------------------------------------------------------------------

struct Inliner<'a> {
    proxies: HashMap<String, Proxy<'a>>,
    changed: bool,
}

impl<'a> Traverse<'a, ()> for Inliner<'a> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let replacement: Option<Expression<'a>> = {
            let Expression::CallExpression(call) = &*expr else {
                return;
            };
            // Callee key: `name` for a standalone proxy, `obj.prop` for an
            // object-method proxy. A computed `obj[k]()` is not matched (the key
            // would have to be a static string we can resolve).
            let key = match &call.callee {
                Expression::Identifier(callee) => callee.name.to_string(),
                Expression::StaticMemberExpression(m) => {
                    let Expression::Identifier(obj) = &m.object else {
                        return;
                    };
                    format!("{}.{}", obj.name, m.property.name)
                }
                _ => return,
            };
            let Some(proxy) = self.proxies.get(key.as_str()) else {
                return;
            };
            if call.arguments.len() != proxy.params.len() {
                return;
            }

            let mut args: HashMap<&str, &Expression<'a>> = HashMap::new();
            for (param, arg) in proxy.params.iter().zip(call.arguments.iter()) {
                let Some(ae) = arg.as_expression() else {
                    return; // spread argument
                };
                if proxy.multi_use.contains(param) && !is_side_effect_free(ae) {
                    return; // would duplicate a side effect
                }
                args.insert(param.as_str(), ae);
            }

            let mut cloned = proxy.body.clone_in(ctx.ast.allocator);
            subst(&mut cloned, &args, ctx.ast.allocator);
            Some(cloned)
        };

        if let Some(r) = replacement {
            *expr = r;
            self.changed = true;
        }
    }
}

fn is_side_effect_free(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::Identifier(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
    )
}

/// Replace parameter identifiers in `expr` (a clone of a proxy body) with clones
/// of the corresponding argument expressions.
fn subst<'a>(expr: &mut Expression<'a>, args: &HashMap<&str, &Expression<'a>>, alloc: &'a Allocator) {
    match expr {
        Expression::Identifier(id) => {
            if let Some(arg) = args.get(id.name.as_str()) {
                *expr = (*arg).clone_in(alloc);
            }
        }
        Expression::UnaryExpression(u) => subst(&mut u.argument, args, alloc),
        Expression::BinaryExpression(b) => {
            subst(&mut b.left, args, alloc);
            subst(&mut b.right, args, alloc);
        }
        Expression::LogicalExpression(l) => {
            subst(&mut l.left, args, alloc);
            subst(&mut l.right, args, alloc);
        }
        Expression::ConditionalExpression(c) => {
            subst(&mut c.test, args, alloc);
            subst(&mut c.consequent, args, alloc);
            subst(&mut c.alternate, args, alloc);
        }
        Expression::StaticMemberExpression(m) => subst(&mut m.object, args, alloc),
        Expression::ComputedMemberExpression(m) => {
            subst(&mut m.object, args, alloc);
            subst(&mut m.expression, args, alloc);
        }
        _ => {}
    }
}
