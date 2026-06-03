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
    ArrayExpressionElement, BindingPattern, CallExpression, Expression, FormalParameters,
    FunctionBody, Program, Statement,
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
        let mut inl = Inliner {
            proxies,
            changed: false,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut inl, allocator, program, scoping, ());
        self.changed = inl.changed;
        self.changed
    }
}

struct Proxy<'a> {
    params: Vec<String>,
    multi_use: HashSet<String>, // params referenced more than once in the body
    /// Params that never appear in the body. Their argument is evaluated at the
    /// call but would vanish on substitution — so an impure one must not be
    /// dropped (`p((x = 1), 5)` with `p(a, b) { return b; }`).
    unused: HashSet<String>,
    /// Params that appear in *callee* position of a call within the body (apply
    /// proxies, `function(f, x){ return f(x); }`). Their argument must not be a
    /// member expression: `O.apply(obj.m, x)` would bind receiver `obj`, whereas
    /// the proxy detaches the function (`f = obj.m; f(x)` has `this`=undefined).
    callee_params: HashSet<String>,
    body: Expression<'a>, // owned clone of the return expression
}

// ---------------------------------------------------------------------------
// Collection
// ---------------------------------------------------------------------------

fn collect_proxies<'a>(
    program: &Program<'a>,
    allocator: &'a Allocator,
) -> HashMap<String, Proxy<'a>> {
    let mut c = ProxyCollector {
        allocator,
        counts: HashMap::new(),
        value_writes: HashMap::new(),
        member_mutated: HashSet::new(),
        candidates: Vec::new(),
    };
    c.visit_program(program);

    // Each candidate carries the *guard* name whose value must be fixed for the
    // inline to be sound: the function's own name for a standalone proxy, or the
    // table object's name for an object-method proxy. A guard is fixed iff it is
    // declared exactly once (unique name), its value is established by exactly
    // one write (the proxy/table we analyzed — a function decl, a declarator
    // initializer, or a single assignment), and it is never member-mutated.
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
        c.counts.get(guard).copied() == Some(1)
            && c.value_writes.get(guard).copied() == Some(1)
            && !c.member_mutated.contains(guard)
            && !dup.contains(key)
    });
    map
}

struct ProxyCollector<'a> {
    allocator: &'a Allocator,
    /// How many times each name is *declared* (binding identifiers). >1 means the
    /// name is not unique and a call site could bind to a different one.
    counts: HashMap<String, usize>,
    /// How many times each name's value is *established*: a function declaration,
    /// a declarator with an initializer, or an identifier-target assignment. The
    /// proxy is only fixed if this is exactly 1.
    value_writes: HashMap<String, usize>,
    /// Names whose member/element is written or deleted (`o.x = …`, `o[i] = …`,
    /// `delete o.x`) — the table is not immutable, so never inline through it.
    member_mutated: HashSet<String>,
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
        let unused = param_names
            .iter()
            .filter(|p| counts.get(*p).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        let mut callee_params = HashSet::new();
        collect_callee_params(ret, &set, &mut callee_params);
        self.candidates.push((
            key,
            guard,
            Proxy {
                params: param_names,
                multi_use,
                unused,
                callee_params,
                body: ret.clone_in(self.allocator),
            },
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

    fn visit_function(
        &mut self,
        func: &oxc_ast::ast::Function<'a>,
        flags: oxc_syntax::scope::ScopeFlags,
    ) {
        if let (Some(id), Some(body)) = (&func.id, &func.body) {
            let name = id.name.to_string();
            *self.value_writes.entry(name.clone()).or_default() += 1;
            self.try_record(name.clone(), name, &func.params, body, false);
        }
        oxc_ast_visit::walk::walk_function(self, func, flags);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            let name = id.name.to_string();
            if node.init.is_some() {
                *self.value_writes.entry(name.clone()).or_default() += 1;
            }
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

    /// Classify the target of every assignment: an identifier target is a value
    /// write (counted); a member/element target is a member mutation (the object
    /// is not immutable). A proxy/table assigned exactly once via `X = {…}` is
    /// also recorded here — that single value write is its definition.
    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        use oxc_ast::ast::AssignmentTarget as T;
        match &node.left {
            T::AssignmentTargetIdentifier(id) => {
                let name = id.name.to_string();
                *self.value_writes.entry(name.clone()).or_default() += 1;
                if let Expression::ObjectExpression(obj) = &node.right {
                    self.try_record_object(&name, obj);
                }
            }
            T::StaticMemberExpression(m) => {
                if let Expression::Identifier(obj) = &m.object {
                    self.member_mutated.insert(obj.name.to_string());
                }
            }
            T::ComputedMemberExpression(m) => {
                if let Expression::Identifier(obj) = &m.object {
                    self.member_mutated.insert(obj.name.to_string());
                }
            }
            _ => {}
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    /// `delete o.x` / `o.x++` style mutations also disqualify the object.
    fn visit_unary_expression(&mut self, node: &oxc_ast::ast::UnaryExpression<'a>) {
        if matches!(node.operator, oxc_syntax::operator::UnaryOperator::Delete) {
            if let Expression::StaticMemberExpression(m) = &node.argument {
                if let Expression::Identifier(obj) = &m.object {
                    self.member_mutated.insert(obj.name.to_string());
                }
            }
            if let Expression::ComputedMemberExpression(m) = &node.argument {
                if let Expression::Identifier(obj) = &m.object {
                    self.member_mutated.insert(obj.name.to_string());
                }
            }
        }
        oxc_ast_visit::walk::walk_unary_expression(self, node);
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

fn single_return_expr<'b, 'a>(
    body: &'b FunctionBody<'a>,
    arrow_expr: bool,
) -> Option<&'b Expression<'a>> {
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
        // Apply proxy: `f(x)`. Callee and every argument must be supported; no
        // spread (we can't expand it). Receiver-binding is handled at inline time
        // via `callee_params`.
        Expression::CallExpression(c) => {
            body_supported(&c.callee, params)
                && c.arguments.iter().all(|a| match a.as_expression() {
                    Some(e) => body_supported(e, params),
                    None => false,
                })
        }
        _ => false,
    }
}

/// Collect parameters used in callee position of a call anywhere in `body`.
fn collect_callee_params(expr: &Expression<'_>, params: &HashSet<&str>, out: &mut HashSet<String>) {
    match expr {
        Expression::CallExpression(c) => {
            if let Expression::Identifier(id) = &c.callee {
                if params.contains(id.name.as_str()) {
                    out.insert(id.name.to_string());
                }
            }
            collect_callee_params(&c.callee, params, out);
            for a in &c.arguments {
                if let Some(e) = a.as_expression() {
                    collect_callee_params(e, params, out);
                }
            }
        }
        Expression::UnaryExpression(u) => collect_callee_params(&u.argument, params, out),
        Expression::BinaryExpression(b) => {
            collect_callee_params(&b.left, params, out);
            collect_callee_params(&b.right, params, out);
        }
        Expression::LogicalExpression(l) => {
            collect_callee_params(&l.left, params, out);
            collect_callee_params(&l.right, params, out);
        }
        Expression::ConditionalExpression(c) => {
            collect_callee_params(&c.test, params, out);
            collect_callee_params(&c.consequent, params, out);
            collect_callee_params(&c.alternate, params, out);
        }
        Expression::StaticMemberExpression(m) => collect_callee_params(&m.object, params, out),
        Expression::ComputedMemberExpression(m) => {
            collect_callee_params(&m.object, params, out);
            collect_callee_params(&m.expression, params, out);
        }
        _ => {}
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
        Expression::CallExpression(c) => {
            count_idents(&c.callee, acc);
            for a in &c.arguments {
                if let Some(e) = a.as_expression() {
                    count_idents(e, acc);
                }
            }
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
            // Resolve the proxy key and the *effective* argument expressions for
            // the three call shapes: `P(args)`, `P.call(this, args)`, and
            // `P.apply(this, [args])`. The proxy body never uses `this`, so the
            // receiver passed to .call/.apply is irrelevant — but it is still
            // evaluated, so a non-pure receiver must not be dropped.
            let Some((key, arg_exprs)) = resolve_proxy_call(call, &self.proxies) else {
                return;
            };
            let Some(proxy) = self.proxies.get(key.as_str()) else {
                return;
            };
            if arg_exprs.len() != proxy.params.len() {
                return;
            }

            // Inlining substitutes args into the body, which may reference params
            // in a different order than the call (`sub(a,b){return b-a}`). Pure
            // and member-read args reorder harmlessly, but two args that each
            // carry a *strong* effect (a call/assignment/new/update) could have
            // their effects swapped. Decline when more than one such arg exists.
            if arg_exprs
                .iter()
                .filter(|ae| has_strong_side_effect(ae))
                .count()
                > 1
            {
                return;
            }

            let mut args: HashMap<&str, &Expression<'a>> = HashMap::new();
            for (param, ae) in proxy.params.iter().zip(arg_exprs.iter().copied()) {
                if proxy.multi_use.contains(param) && !is_side_effect_free(ae) {
                    return; // would duplicate a side effect
                }
                if proxy.unused.contains(param) && !is_side_effect_free(ae) {
                    return; // would drop the argument's side effect
                }
                // A cloned argument containing a function/class/arrow carries
                // scope cells that are reset to None by the clone; the traverse
                // walker unwraps a function's scope_id and would panic when it
                // descends into the spliced node. Such args are rare for operator
                // proxies, so just decline the inline (semantics unaffected).
                if contains_scoped_node(ae) {
                    return;
                }
                // Receiver guard: an apply proxy detaches its callee from any
                // receiver. Substituting a member expression into callee position
                // would re-bind `this`, changing semantics — decline.
                if proxy.callee_params.contains(param) && is_member_expr(ae) {
                    return;
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

/// The proxy-map key for a callee that is an identifier (`p`) or a static member
/// of an identifier (`obj.prop`). Other shapes have no key.
fn callee_key(expr: &Expression<'_>) -> Option<String> {
    match expr {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::StaticMemberExpression(m) => match &m.object {
            Expression::Identifier(obj) => Some(format!("{}.{}", obj.name, m.property.name)),
            _ => None,
        },
        _ => None,
    }
}

/// Resolve a call against the proxy table, returning the proxy key and the
/// effective argument expressions. Handles three shapes:
///   * `P(a, b)`               — direct call
///   * `P.call(this, a, b)`    — `this` evaluated then ignored by the body
///   * `P.apply(this, [a, b])` — args from a literal array
///
/// For `.call`/`.apply` the receiver must be side-effect-free (it is dropped).
fn resolve_proxy_call<'a, 'e>(
    call: &'e CallExpression<'a>,
    proxies: &HashMap<String, Proxy<'a>>,
) -> Option<(String, Vec<&'e Expression<'a>>)> {
    // Direct call form (also covers an object method literally named `call`).
    if let Some(key) = callee_key(&call.callee) {
        if proxies.contains_key(&key) {
            let mut out = Vec::with_capacity(call.arguments.len());
            for a in &call.arguments {
                out.push(a.as_expression()?);
            }
            return Some((key, out));
        }
    }

    // `.call` / `.apply` invocation of a proxy.
    let Expression::StaticMemberExpression(m) = &call.callee else {
        return None;
    };
    let method = m.property.name.as_str();
    if method != "call" && method != "apply" {
        return None;
    }
    let base = callee_key(&m.object)?;
    if !proxies.contains_key(&base) {
        return None;
    }
    // The receiver (first arg) is evaluated then discarded; only drop it if it
    // has no side effect. A missing receiver means a zero-argument call.
    let Some(first) = call.arguments.first() else {
        return Some((base, Vec::new()));
    };
    let this_arg = first.as_expression()?;
    if !is_side_effect_free(this_arg) {
        return None;
    }

    if method == "call" {
        let mut out = Vec::new();
        for a in call.arguments.iter().skip(1) {
            out.push(a.as_expression()?);
        }
        return Some((base, out));
    }

    // apply: the second argument must be a literal array with no holes/spreads.
    let Some(second) = call.arguments.get(1) else {
        return Some((base, Vec::new())); // `P.apply(this)` → no args
    };
    let Expression::ArrayExpression(arr) = second.as_expression()? else {
        return None;
    };
    let mut out = Vec::with_capacity(arr.elements.len());
    for el in &arr.elements {
        match el {
            ArrayExpressionElement::SpreadElement(_) | ArrayExpressionElement::Elision(_) => {
                return None;
            }
            other => out.push(other.as_expression()?),
        }
    }
    Some((base, out))
}

/// Whether `expr` contains a node that owns a scope (function, arrow, class).
/// Cloning such a node resets its scope_id cell to None, which the traverse
/// walker later unwraps — so we must not splice one mid-traversal.
fn contains_scoped_node(expr: &Expression<'_>) -> bool {
    struct Finder {
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(
            &mut self,
            _f: &oxc_ast::ast::Function<'a>,
            _flags: oxc_syntax::scope::ScopeFlags,
        ) {
            self.found = true; // do not recurse: one is enough
        }
        fn visit_arrow_function_expression(
            &mut self,
            _f: &oxc_ast::ast::ArrowFunctionExpression<'a>,
        ) {
            self.found = true;
        }
        fn visit_class(&mut self, _c: &oxc_ast::ast::Class<'a>) {
            self.found = true;
        }
    }
    let mut f = Finder { found: false };
    f.visit_expression(expr);
    f.found
}

fn is_member_expr(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::StaticMemberExpression(_) | Expression::ComputedMemberExpression(_)
    )
}

/// Whether `expr` contains a node whose evaluation has an observable, ordered
/// effect — a call, `new`, assignment, update (`++`/`--`), await, yield, or
/// tagged template. Bare member reads are deliberately excluded: the pass treats
/// property reads as reorderable (consistent with its other inlining). Used to
/// forbid swapping the effects of two effectful arguments.
fn has_strong_side_effect(expr: &Expression<'_>) -> bool {
    struct Finder {
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_call_expression(&mut self, _c: &oxc_ast::ast::CallExpression<'a>) {
            self.found = true;
        }
        fn visit_new_expression(&mut self, _n: &oxc_ast::ast::NewExpression<'a>) {
            self.found = true;
        }
        fn visit_assignment_expression(&mut self, _a: &oxc_ast::ast::AssignmentExpression<'a>) {
            self.found = true;
        }
        fn visit_update_expression(&mut self, _u: &oxc_ast::ast::UpdateExpression<'a>) {
            self.found = true;
        }
        fn visit_await_expression(&mut self, _a: &oxc_ast::ast::AwaitExpression<'a>) {
            self.found = true;
        }
        fn visit_yield_expression(&mut self, _y: &oxc_ast::ast::YieldExpression<'a>) {
            self.found = true;
        }
        fn visit_tagged_template_expression(
            &mut self,
            _t: &oxc_ast::ast::TaggedTemplateExpression<'a>,
        ) {
            self.found = true;
        }
    }
    let mut f = Finder { found: false };
    f.visit_expression(expr);
    f.found
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
fn subst<'a>(
    expr: &mut Expression<'a>,
    args: &HashMap<&str, &Expression<'a>>,
    alloc: &'a Allocator,
) {
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
        Expression::CallExpression(c) => {
            subst(&mut c.callee, args, alloc);
            for a in c.arguments.iter_mut() {
                if let Some(e) = a.as_expression_mut() {
                    subst(e, args, alloc);
                }
            }
        }
        _ => {}
    }
}
