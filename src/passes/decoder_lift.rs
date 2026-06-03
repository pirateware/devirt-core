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
pub(crate) const GLOBALS: &[&str] = &[
    "String",
    "Math",
    "Array",
    "Number",
    "Boolean",
    "Object",
    "JSON",
    "RegExp",
    "parseInt",
    "parseFloat",
    "isNaN",
    "isFinite",
    "decodeURIComponent",
    "encodeURIComponent",
    "unescape",
    "escape",
    "undefined",
    "NaN",
    "Infinity",
    "Date",
    // Collections & symbols — common in modern decoders (e.g. a Map charset
    // lookup). Pure to construct; mutation stays inside the sandbox.
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Symbol",
    "BigInt",
    // Binary/typed-array machinery used by byte-oriented decoders.
    "ArrayBuffer",
    "SharedArrayBuffer",
    "DataView",
    "Int8Array",
    "Uint8Array",
    "Uint8ClampedArray",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "Float32Array",
    "Float64Array",
    "BigInt64Array",
    "BigUint64Array",
    // Error constructors (decoders sometimes build/throw-and-catch internally).
    "Error",
    "TypeError",
    "RangeError",
    "SyntaxError",
    "ReferenceError",
    "EvalError",
    "URIError",
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

        // Names that are reassigned/mutated anywhere (more than one value write,
        // or a compound/update). Such a binding is NOT a stable library member:
        // its declaration value, baked into the preamble, may be stale at a decode
        // site (e.g. `var s=[1,2]; function get(){return s[0]} s=[3,4]; get()`).
        // Excluding them keeps the lift sound by construction.
        let unstable = scan_unstable(program);

        // Pure-data tables anywhere in the program (the local key/string tables,
        // e.g. DataDome's `var597`). Collected here — *before* classification —
        // so a decoder nested in a function scope whose only free names are such a
        // table (+ globals) can be proven pure: its table must already be a known
        // pure name when we run the purity fixpoint. See `collect_scoped_funcs`.
        // Symbol-precise soundness gate (plan §5.3): identify every *aggregate*
        // table the SSA escape analysis cannot prove immutable — one reassigned,
        // member-written (`t[i]=x`), mutated (`t.push`), or aliased anywhere. This
        // closes the hole the name-based `scan_invariants` count is blind to (it
        // sees only bare assignments), the hazard scoped lifting widens. One source
        // of truth: `src/ir/analysis.rs`.
        let table_scoping = build_scoping(program);
        let mutable = crate::ir::mutable_aggregate_tables(program, &table_scoping);

        // Feed the exclusion *into* invariant collection so its groundedness
        // fixpoint also drops anything that transitively depends on an excluded
        // table (e.g. an identity alias `b = t`), preserving dependency closure —
        // a post-hoc filter would leave such dependents baked against an undefined
        // name and break the preamble.
        let local_data = scan_invariants(program, &mutable);
        let data_names: HashSet<String> = local_data.iter().map(|(n, _)| n.clone()).collect();

        // Candidate decoder functions at *every* scope depth — not just the
        // program top level. This is the Tier-C/D generalisation: the DataDome
        // decoder library lives ~5 scopes deep inside `(() => {…})()`, which
        // `collect_top_level` never descends into. Name-uniqueness + the
        // `unstable` gate keep flattening them into one global preamble sound (a
        // shadowed or reassigned name is dropped).
        let scoped_funcs = collect_scoped_funcs(program, &unstable);

        // Private memoization caches: state a decoder writes to cache a pure result.
        // Recognizing them lets a memo decoder (`C[k]===undefined?(C[k]=miss):C[k]`)
        // pass the purity gate despite its free write to `C`. See
        // `collect_private_caches` for the (strong) soundness conditions.
        let private_cache_decls = collect_private_caches(program, &unstable);
        let private_caches: HashSet<String> =
            private_cache_decls.iter().map(|(n, _)| n.clone()).collect();

        if std::env::var("DL_DEBUG").is_ok() {
            eprintln!(
                "[dl] private_caches={} sample={:?}",
                private_caches.len(),
                private_caches.iter().take(5).collect::<Vec<_>>()
            );
        }

        let analysis = classify(&tops, &unstable, &data_names, scoped_funcs, &private_caches);
        if analysis.pure_funcs.is_empty() {
            return false;
        }

        // Resolve simple decoder aliases (`v = decoder`, `var v = decoder`) so
        // calls through the alias are liftable too. A reassigned/mutated alias is
        // not stable, so drop those too.
        let mut aliases = collect_aliases(program, &analysis.pure_funcs);
        aliases.retain(|k, _| !unstable.contains(k));

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

        // Serialize *every* pure decoder function — at any scope depth — as a
        // standalone preamble declaration. This is the one mechanism that emits
        // functions (the top-level data walk in `build_preamble` emits only pure
        // data vars), so a multi-declarator or deeply-nested decoder can't fall
        // through a crack. Names are unique, so flattening is collision-free.
        let pure_func_decls = serialize_pure_funcs(program, &analysis.pure_funcs, allocator);

        let preamble = build_preamble(
            &tops,
            &analysis,
            &aliases,
            &allowed,
            &local_data,
            &pure_func_decls,
            &private_cache_decls,
            allocator,
        );
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

        // Strip spent scaffolding: a string-array rotation IIFE whose only
        // observable effect is mutating decoder tables that no *real* code reads
        // anymore (every decode site was lifted). DCE can't remove a bare-call
        // expression statement, so it would otherwise keep the rotation — and the
        // array it mutates — alive forever. We own this cleanup because only this
        // pass knows which bindings are the decoder library.
        let mut scaffolding: HashSet<String> = analysis.pureset.clone();
        scaffolding.extend(data_names.iter().cloned());
        scaffolding.extend(aliases.keys().cloned());
        scaffolding.extend(private_caches.iter().cloned());
        if self.cleanup_spent_rotations(program, &scaffolding) {
            self.changed = true;
        }

        self.changed
    }
}

impl DecoderLift {
    /// Remove top-level string-array *rotation* IIFEs that are now dead. A bare
    /// `(function(arr,n){ … arr.push(arr.shift()) … })(table, k);` statement only
    /// mutates the decoder `table`; once no live code reads that table (all decode
    /// sites lifted), the rotation is unobservable and safe to drop. We remove
    /// only the rotation statement — the now-unreferenced table/decoder bindings
    /// fall to DCE on the next round.
    ///
    /// Liveness is name-based (like the rest of this pass): a scaffolding name is
    /// *live* if a non-scaffolding statement references it, closed under the
    /// scaffolding declarations' own dependencies. A rotation IIFE is removable
    /// iff it touches no live scaffolding name.
    fn cleanup_spent_rotations(
        &self,
        program: &mut Program<'_>,
        scaffolding: &HashSet<String>,
    ) -> bool {
        // Classify each top-level statement and gather the scaffolding names it
        // references (identifier reads only).
        let mut decl_deps: HashMap<String, HashSet<String>> = HashMap::new(); // name -> scaffolding refs in its init
        let mut rotation_idx: Vec<(usize, HashSet<String>)> = Vec::new(); // (stmt index, touched scaffolding names)
        let mut live: HashSet<String> = HashSet::new();

        for (i, stmt) in program.body.iter().enumerate() {
            if let Some(names) = scaffold_decl_names(stmt, scaffolding) {
                let refs = stmt_scaffold_refs(stmt, scaffolding);
                for n in &names {
                    decl_deps.insert(n.clone(), refs.clone());
                }
                continue;
            }
            if let Some(touched) = rotation_iife_touches(stmt, scaffolding) {
                rotation_idx.push((i, touched));
                continue;
            }
            // Real statement: every scaffolding name it reads is live.
            live.extend(stmt_scaffold_refs(stmt, scaffolding));
        }

        if rotation_idx.is_empty() {
            return false;
        }

        // Close `live` under declaration dependencies: if a live name is defined
        // by a scaffolding decl, that decl's own scaffolding references are live.
        loop {
            let mut added = false;
            for (name, deps) in &decl_deps {
                if live.contains(name) {
                    for d in deps {
                        if live.insert(d.clone()) {
                            added = true;
                        }
                    }
                }
            }
            if !added {
                break;
            }
        }

        // Remove rotation IIFEs that touch only dead scaffolding.
        let dead: HashSet<usize> = rotation_idx
            .iter()
            .filter(|(_, touched)| touched.is_disjoint(&live))
            .map(|(i, _)| *i)
            .collect();
        if dead.is_empty() {
            return false;
        }
        let mut idx = 0usize;
        program.body.retain(|_| {
            let keep = !dead.contains(&idx);
            idx += 1;
            keep
        });
        true
    }
}

/// If `stmt` is a `var`/`let`/`const` declaration binding only scaffolding names
/// with pure (data or function) initializers, return those names.
fn scaffold_decl_names(stmt: &Statement<'_>, scaffolding: &HashSet<String>) -> Option<Vec<String>> {
    let Statement::VariableDeclaration(vd) = stmt else {
        return None;
    };
    let mut names = Vec::new();
    for d in &vd.declarations {
        let BindingPattern::BindingIdentifier(id) = &d.id else {
            return None;
        };
        if !scaffolding.contains(id.name.as_str()) {
            return None;
        }
        match &d.init {
            Some(init) if is_pure_data(init) || is_function_value(init) => {}
            _ => return None,
        }
        names.push(id.name.to_string());
    }
    (!names.is_empty()).then_some(names)
}

fn is_function_value(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
    )
}

/// If `stmt` is a bare, side-effect-free IIFE expression statement that touches
/// (reads/mutates) at least one scaffolding name and nothing else outside the
/// safe globals, return the scaffolding names it touches. This is the string-array
/// rotation: `(function(arr, n){ … })(table, k);`.
fn rotation_iife_touches(
    stmt: &Statement<'_>,
    scaffolding: &HashSet<String>,
) -> Option<HashSet<String>> {
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
    if !matches!(
        &call.callee,
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
    ) {
        return None;
    }
    let fv = freevars_of_expr(expr);
    // Pure modulo the table mutation: no `this`, no writes to free vars, every
    // free name is scaffolding or a side-effect-free global. A rotation IIFE never
    // writes a private cache, so no cache whitelist is needed here.
    if !fv.all_allowed(scaffolding, None, &HashSet::new()) {
        return None;
    }
    let touched: HashSet<String> = fv
        .free()
        .into_iter()
        .filter(|n| scaffolding.contains(n))
        .collect();
    (!touched.is_empty()).then_some(touched)
}

/// Scaffolding names read (as identifier references) anywhere in `stmt`.
fn stmt_scaffold_refs(stmt: &Statement<'_>, scaffolding: &HashSet<String>) -> HashSet<String> {
    struct RefScan<'r> {
        scaffolding: &'r HashSet<String>,
        out: HashSet<String>,
    }
    impl<'a, 'r> Visit<'a> for RefScan<'r> {
        fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
            if self.scaffolding.contains(it.name.as_str()) {
                self.out.insert(it.name.to_string());
            }
        }
    }
    let mut s = RefScan {
        scaffolding,
        out: HashSet::new(),
    };
    s.visit_statement(stmt);
    s.out
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
pub(crate) struct FreeVars {
    used: HashSet<String>,
    bound: HashSet<String>,
    /// Base names of every *strictly disqualifying* write target: bare identifier
    /// writes (`x = …`, `x++`), compound member writes (`o.p += …`), `delete o.p`,
    /// and receiver-mutating method calls (`a.push(…)`). A write whose base is not
    /// one of `bound` mutates outer/global state — a side effect lifting would drop.
    written: HashSet<String>,
    /// Base names of *simple* member assignments (`o[k] = …` / `o.p = …`, plain `=`).
    /// Kept separate from `written` so the memoized-decoder relaxation can whitelist
    /// exactly this class — and only when the base is a proven *private cache*. Any
    /// other write shape stays in `written` and is never whitelistable.
    written_member: HashSet<String>,
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
        use oxc_ast::ast::AssignmentTarget as T;
        let simple = matches!(
            node.operator,
            oxc_syntax::operator::AssignmentOperator::Assign
        );
        // A *simple* member assignment (`o[k] = …`, `o.p = …`) is the only write
        // shape the private-cache relaxation may whitelist; record it separately.
        // Everything else (bare-ident write, compound member write) is strictly
        // disqualifying and goes to `written`.
        match (&node.left, simple) {
            (T::ComputedMemberExpression(m), true) => {
                self.record_member_write(member_base(&m.object))
            }
            (T::StaticMemberExpression(m), true) => {
                self.record_member_write(member_base(&m.object))
            }
            _ => self.record_write(assign_target_base(&node.left)),
        }
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

    fn visit_call_expression(&mut self, node: &oxc_ast::ast::CallExpression<'a>) {
        // A well-known mutating method (`a.push(…)`, `m.set(…)`, `a.splice(…)`)
        // mutates its receiver. If that receiver's base is a free name, the call
        // mutates outer/shared state — a side effect a lifted value would drop.
        // We can't decide purity for an arbitrary method, so this catches the
        // realistic accidental mutators while leaving pure reads (`s.charCodeAt`,
        // `a.slice`, `a[i]`) liftable. Calls through a *local* receiver are fine.
        let prop = match &node.callee {
            Expression::StaticMemberExpression(m) => Some((m.property.name.as_str(), &m.object)),
            Expression::ComputedMemberExpression(m) => match &m.expression {
                Expression::StringLiteral(s) => Some((s.value.as_str(), &m.object)),
                _ => None,
            },
            _ => None,
        };
        if let Some((name, object)) = prop {
            if MUTATOR_METHODS.contains(&name) {
                self.record_write(member_base(object));
            }
        }
        oxc_ast_visit::walk::walk_call_expression(self, node);
    }
}

/// Method names that mutate their receiver (Array, Map/Set, typed arrays). A call
/// to one on a *free* object is treated as a write to that object. Not exhaustive
/// — an unknown method on a free object is assumed pure, the same optimism every
/// decoder lifter makes; the double-eval guard catches non-deterministic returns.
const MUTATOR_METHODS: &[&str] = &[
    "push",
    "pop",
    "shift",
    "unshift",
    "splice",
    "sort",
    "reverse",
    "fill",
    "copyWithin",
    "set",
    "add",
    "delete",
    "clear",
    "setItem",
    "removeItem",
    "setUint8",
    "setInt8",
    "setUint16",
    "setInt16",
    "setUint32",
    "setInt32",
    "setFloat32",
    "setFloat64",
];

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

    /// Record a *simple* member assignment whose object base resolves to `name`
    /// (the whitelistable cache-write class). An unresolvable base is impure.
    fn record_member_write(&mut self, name: Option<String>) {
        match name {
            Some(n) => {
                self.written_member.insert(n);
            }
            None => self.impure = true,
        }
    }

    pub(crate) fn free(&self) -> HashSet<String> {
        self.used.difference(&self.bound).cloned().collect()
    }

    /// True if the body writes to any name it does not itself bind — i.e. mutates
    /// an outer variable or a global. Such a function has a side effect beyond its
    /// return value and must never be lifted away. A simple member write to a name
    /// in `caches` (a proven *private memo cache*) is exempt: writing a private
    /// cache is observationally invisible, so it doesn't count as a free write.
    fn writes_free(&self, caches: &HashSet<String>) -> bool {
        self.written.iter().any(|n| !self.bound.contains(n))
            || self
                .written_member
                .iter()
                .any(|n| !self.bound.contains(n) && !caches.contains(n))
    }

    pub(crate) fn all_allowed(
        &self,
        pureset: &HashSet<String>,
        extra: Option<&str>,
        caches: &HashSet<String>,
    ) -> bool {
        !self.impure
            && !self.writes_free(caches)
            && self.free().iter().all(|n| {
                Some(n.as_str()) == extra
                    || pureset.contains(n)
                    || caches.contains(n)
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

pub(crate) fn freevars_of_expr(expr: &Expression<'_>) -> FreeVars {
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

/// A function whose purity is still to be decided: its name and free-var facts.
struct Pending {
    name: String,
    fv: FreeVars,
}

/// Classify the pure decoder library.
///
/// `seed_pure` are names already *proven* pure data (the local key/string tables
/// from `scan_invariants`) — seeding them lets a decoder whose only free name is
/// such a table pass the purity gate. `scoped_funcs` are candidate functions
/// found at *every* scope depth (see `collect_scoped_funcs`); the top-level path
/// still seeds pure-data vars directly from `tops`.
fn classify(
    tops: &[&Statement<'_>],
    unstable: &HashSet<String>,
    seed_pure: &HashSet<String>,
    scoped_funcs: Vec<Pending>,
    caches: &HashSet<String>,
) -> Analysis {
    let mut pureset: HashSet<String> = seed_pure.clone();
    let mut pure_funcs: HashSet<String> = HashSet::new();

    let mut pending: Vec<Pending> = scoped_funcs;

    // Top-level pure-data var bindings (`var t = {…}`) seed the pure set even when
    // `scan_invariants` (which is groundedness-gated) declined them.
    for stmt in tops {
        if let Statement::VariableDeclaration(vd) = stmt {
            for d in &vd.declarations {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    continue;
                };
                let Some(init) = &d.init else { continue };
                if unstable.contains(id.name.as_str()) {
                    continue;
                }
                if is_pure_data(init) {
                    pureset.insert(id.name.to_string());
                }
            }
        }
    }

    loop {
        let mut progressed = false;
        pending.retain(|p| {
            if p.fv.impure {
                return false;
            }
            if p.fv.all_allowed(&pureset, Some(&p.name), caches) {
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

    Analysis {
        pureset,
        pure_funcs,
    }
}

// ---------------------------------------------------------------------------
// Scoped function collection (Tier C/D): decoders nested in function scopes
// ---------------------------------------------------------------------------

/// Every function binding in the program — `function f(){…}` declarations and
/// `var f = function/arrow` bindings — at *any* depth, as classification
/// candidates. Two soundness gates make flattening them into one global preamble
/// safe: the name must be globally unique (declared exactly once; a shadowed name
/// would make the name-based purity analysis ambiguous), and it must not be
/// `unstable` (reassigned/mutated). The purity fixpoint in `classify` then proves
/// each one side-effect-free before it is ever lifted.
fn collect_scoped_funcs(program: &Program<'_>, unstable: &HashSet<String>) -> Vec<Pending> {
    let mut c = FuncScan::default();
    c.visit_program(program);
    c.funcs
        .into_iter()
        .filter(|(name, _)| !unstable.contains(name))
        .filter(|(name, _)| c.name_counts.get(name).copied() == Some(1))
        .map(|(name, fv)| Pending { name, fv })
        .collect()
}

#[derive(Default)]
struct FuncScan {
    funcs: Vec<(String, FreeVars)>,
    name_counts: HashMap<String, usize>,
}

impl<'a> Visit<'a> for FuncScan {
    fn visit_function(&mut self, func: &oxc_ast::ast::Function<'a>, flags: ScopeFlags) {
        if let Some(id) = &func.id {
            *self.name_counts.entry(id.name.to_string()).or_default() += 1;
            let mut fv = FreeVars::default();
            fv.visit_function(func, flags);
            self.funcs.push((id.name.to_string(), fv));
        }
        oxc_ast_visit::walk::walk_function(self, func, flags);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if let Some(init) = &node.init {
                if matches!(
                    init,
                    Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
                ) {
                    *self.name_counts.entry(id.name.to_string()).or_default() += 1;
                    self.funcs
                        .push((id.name.to_string(), freevars_of_expr(init)));
                }
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }
}

/// Collect *every* pure decoder function (classified pure, at any scope depth) as
/// a standalone preamble statement — an arena clone the preamble codegens as the
/// decoder library. A `function f(){…}` declaration is cloned verbatim
/// (declarations hoist, so order among them is irrelevant); a `var f = function…`
/// becomes a fresh single-declarator `var f = <init>;`. This is the *sole* emitter
/// of functions into the preamble, so none can be missed or double-defined.
fn serialize_pure_funcs<'a>(
    program: &Program<'a>,
    pure_funcs: &HashSet<String>,
    allocator: &'a Allocator,
) -> Vec<Statement<'a>> {
    let mut c = FuncSerializer {
        pure_funcs,
        ast: AstBuilder::new(allocator),
        out: Vec::new(),
    };
    c.visit_program(program);
    c.out
}

struct FuncSerializer<'a, 'r> {
    pure_funcs: &'r HashSet<String>,
    ast: AstBuilder<'a>,
    out: Vec<Statement<'a>>,
}

impl<'a, 'r> FuncSerializer<'a, 'r> {
    fn wants(&self, name: &str) -> bool {
        self.pure_funcs.contains(name)
    }
}

impl<'a, 'r> Visit<'a> for FuncSerializer<'a, 'r> {
    fn visit_function(&mut self, func: &oxc_ast::ast::Function<'a>, flags: ScopeFlags) {
        if let Some(id) = &func.id {
            if self.wants(id.name.as_str()) {
                let cloned = func.clone_in(self.ast.allocator);
                self.out
                    .push(Statement::FunctionDeclaration(oxc_allocator::Box::new_in(
                        cloned,
                        self.ast.allocator,
                    )));
            }
        }
        oxc_ast_visit::walk::walk_function(self, func, flags);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if let Some(init) = &node.init {
                if matches!(
                    init,
                    Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
                ) && self.wants(id.name.as_str())
                {
                    use oxc_ast::ast::VariableDeclarationKind::Var;
                    let id_pat = self.ast.binding_pattern_binding_identifier(SPAN, id.name);
                    let decl = self.ast.variable_declarator(
                        SPAN,
                        Var,
                        id_pat,
                        oxc_ast::NONE,
                        Some(init.clone_in(self.ast.allocator)),
                        false,
                    );
                    let mut decls = self.ast.vec();
                    decls.push(decl);
                    self.out
                        .push(Statement::VariableDeclaration(self.ast.alloc(
                            self.ast.variable_declaration(SPAN, Var, decls, false),
                        )));
                }
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }
}

/// Names that are reassigned or mutated, hence not stable library members. A name
/// qualifies as unstable if it has more than one value-establishing write
/// (declaration + a later `name = …`), or any compound/`++`/`--` write. The
/// single declaration write is the only one the preamble reproduces; anything
/// beyond it could leave the baked value stale at a decode site.
///
/// Name-based (like the rest of this pass) and therefore conservative under
/// shadowing — which is sound: at worst it declines to lift. Note the array
/// *rotation* mutates contents via methods (`arr.push(arr.shift())`), not an
/// identifier assignment, so it does not mark the array unstable.
fn scan_unstable(program: &Program<'_>) -> HashSet<String> {
    let mut s = WriteScan::default();
    s.visit_program(program);
    let mut out = s.flagged;
    for (n, c) in &s.writes {
        if *c > 1 {
            out.insert(n.clone());
        }
    }
    out
}

#[derive(Default)]
struct WriteScan {
    writes: HashMap<String, usize>, // value-establishing writes per name
    flagged: HashSet<String>,       // compound/update → inherently unstable
}

impl<'a> Visit<'a> for WriteScan {
    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if node.init.is_some() {
                *self.writes.entry(id.name.to_string()).or_default() += 1;
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }

    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(t) = &node.left {
            *self.writes.entry(t.name.to_string()).or_default() += 1;
            if !matches!(
                node.operator,
                oxc_syntax::operator::AssignmentOperator::Assign
            ) {
                self.flagged.insert(t.name.to_string());
            }
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &oxc_ast::ast::UpdateExpression<'a>) {
        if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &node.argument
        {
            *self.writes.entry(id.name.to_string()).or_default() += 1;
            self.flagged.insert(id.name.to_string());
        }
        oxc_ast_visit::walk::walk_update_expression(self, node);
    }
}

// ---------------------------------------------------------------------------
// Private memo-cache detection (Phase 1: memoized decoder lifting)
// ---------------------------------------------------------------------------

/// Names of bindings that are *private memoization caches*: state whose only role
/// is to cache a pure function of a key, so writing it is observationally
/// invisible and a decoder that writes it is liftable.
///
/// A name `C` qualifies iff **all** hold:
///   1. `C` is declared exactly once (globally unique name), initialized to a fresh
///      *empty* aggregate (`{}`, `[]`, or `Object.create(null)`), and never
///      reassigned (no later `C = …`, compound, or `++`/`--`) — so baking
///      `var C = <init>` into the sandbox preamble reproduces the true start state.
///   2. **Every** reference to `C` in the whole program is the object of a member
///      access `C[k]` that sits inside a recognized *memo ternary*
///      `C[k] === undefined ? (C[k] = miss) : C[k]` (and the `void 0` / `!==`
///      polarities), with the same key `k` in all three positions and a plain `=`
///      store. There must be at least one such ternary.
///
/// Condition 2 is the soundness crux and is *stronger* than escape analysis alone:
/// it guarantees `C` is never read bare, aliased, passed, returned, deleted from,
/// or — critically — read **outside** the memo idiom (where a cold/warm cache
/// would be observable). Because the guarded read always returns the same value
/// the miss computes, `C` holds no observable state: it is a pure memo of `k`.
/// The miss expression's own purity is *not* checked here — it is enforced by the
/// per-function purity gate that classifies the decoder (an impure miss makes the
/// decoder fail `all_allowed`, so it is never lifted and the side effect survives).
///
/// Returns `(name, "var name = <init>;")` pairs: the exact (empty) initializer is
/// reproduced so the sandbox cache starts in the true state — `{}` and
/// `Object.create(null)` differ for prototype-named keys, so we never normalize.
fn collect_private_caches(
    program: &Program<'_>,
    unstable: &HashSet<String>,
) -> Vec<(String, String)> {
    let mut scan = CacheScan::default();
    scan.visit_program(program);

    scan.empty_init
        .iter()
        .filter(|c| {
            // Globally unique binding, never reassigned/compound-written.
            scan.bind_count.get(*c).copied() == Some(1)
                && !scan.conflict.contains(*c)
                && !unstable.contains(*c)
        })
        .filter(|c| {
            // At least one memo ternary, and every bare reference is accounted for
            // by (contained within) a memo-ternary member access.
            let Some(accounted) = scan.memo_spans.get(*c) else {
                return false;
            };
            let refs = scan.ref_spans.get(*c).map(Vec::as_slice).unwrap_or(&[]);
            refs.iter().all(|r| {
                accounted
                    .iter()
                    .any(|a| a.start <= r.start && r.end <= a.end)
            })
        })
        .map(|c| (c.clone(), scan.init_src[c].clone()))
        .collect()
}

#[derive(Default)]
struct CacheScan {
    bind_count: HashMap<String, usize>, // binding-identifier occurrences per name
    empty_init: HashSet<String>,        // declared once with a fresh empty aggregate
    init_src: HashMap<String, String>,  // `var name = <init>;` for each empty-init name
    conflict: HashSet<String>,          // reassigned / non-empty / ambiguous init
    memo_spans: HashMap<String, Vec<oxc_span::Span>>, // accounted `C[k]` member spans
    ref_spans: HashMap<String, Vec<oxc_span::Span>>, // all identifier-reference spans
}

impl<'a> Visit<'a> for CacheScan {
    fn visit_binding_identifier(&mut self, id: &oxc_ast::ast::BindingIdentifier<'a>) {
        *self.bind_count.entry(id.name.to_string()).or_default() += 1;
    }

    fn visit_identifier_reference(&mut self, id: &oxc_ast::ast::IdentifierReference<'a>) {
        self.ref_spans
            .entry(id.name.to_string())
            .or_default()
            .push(id.span);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            match &node.init {
                Some(init) if is_empty_cache_init(init) => {
                    self.empty_init.insert(id.name.to_string());
                    let mut cg = Codegen::new();
                    cg.print_expression(init);
                    self.init_src.insert(
                        id.name.to_string(),
                        format!("var {} = {};", id.name, cg.into_source_text()),
                    );
                }
                Some(_) => {
                    // Initialized to something other than a fresh empty aggregate.
                    self.conflict.insert(id.name.to_string());
                }
                None => {}
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }

    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        // A bare `C = …` reassignment disqualifies the candidate.
        if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(t) = &node.left {
            self.conflict.insert(t.name.to_string());
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_conditional_expression(&mut self, node: &oxc_ast::ast::ConditionalExpression<'a>) {
        if let Some((cache, spans)) = match_memo_ternary(node) {
            self.memo_spans.entry(cache).or_default().extend(spans);
        }
        oxc_ast_visit::walk::walk_conditional_expression(self, node);
    }
}

/// A fresh, empty aggregate initializer (`{}`, `[]`, or `Object.create(null)`).
fn is_empty_cache_init(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::ObjectExpression(o) => o.properties.is_empty(),
        Expression::ArrayExpression(a) => a.elements.is_empty(),
        Expression::CallExpression(c) => {
            // Object.create(null)
            matches!(&c.callee, Expression::StaticMemberExpression(m)
                if matches!(&m.object, Expression::Identifier(o) if o.name == "Object")
                    && m.property.name == "create")
                && c.arguments.len() == 1
                && matches!(
                    c.arguments.first().and_then(|a| a.as_expression()),
                    Some(Expression::NullLiteral(_))
                )
        }
        _ => false,
    }
}

/// `(cacheName, keyText, span)` of a member-read `C[k]` / `C.k` whose object is a
/// bare identifier — `None` for any other shape.
fn read_member_cache_key(e: &Expression<'_>) -> Option<(String, String, oxc_span::Span)> {
    match e {
        Expression::ComputedMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            let mut cg = Codegen::new();
            cg.print_expression(&m.expression);
            Some((obj.name.to_string(), cg.into_source_text(), m.span))
        }
        Expression::StaticMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            Some((obj.name.to_string(), m.property.name.to_string(), m.span))
        }
        _ => None,
    }
}

/// Same as [`read_member_cache_key`] for a *write target* `C[k] = …` / `C.k = …`.
fn target_member_cache_key(
    t: &oxc_ast::ast::AssignmentTarget<'_>,
) -> Option<(String, String, oxc_span::Span)> {
    use oxc_ast::ast::AssignmentTarget as T;
    match t {
        T::ComputedMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            let mut cg = Codegen::new();
            cg.print_expression(&m.expression);
            Some((obj.name.to_string(), cg.into_source_text(), m.span))
        }
        T::StaticMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            Some((obj.name.to_string(), m.property.name.to_string(), m.span))
        }
        _ => None,
    }
}

/// True if `e` is `undefined` or `void <anything>` (the two cache-miss sentinels).
fn is_undefined_ish(e: &Expression<'_>) -> bool {
    match e {
        Expression::Identifier(id) => id.name == "undefined",
        Expression::UnaryExpression(u) => {
            matches!(u.operator, oxc_syntax::operator::UnaryOperator::Void)
        }
        _ => false,
    }
}

/// Match a cache-miss/hit test `C[k] <eq> undefined` (either operand order). On
/// success returns `(cache, key, memberSpan, is_miss_test)` where `is_miss_test`
/// is true for `===`/`==` against undefined (the *miss* arm is then the `then`
/// branch) and false for `!==`/`!=` (the *miss* arm is the `else` branch).
fn match_miss_test(test: &Expression<'_>) -> Option<(String, String, oxc_span::Span, bool)> {
    use oxc_syntax::operator::BinaryOperator as B;
    let Expression::BinaryExpression(b) = test else {
        return None;
    };
    let is_miss = match b.operator {
        B::StrictEquality | B::Equality => true,
        B::StrictInequality | B::Inequality => false,
        _ => return None,
    };
    // One side is the cache member, the other is undefined-ish.
    let member = if is_undefined_ish(&b.right) {
        &b.left
    } else if is_undefined_ish(&b.left) {
        &b.right
    } else {
        return None;
    };
    let (c, k, span) = read_member_cache_key(member)?;
    Some((c, k, span, is_miss))
}

/// Recognize a memo ternary `C[k] === undefined ? (C[k] = miss) : C[k]` (and the
/// `void 0` / `!==` polarities). Returns the cache name and the spans of the three
/// `C[k]` member accesses (so the detector can confirm they account for every
/// reference to `C`). The key `k` and cache `C` must be identical in all three.
fn match_memo_ternary(
    cond: &oxc_ast::ast::ConditionalExpression<'_>,
) -> Option<(String, [oxc_span::Span; 3])> {
    let (tc, tk, tspan, is_miss_test) = match_miss_test(&cond.test)?;
    // The miss arm (the `C[k] = miss` write) is `then` for a miss test, `else` for
    // a hit test; the other arm is the warm read `C[k]`.
    let (write_arm, read_arm) = if is_miss_test {
        (&cond.consequent, &cond.alternate)
    } else {
        (&cond.alternate, &cond.consequent)
    };
    let Expression::AssignmentExpression(a) = write_arm else {
        return None;
    };
    if !matches!(a.operator, oxc_syntax::operator::AssignmentOperator::Assign) {
        return None;
    }
    let (wc, wk, wspan) = target_member_cache_key(&a.left)?;
    let (rc, rk, rspan) = read_member_cache_key(read_arm)?;
    if tc == wc && wc == rc && tk == wk && wk == rk {
        Some((tc, [tspan, wspan, rspan]))
    } else {
        None
    }
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
///
/// `excluded` names (mutable/aliased aggregate tables proven unsafe by the SSA
/// escape analysis) are treated as conflicts: they are never candidates, and the
/// groundedness fixpoint therefore also drops anything that references them (e.g.
/// an identity alias `b = t`), so the emitted set stays dependency-closed.
fn scan_invariants(program: &Program<'_>, excluded: &HashSet<String>) -> Vec<(String, String)> {
    let mut c = DataCollector::default();
    c.visit_program(program);

    let cand: HashMap<String, (String, HashSet<String>)> = c
        .value
        .iter()
        .filter(|(n, _)| {
            c.counts.get(*n).copied() == Some(1)
                && c.writes.get(*n).copied() == Some(1)
                && !c.conflict.contains(*n)
                && !excluded.contains(*n)
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
            let Some((_, ops)) = cand.get(n) else {
                continue;
            };
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

    emit.into_iter()
        .map(|n| (n.clone(), cand[&n].0.clone()))
        .collect()
}

#[derive(Default)]
struct DataCollector {
    counts: HashMap<String, usize>, // binding occurrences
    writes: HashMap<String, usize>, // write occurrences
    value: HashMap<String, (String, HashSet<String>)>, // name -> (decl, operands)
    conflict: HashSet<String>,      // non-invariant / ambiguous
    order: Vec<String>,             // first-seen order
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
            if matches!(
                node.operator,
                oxc_syntax::operator::AssignmentOperator::Assign
            ) {
                self.value(t.name.as_str(), &node.right);
            } else {
                self.conflict.insert(t.name.to_string()); // compound assignment
            }
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &oxc_ast::ast::UpdateExpression<'a>) {
        if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &node.argument
        {
            self.write(id.name.as_str());
            self.conflict.insert(id.name.to_string()); // ++ / -- → not constant
        }
        oxc_ast_visit::walk::walk_update_expression(self, node);
    }
}

#[allow(clippy::too_many_arguments)]
fn build_preamble<'a>(
    tops: &[&Statement<'a>],
    analysis: &Analysis,
    aliases: &HashMap<String, String>,
    allowed: &HashSet<String>,
    local_data: &[(String, String)],
    nested_funcs: &[Statement<'a>],
    cache_decls: &[(String, String)],
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
    // Pure decoder functions lifted from nested scopes (Tier C/D). Declarations
    // hoist, so they may safely precede the data tables they index (the tables are
    // appended as text below; a decoder only reads them when *called*, which never
    // happens during setup).
    for stmt in nested_funcs {
        body.push(stmt.clone_in(allocator));
    }
    let prog = ast.program(
        SPAN,
        SourceType::default(),
        "",
        ast.vec(),
        None,
        ast.vec(),
        body,
    );
    let mut code = Codegen::new().build(&prog).code;

    // Pure IIFEs that manipulate a decoder (the array *rotation* — crucially,
    // even when nested inside otherwise-impure init code). Running them in the
    // sandbox is harmless (preamble is throwaway) and is what makes a rotated
    // decoder return the *correct* strings.
    let mut iifes = IifeCollector {
        allowed,
        pure_funcs: &analysis.pure_funcs,
        out: Vec::new(),
    };
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

    // Private memo caches (`var fu = {};`). Emitted last (after the tables they
    // sit beside) so each starts as a fresh empty container at eval time, exactly
    // as in the real program. A memo decoder writes this inside the sandbox; the
    // write is invisible to the host. A redundant re-declaration (the cache may
    // also appear among the pure-data vars above) is harmless: it re-empties an
    // already-empty container in throwaway preamble code.
    for (_, decl) in cache_decls {
        code.push('\n');
        code.push_str(decl);
    }

    code
}

/// Whether a top-level statement is a pure-**data** binding the preamble should
/// emit. Functions are deliberately excluded here — they are emitted *only* by
/// `serialize_pure_funcs` (the single function emitter), so a pure data var and a
/// pure function never both claim the same statement. A var declaration qualifies
/// only when every declarator is a pure-data initializer in the pure set (a
/// function-valued declarator returns false and is left to the serializer).
fn statement_is_pure(stmt: &Statement<'_>, analysis: &Analysis) -> bool {
    match stmt {
        Statement::VariableDeclaration(vd) => {
            !vd.declarations.is_empty()
                && vd.declarations.iter().all(|d| {
                    matches!(&d.id, BindingPattern::BindingIdentifier(id)
                        if analysis.pureset.contains(id.name.as_str()))
                        && d.init.as_ref().is_some_and(is_pure_data)
                })
        }
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
    let mut c = AliasCollector {
        edges: HashMap::new(),
        disqualified: HashSet::new(),
    };
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
        *rhs = ctx.ast.expression_numeric_literal(
            span,
            n,
            None,
            oxc_syntax::number::NumberBase::Decimal,
        );
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
            // The call/member expression being lifted must reference only allowed
            // names and perform no free writes. We deliberately pass an empty cache
            // whitelist here: a bare cache read (`fu[k]`) must NOT be lifted (it
            // observes cache state), and a lifted call expression never writes a
            // cache itself — the memo write lives inside the (already-classified)
            // decoder body, evaluated in the sandbox.
            if !freevars_of_expr(&*expr).all_allowed(self.allowed, None, &HashSet::new()) {
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
fn num_literal<'a>(n: f64, span: oxc_span::Span, ctx: &mut TraverseCtx<'a, ()>) -> Expression<'a> {
    use oxc_syntax::number::NumberBase;
    if n.is_sign_negative() {
        let lit = ctx
            .ast
            .expression_numeric_literal(span, -n, None, NumberBase::Decimal);
        ctx.ast.expression_unary(
            span,
            oxc_syntax::operator::UnaryOperator::UnaryNegation,
            lit,
        )
    } else {
        ctx.ast
            .expression_numeric_literal(span, n, None, NumberBase::Decimal)
    }
}
