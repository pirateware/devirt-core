//! Decoder lifting via sandboxed partial evaluation.
//!
//! The generic idea (no sample targeting): obfuscators ship a small *pure*
//! "library" — a string array, a decoder function that indexes/transforms it,
//! and often a rotation IIFE that shuffles the array — then replace string
//! literals all over the program with calls like `decode(0x1b)` / `dec(tbl.k)`.
//!
//! We (1) find the pure top-level bindings by free-variable analysis (a binding
//! is pure if every free name it references is itself pure or a known global),
//! (2) serialize them in order as a *preamble*, and (3) for every call whose
//! callee is a pure decoder and whose arguments only reference pure names,
//! execute `preamble; theCall` in the sandbox and splice the resulting string
//! back in. Execution is the validator: a non-pure or non-string-returning
//! "decoder" simply fails to evaluate and is left untouched — so a lift can only
//! ever replace a call with the value it actually computes.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{
    ArrayExpressionElement, BindingPattern, Expression, ObjectPropertyKind, Program, PropertyKey,
    Statement,
};
use oxc_ast::AstBuilder;
use oxc_ast_visit::Visit;
use oxc_codegen::Codegen;
use oxc_span::{GetSpan, SourceType, SPAN};
use oxc_syntax::scope::ScopeFlags;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::sandbox::{EvalValue, Sandbox};
use crate::util::build_scoping;

/// Globals a pure decoder may reference (present in QuickJS, side-effect free).
/// Referencing/constructing any of these has no host-visible effect, so a
/// function whose only free names are these (or other pure bindings) is pure.
/// The double-eval determinism guard separately rejects any that turn out
/// non-deterministic (e.g. `Date`), so the list can stay generous.
const GLOBALS: &[&str] = &[
    "String", "Math", "Array", "Number", "Boolean", "Object", "JSON", "RegExp", "parseInt",
    "parseFloat", "isNaN", "isFinite", "decodeURIComponent", "encodeURIComponent", "unescape",
    "escape", "undefined", "NaN", "Infinity", "Date",
    // Collections & symbols — common in modern decoders (e.g. a Map charset
    // lookup). Pure to construct; mutation stays inside the sandbox.
    "Map", "Set", "WeakMap", "WeakSet", "Symbol", "BigInt",
    // Binary/typed-array machinery used by byte-oriented decoders.
    "ArrayBuffer", "SharedArrayBuffer", "DataView", "Int8Array", "Uint8Array",
    "Uint8ClampedArray", "Int16Array", "Uint16Array", "Int32Array", "Uint32Array", "Float32Array",
    "Float64Array", "BigInt64Array", "BigUint64Array",
    // Error constructors (decoders sometimes build/throw-and-catch internally).
    "Error", "TypeError", "RangeError", "SyntaxError", "ReferenceError", "EvalError", "URIError",
    // Pure reflection helpers.
    "Reflect",
];

#[derive(Default)]
pub struct DecoderLift {
    changed: bool,
}

impl Pass for DecoderLift {
    fn name(&self) -> &'static str {
        "decoder-lift"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let tops = collect_top_level(program);
        if tops.is_empty() {
            return false;
        }

        let analysis = classify(&tops);
        if analysis.pure_funcs.is_empty() {
            return false;
        }

        // Resolve simple decoder aliases (`v = decoder`, `var v = decoder`) so
        // calls through the alias are liftable too.
        let aliases = collect_aliases(program, &analysis.pure_funcs);

        // Pure-data object/array bindings anywhere in the program (e.g. the
        // local `{g:219,...}` key tables), gated by global name-uniqueness so
        // flattening them into one global preamble can never collide. After the
        // rename pass every name is unique, so this captures essentially all of
        // them on later fixpoint rounds.
        let local_data = scan_invariants(program);

        if std::env::var("DL_DEBUG").is_ok() {
            eprintln!(
                "[dl] pure_funcs={} aliases={} local_data={} alias_sample={:?}",
                analysis.pure_funcs.len(),
                aliases.len(),
                local_data.len(),
                aliases.iter().take(3).collect::<Vec<_>>()
            );
        }

        // Callable: classified-pure decoder functions + their aliases (the only
        // calls we lift). Allowed: every free var permitted in a lifted
        // expression — pure names, aliases, and local pure data tables/chains.
        let mut callable = analysis.pure_funcs.clone();
        callable.extend(aliases.keys().cloned());
        let mut allowed = analysis.pureset.clone();
        allowed.extend(aliases.keys().cloned());
        allowed.extend(local_data.iter().map(|(n, _)| n.clone()));

        let preamble = build_preamble(&tops, &analysis, &aliases, &allowed, &local_data, allocator);
        let Some(sandbox) = Sandbox::new() else {
            return false;
        };
        // Define the whole library once; per-call evals reuse it (fast + the
        // array is rotated exactly once, as in the real program).
        if !sandbox.setup(&preamble) {
            return false;
        }

        // Names of grounded invariants (incl. base-N arithmetic), for
        // materializing `x = <polynomial>` to `x = <constant>`.
        let grounded: HashSet<String> = local_data.iter().map(|(n, _)| n.clone()).collect();

        let mut lifter = Lifter {
            callable: &callable,
            allowed: &allowed,
            grounded: &grounded,
            sandbox,
            memo: HashMap::new(),
            changed: false,
            _marker: std::marker::PhantomData,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut lifter, allocator, program, scoping, ());
        self.changed = lifter.changed;
        self.changed
    }
}

// ---------------------------------------------------------------------------
// Top-level statement collection (descends one level into wrapping IIFEs)
// ---------------------------------------------------------------------------

fn collect_top_level<'b, 'a>(program: &'b Program<'a>) -> Vec<&'b Statement<'a>> {
    let mut out: Vec<&Statement> = Vec::new();
    for stmt in &program.body {
        collect_stmt(stmt, &mut out, 0);
    }
    out
}

/// Collect a statement and recurse through *transparent wrappers* — wrapping
/// IIFEs, `try {}` blocks, and bare blocks — that hold the decoder library
/// without forming a meaningful value scope. We do NOT descend into function
/// bodies, loops, or conditionals (those are real logic). Depth-bounded.
fn collect_stmt<'b, 'a>(stmt: &'b Statement<'a>, out: &mut Vec<&'b Statement<'a>>, depth: usize) {
    if depth > 6 {
        return;
    }
    out.push(stmt);
    if let Some(body) = iife_body(stmt) {
        for s in body {
            collect_stmt(s, out, depth + 1);
        }
    }
    match stmt {
        Statement::TryStatement(t) => {
            for s in &t.block.body {
                collect_stmt(s, out, depth + 1);
            }
        }
        Statement::BlockStatement(b) => {
            for s in &b.body {
                collect_stmt(s, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// If `stmt` is `(<unary>* (function(){ <body> })())`, return its body.
fn iife_body<'b, 'a>(stmt: &'b Statement<'a>) -> Option<&'b ArenaVec<'a, Statement<'a>>> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let mut expr = &es.expression;
    while let Expression::UnaryExpression(u) = expr {
        expr = &u.argument;
    }
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    match &call.callee {
        Expression::FunctionExpression(f) => f.body.as_ref().map(|b| &b.statements),
        Expression::ArrowFunctionExpression(f) => Some(&f.body.statements),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Free-variable analysis
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FreeVars {
    used: HashSet<String>,
    bound: HashSet<String>,
    /// Base names of every write target (`x = …`, `x++`, `o.p = …`, `delete o.p`).
    /// A write whose base is *not* one of `bound` mutates outer/global state — a
    /// side effect that lifting the value would silently drop.
    written: HashSet<String>,
    impure: bool, // uses `this`, or performs an un-analyzable write → not liftable
}

impl<'a> Visit<'a> for FreeVars {
    fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
        self.used.insert(it.name.to_string());
    }
    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        self.bound.insert(it.name.to_string());
    }
    fn visit_this_expression(&mut self, _it: &oxc_ast::ast::ThisExpression) {
        self.impure = true;
    }

    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        self.record_write(assign_target_base(&node.left));
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &oxc_ast::ast::UpdateExpression<'a>) {
        self.record_write(simple_target_base(&node.argument));
        oxc_ast_visit::walk::walk_update_expression(self, node);
    }

    fn visit_unary_expression(&mut self, node: &oxc_ast::ast::UnaryExpression<'a>) {
        // `delete o.p` mutates `o`; treat its base like any other write.
        if matches!(node.operator, oxc_syntax::operator::UnaryOperator::Delete) {
            self.record_write(member_base(&node.argument));
        }
        oxc_ast_visit::walk::walk_unary_expression(self, node);
    }
}

impl FreeVars {
    /// Record a write whose target base resolves to `name`. An unresolvable shape
    /// (destructuring, a write through a call result) is conservatively impure.
    fn record_write(&mut self, name: Option<String>) {
        match name {
            Some(n) => {
                self.written.insert(n);
            }
            None => self.impure = true,
        }
    }

    fn free(&self) -> HashSet<String> {
        self.used.difference(&self.bound).cloned().collect()
    }

    /// True if the body writes to any name it does not itself bind — i.e. mutates
    /// an outer variable or a global. Such a function has a side effect beyond its
    /// return value and must never be lifted away.
    fn writes_free(&self) -> bool {
        self.written.iter().any(|n| !self.bound.contains(n))
    }

    fn all_allowed(&self, pureset: &HashSet<String>, extra: Option<&str>) -> bool {
        !self.impure
            && !self.writes_free()
            && self.free().iter().all(|n| {
                Some(n.as_str()) == extra
                    || pureset.contains(n)
                    || GLOBALS.contains(&n.as_str())
            })
    }
}

/// The base identifier name of an assignment target (`x` for `x`, `x.a`, `x[i]`),
/// or `None` for destructuring patterns and member chains not rooted in a name.
fn assign_target_base(target: &oxc_ast::ast::AssignmentTarget<'_>) -> Option<String> {
    use oxc_ast::ast::AssignmentTarget as T;
    match target {
        T::AssignmentTargetIdentifier(id) => Some(id.name.to_string()),
        T::StaticMemberExpression(m) => member_base(&m.object),
        T::ComputedMemberExpression(m) => member_base(&m.object),
        T::PrivateFieldExpression(m) => member_base(&m.object),
        _ => None,
    }
}

fn simple_target_base(target: &oxc_ast::ast::SimpleAssignmentTarget<'_>) -> Option<String> {
    use oxc_ast::ast::SimpleAssignmentTarget as S;
    match target {
        S::AssignmentTargetIdentifier(id) => Some(id.name.to_string()),
        S::StaticMemberExpression(m) => member_base(&m.object),
        S::ComputedMemberExpression(m) => member_base(&m.object),
        S::PrivateFieldExpression(m) => member_base(&m.object),
        _ => None,
    }
}

/// Walk a member chain down to its root identifier name. A base that is itself a
/// call/other expression (`f().x = …`) returns `None` → treated conservatively.
fn member_base(e: &Expression<'_>) -> Option<String> {
    match e {
        Expression::Identifier(id) => Some(id.name.to_string()),
        Expression::StaticMemberExpression(m) => member_base(&m.object),
        Expression::ComputedMemberExpression(m) => member_base(&m.object),
        _ => None,
    }
}

fn freevars_of_expr(expr: &Expression<'_>) -> FreeVars {
    let mut fv = FreeVars::default();
    fv.visit_expression(expr);
    fv
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

struct Analysis {
    pureset: HashSet<String>,    // all pure names (data + funcs)
    pure_funcs: HashSet<String>, // subset that are callable
}

fn is_pure_data(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::StringLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::BigIntLiteral(_) => true,
        Expression::UnaryExpression(u) => is_pure_data(&u.argument),
        Expression::ArrayExpression(a) => a.elements.iter().all(is_pure_element),
        Expression::ObjectExpression(o) => o.properties.iter().all(|p| match p {
            ObjectPropertyKind::ObjectProperty(prop) => {
                matches!(
                    prop.key,
                    PropertyKey::StaticIdentifier(_)
                        | PropertyKey::StringLiteral(_)
                        | PropertyKey::NumericLiteral(_)
                ) && is_pure_data(&prop.value)
            }
            ObjectPropertyKind::SpreadProperty(_) => false,
        }),
        _ => false,
    }
}

/// Array element purity. The `Expression` literal variants are inlined directly
/// onto `ArrayExpressionElement` (via `inherit_variants!`), so we match them
/// here without converting to `Expression`.
fn is_pure_element(e: &ArrayExpressionElement<'_>) -> bool {
    match e {
        ArrayExpressionElement::Elision(_)
        | ArrayExpressionElement::StringLiteral(_)
        | ArrayExpressionElement::NumericLiteral(_)
        | ArrayExpressionElement::BooleanLiteral(_)
        | ArrayExpressionElement::NullLiteral(_)
        | ArrayExpressionElement::BigIntLiteral(_) => true,
        ArrayExpressionElement::ArrayExpression(a) => a.elements.iter().all(is_pure_element),
        ArrayExpressionElement::UnaryExpression(u) => is_pure_data(&u.argument),
        _ => false,
    }
}

fn classify(tops: &[&Statement<'_>]) -> Analysis {
    let mut pureset: HashSet<String> = HashSet::new();
    let mut pure_funcs: HashSet<String> = HashSet::new();

    struct Pending {
        name: String,
        fv: FreeVars,
    }
    let mut pending: Vec<Pending> = Vec::new();

    for stmt in tops {
        match stmt {
            Statement::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    let mut fv = FreeVars::default();
                    fv.visit_function(func, ScopeFlags::Function);
                    pending.push(Pending { name: id.name.to_string(), fv });
                }
            }
            Statement::VariableDeclaration(vd) => {
                for d in &vd.declarations {
                    let BindingPattern::BindingIdentifier(id) = &d.id else {
                        continue;
                    };
                    let Some(init) = &d.init else { continue };
                    if matches!(
                        init,
                        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
                    ) {
                        pending.push(Pending { name: id.name.to_string(), fv: freevars_of_expr(init) });
                    } else if is_pure_data(init) {
                        pureset.insert(id.name.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    loop {
        let mut progressed = false;
        pending.retain(|p| {
            if p.fv.impure {
                return false;
            }
            if p.fv.all_allowed(&pureset, Some(&p.name)) {
                pureset.insert(p.name.clone());
                pure_funcs.insert(p.name.clone());
                progressed = true;
                false
            } else {
                true
            }
        });
        if !progressed {
            break;
        }
    }

    Analysis { pureset, pure_funcs }
}

// ---------------------------------------------------------------------------
// Preamble construction
// ---------------------------------------------------------------------------

/// Globals whose value is a constant available in the sandbox.
const CONST_GLOBALS: &[&str] = &["undefined", "NaN", "Infinity"];

/// Collect bindings whose value is an *invariant*: a key-table (`{g:219,…}`) or
/// an arithmetic/logical expression that bottoms out in literals through other
/// invariants (DataDome hides decoder indices this way: `b = a + a`, `a = 0`).
///
/// Soundness: a binding qualifies only if its name is globally unique, it is
/// written exactly once, its value is the same everywhere, and — crucially —
/// *every* identifier it references is itself an invariant (a groundedness
/// fixpoint). Anything reaching a runtime value is dropped, so the sandbox
/// always computes the true value. Emitted in dependency order.
fn scan_invariants(program: &Program<'_>) -> Vec<(String, String)> {
    let mut c = DataCollector::default();
    c.visit_program(program);

    let cand: HashMap<String, (String, HashSet<String>)> = c
        .value
        .iter()
        .filter(|(n, _)| {
            c.counts.get(*n).copied() == Some(1)
                && c.writes.get(*n).copied() == Some(1)
                && !c.conflict.contains(*n)
        })
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();

    // Groundedness fixpoint; records dependency-safe emission order.
    let mut grounded: HashSet<String> = HashSet::new();
    let mut emit: Vec<String> = Vec::new();
    loop {
        let mut added = false;
        for n in &c.order {
            if grounded.contains(n) {
                continue;
            }
            let Some((_, ops)) = cand.get(n) else { continue };
            if ops
                .iter()
                .all(|o| grounded.contains(o) || CONST_GLOBALS.contains(&o.as_str()))
            {
                grounded.insert(n.clone());
                emit.push(n.clone());
                added = true;
            }
        }
        if !added {
            break;
        }
    }

    emit.into_iter().map(|n| (n.clone(), cand[&n].0.clone())).collect()
}

#[derive(Default)]
struct DataCollector {
    counts: HashMap<String, usize>,                     // binding occurrences
    writes: HashMap<String, usize>,                     // write occurrences
    value: HashMap<String, (String, HashSet<String>)>,  // name -> (decl, operands)
    conflict: HashSet<String>,                          // non-invariant / ambiguous
    order: Vec<String>,                                 // first-seen order
}

impl DataCollector {
    fn write(&mut self, name: &str) {
        *self.writes.entry(name.to_string()).or_default() += 1;
    }

    fn value(&mut self, name: &str, init: &Expression<'_>) {
        let mut ops = HashSet::new();
        if !invariant_shape(init, &mut ops) {
            self.conflict.insert(name.to_string());
            return;
        }
        let mut cg = Codegen::new();
        cg.print_expression(init);
        let decl = format!("var {} = {};", name, cg.into_source_text());
        match self.value.get(name) {
            Some((prev, _)) if *prev != decl => {
                self.conflict.insert(name.to_string());
            }
            _ => {
                if !self.value.contains_key(name) {
                    self.order.push(name.to_string());
                }
                self.value.insert(name.to_string(), (decl, ops));
            }
        }
    }
}

/// Is `expr` an invariant-shaped value? Fills `ops` with the identifier names it
/// depends on. Pure data (literals/arrays/objects of literals) has no operands;
/// arithmetic/logical/conditional expressions contribute their identifier leaves.
fn invariant_shape(expr: &Expression<'_>, ops: &mut HashSet<String>) -> bool {
    use oxc_syntax::operator::BinaryOperator;
    if is_pure_data(expr) {
        return true;
    }
    match expr {
        Expression::Identifier(id) => {
            ops.insert(id.name.to_string());
            true
        }
        Expression::UnaryExpression(u) => {
            !matches!(u.operator, oxc_syntax::operator::UnaryOperator::Delete)
                && invariant_shape(&u.argument, ops)
        }
        Expression::BinaryExpression(b) => {
            !matches!(b.operator, BinaryOperator::In | BinaryOperator::Instanceof)
                && invariant_shape(&b.left, ops)
                && invariant_shape(&b.right, ops)
        }
        Expression::LogicalExpression(l) => {
            invariant_shape(&l.left, ops) && invariant_shape(&l.right, ops)
        }
        Expression::ConditionalExpression(c) => {
            invariant_shape(&c.test, ops)
                && invariant_shape(&c.consequent, ops)
                && invariant_shape(&c.alternate, ops)
        }
        _ => false,
    }
}

impl<'a> Visit<'a> for DataCollector {
    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        *self.counts.entry(it.name.to_string()).or_default() += 1;
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if let Some(init) = &node.init {
                self.write(id.name.as_str());
                self.value(id.name.as_str(), init);
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }

    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(t) = &node.left {
            self.write(t.name.as_str());
            if matches!(node.operator, oxc_syntax::operator::AssignmentOperator::Assign) {
                self.value(t.name.as_str(), &node.right);
            } else {
                self.conflict.insert(t.name.to_string()); // compound assignment
            }
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &oxc_ast::ast::UpdateExpression<'a>) {
        if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &node.argument {
            self.write(id.name.as_str());
            self.conflict.insert(id.name.to_string()); // ++ / -- → not constant
        }
        oxc_ast_visit::walk::walk_update_expression(self, node);
    }
}

fn build_preamble<'a>(
    tops: &[&Statement<'a>],
    analysis: &Analysis,
    aliases: &HashMap<String, String>,
    allowed: &HashSet<String>,
    local_data: &[(String, String)],
    allocator: &'a Allocator,
) -> String {
    // Pure data + decoder function declarations (hoisted, so order is fine).
    let ast = AstBuilder::new(allocator);
    let mut body = ast.vec();
    for stmt in tops {
        if statement_is_pure(stmt, analysis) {
            body.push(stmt.clone_in(allocator));
        }
    }
    let prog = ast.program(SPAN, SourceType::default(), "", ast.vec(), None, ast.vec(), body);
    let mut code = Codegen::new().build(&prog).code;

    // Pure IIFEs that manipulate a decoder (the array *rotation* — crucially,
    // even when nested inside otherwise-impure init code). Running them in the
    // sandbox is harmless (preamble is throwaway) and is what makes a rotated
    // decoder return the *correct* strings.
    let mut iifes = IifeCollector { allowed, pure_funcs: &analysis.pure_funcs, out: Vec::new() };
    for stmt in tops {
        iifes.visit_statement(stmt);
    }
    for src in iifes.out {
        code.push_str("\n;");
        code.push_str(&src);
        code.push(';');
    }

    // Alias definitions, so calls through `var v = decoder` resolve.
    for (a, b) in aliases {
        code.push_str(&format!("\nvar {a} = {b};"));
    }

    // Local pure-data key tables (`var t = {g:219,...}`), so calls of the form
    // `decoder(t.g)` evaluate.
    for (_, decl) in local_data {
        code.push('\n');
        code.push_str(decl);
    }

    code
}

fn statement_is_pure(stmt: &Statement<'_>, analysis: &Analysis) -> bool {
    match stmt {
        Statement::FunctionDeclaration(func) => func
            .id
            .as_ref()
            .is_some_and(|id| analysis.pureset.contains(id.name.as_str())),
        Statement::VariableDeclaration(vd) => !vd.declarations.is_empty() && vd.declarations.iter().all(|d| {
            matches!(&d.id, BindingPattern::BindingIdentifier(id) if analysis.pureset.contains(id.name.as_str()))
        }),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Alias resolution: `v = decoder` / `var v = decoder`
// ---------------------------------------------------------------------------

fn collect_aliases(program: &Program<'_>, pure_funcs: &HashSet<String>) -> HashMap<String, String> {
    // 1. Collect identifier→identifier assignment edges (`a = b`). A name is
    //    disqualified if it is ever assigned a non-identifier or two different
    //    identifiers (then it isn't a stable alias).
    let mut c = AliasCollector { edges: HashMap::new(), disqualified: HashSet::new() };
    c.visit_program(program);

    // 2. Resolve each edge transitively to the pure decoder it ultimately
    //    points at: a -> b -> ... -> decoder.
    let mut out = HashMap::new();
    for name in c.edges.keys() {
        if c.disqualified.contains(name) {
            continue;
        }
        let mut cur = c.edges[name].clone();
        for _ in 0..32 {
            if c.disqualified.contains(&cur) {
                break;
            }
            if pure_funcs.contains(&cur) {
                out.insert(name.clone(), cur);
                break;
            }
            match c.edges.get(&cur) {
                Some(next) => cur = next.clone(),
                None => break,
            }
        }
    }
    out
}

struct AliasCollector {
    edges: HashMap<String, String>,
    disqualified: HashSet<String>,
}

impl AliasCollector {
    fn record(&mut self, name: String, rhs: Option<&str>) {
        match rhs {
            Some(b) => match self.edges.get(&name) {
                Some(prev) if prev != b => {
                    self.disqualified.insert(name); // two different targets
                }
                _ => {
                    self.edges.insert(name, b.to_string());
                }
            },
            None => {
                self.disqualified.insert(name); // assigned a non-identifier
            }
        }
    }
}

impl<'a> Visit<'a> for AliasCollector {
    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(target) = &node.left {
            let rhs = match &node.right {
                Expression::Identifier(b) => Some(b.name.as_str()),
                _ => None,
            };
            self.record(target.name.to_string(), rhs);
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if let Some(Expression::Identifier(b)) = &node.init {
                self.record(id.name.to_string(), Some(b.name.as_str()));
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }
}

// ---------------------------------------------------------------------------
// Pure-IIFE collection (the rotation), even when nested in impure code
// ---------------------------------------------------------------------------

struct IifeCollector<'r> {
    allowed: &'r HashSet<String>,
    pure_funcs: &'r HashSet<String>,
    out: Vec<String>,
}

impl<'a, 'r> Visit<'a> for IifeCollector<'r> {
    fn visit_expression(&mut self, expr: &Expression<'a>) {
        if let Expression::CallExpression(call) = expr {
            let is_iife = matches!(
                &call.callee,
                Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
            );
            if is_iife {
                let fv = freevars_of_expr(expr);
                let free = fv.free();
                let touches_decoder = free.iter().any(|n| self.pure_funcs.contains(n));
                let pure = !fv.impure
                    && free
                        .iter()
                        .all(|n| self.allowed.contains(n) || GLOBALS.contains(&n.as_str()));
                if touches_decoder && pure {
                    let mut cg = Codegen::new();
                    cg.print_expression(expr);
                    self.out.push(cg.into_source_text());
                }
            }
        }
        oxc_ast_visit::walk::walk_expression(self, expr);
    }
}

// ---------------------------------------------------------------------------
// The lifter
// ---------------------------------------------------------------------------

struct Lifter<'a, 'r> {
    callable: &'r HashSet<String>,
    allowed: &'r HashSet<String>,
    grounded: &'r HashSet<String>,
    sandbox: Sandbox,
    memo: HashMap<String, Option<EvalValue>>,
    changed: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'r> Lifter<'a, 'r> {
    /// If `name` is a grounded invariant and `rhs` is an arithmetic expression
    /// (a base-N polynomial hiding a constant), replace `rhs` with the constant
    /// the sandbox computes. Sound: the assignment still happens at the same
    /// point; only its right-hand side is simplified to an equal value.
    fn materialize(&mut self, name: &str, rhs: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if !self.grounded.contains(name) || !is_foldable_arith(rhs) {
            return;
        }
        let Some(val) = self.sandbox.eval_to_string(&format!("({});", name)) else {
            return;
        };
        let Ok(n) = val.parse::<f64>() else {
            return;
        };
        if !n.is_finite() || n < 0.0 {
            return;
        }
        let span = rhs.span();
        *rhs = ctx.ast.expression_numeric_literal(span, n, None, oxc_syntax::number::NumberBase::Decimal);
        self.changed = true;
    }
}

fn is_foldable_arith(e: &Expression<'_>) -> bool {
    matches!(
        e,
        Expression::BinaryExpression(_)
            | Expression::UnaryExpression(_)
            | Expression::LogicalExpression(_)
            | Expression::ConditionalExpression(_)
            | Expression::Identifier(_)
    )
}

impl<'a, 'r> Traverse<'a, ()> for Lifter<'a, 'r> {
    fn enter_variable_declarator(
        &mut self,
        node: &mut oxc_ast::ast::VariableDeclarator<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            let name = id.name.to_string();
            if let Some(init) = &mut node.init {
                self.materialize(&name, init, ctx);
            }
        }
    }

    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Materialize grounded base-N arithmetic assignments to their constant.
        if let Expression::AssignmentExpression(a) = expr {
            if matches!(a.operator, oxc_syntax::operator::AssignmentOperator::Assign) {
                if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                    let name = id.name.to_string();
                    self.materialize(&name, &mut a.right, ctx);
                }
            }
            return;
        }

        let lift: Option<EvalValue> = {
            // Two kinds of decode site are sound to lift:
            //  * a CALL whose callee is a *classified-pure* decoder/alias
            //    identifier — purity analysis proved it side-effect-free, so
            //    replacing it with its value is safe;
            //  * a computed-member READ `library[index]` — reading an element of
            //    a pure data table can't mutate anything.
            // We do NOT lift method-form calls like `obj.m(args)`: a method may
            // mutate shared state (e.g. the rotation `arr.push(arr.shift())`),
            // and the determinism guard can't see a length-preserving mutation.
            match &*expr {
                Expression::CallExpression(c) => {
                    let Expression::Identifier(callee) = &c.callee else {
                        return;
                    };
                    if !self.callable.contains(callee.name.as_str()) {
                        return;
                    }
                }
                Expression::ComputedMemberExpression(m) => {
                    let touches = freevars_of_expr(&m.object)
                        .free()
                        .iter()
                        .any(|n| self.allowed.contains(n));
                    if !touches {
                        return;
                    }
                }
                _ => return,
            }
            if !freevars_of_expr(&*expr).all_allowed(self.allowed, None) {
                return;
            }

            let mut cg = Codegen::new();
            cg.print_expression(&*expr);
            let src = cg.into_source_text();

            if let Some(cached) = self.memo.get(&src) {
                cached.clone()
            } else {
                // The library is already defined in the persistent context;
                // evaluate just the call expression. Evaluate twice and require
                // agreement: this refuses to lift any non-deterministic "decoder"
                // (e.g. one reaching Math.random / Date), so a lift is only ever
                // a stable, reproducible value.
                let call = format!("({});", src);
                let result = match self.sandbox.eval_to_value(&call) {
                    Some(a) if self.sandbox.eval_to_value(&call).as_ref() == Some(&a) => Some(a),
                    _ => None,
                };
                self.memo.insert(src, result.clone());
                result
            }
        };

        if let Some(val) = lift {
            let span = expr.span();
            *expr = match val {
                EvalValue::Str(s) => {
                    let value = ctx.ast.str(&s);
                    ctx.ast.expression_string_literal(span, value, None)
                }
                // A numeric/boolean decoder result must splice as that literal
                // kind, not as a string — `dec(x) + 1` must stay arithmetic.
                EvalValue::Num(n) if n.is_finite() => num_literal(n, span, ctx),
                EvalValue::Bool(b) => ctx.ast.expression_boolean_literal(span, b),
                // NaN / Infinity have no literal form; leave the call in place.
                EvalValue::Num(_) => return,
            };
            self.changed = true;
        }
    }
}

/// Build a numeric literal expression for `n`. A numeric literal is always
/// non-negative in the grammar, so negatives become `-(lit)` to round-trip.
fn num_literal<'a>(
    n: f64,
    span: oxc_span::Span,
    ctx: &mut TraverseCtx<'a, ()>,
) -> Expression<'a> {
    use oxc_syntax::number::NumberBase;
    if n.is_sign_negative() {
        let lit = ctx.ast.expression_numeric_literal(span, -n, None, NumberBase::Decimal);
        ctx.ast.expression_unary(span, oxc_syntax::operator::UnaryOperator::UnaryNegation, lit)
    } else {
        ctx.ast.expression_numeric_literal(span, n, None, NumberBase::Decimal)
    }
}
