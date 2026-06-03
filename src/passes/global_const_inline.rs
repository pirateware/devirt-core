//! Cross-function write-once constant inlining.
//!
//! Control-flow-flattened VMs hide their opcode/case labels as a table of
//! write-once integers, assigned in an *init* function and read as switch labels
//! in the dispatchers:
//!
//! ```text
//! var L1, L2;                              // declared once at IIFE scope, no init
//! function init(){ L1 = 19; L2 = 187; }    // each written exactly once, a literal
//! init();                                   // run unconditionally, once, up top
//! var run = function(s){ while(s){ switch(s){ case L1: …; case L2: …; } } };
//! ```
//!
//! Replacing every read of `L1` with `19` turns the dispatcher into a plain
//! numeric switch (which `unflatten` and const-fold can then act on) and lets DCE
//! delete the now-dead table and the emptied init function.
//!
//! Why a new pass: `assigned-once-inline` is scoped to one function — it records
//! only declarator inits (not bare `X = lit`), refuses any symbol with a write
//! reference, and requires the read in the *same* function as the def. Here the
//! def lives in `init` and the reads live in the dispatcher.
//!
//! ## Soundness — the ordering proof is the crux
//!
//! A read may only be replaced when the write is **guaranteed to have executed
//! before it**; otherwise the read could observe the pre-init `undefined`. We
//! inline a binding `X` only when ALL hold:
//!
//!  1. **Single literal write.** Exactly one value-establishing write program-wide,
//!     RHS an inlineable literal. No compound/update/delete/destructuring/for-head
//!     write anywhere.
//!  2. **Unconditional write in an init function.** The write runs at conditional
//!     depth 0 inside a function `F` (not under any `if`/loop/`switch`/`try`/`&&`/
//!     `?:`/`with`), so it runs exactly once whenever `F` is called.
//!  3. **`F` is an init function.** `F` is called *exactly once* in the whole
//!     program, via an unconditional bare `F();` statement that is a direct child
//!     of a *region* `S` (the program body, or a top-level IIFE body), at index `c`.
//!  4. **Every read happens-after the write.** For each read of `X`: a top-level
//!     statement of `S` must have index > `c`; a read inside a nested function `G`
//!     requires `G` not be reachable from the "setup" — the transitive set of
//!     functions invoked by `S`'s statements at index ≤ `c`. A function unreachable
//!     from setup cannot run until the setup prefix completes, i.e. after the write.
//!
//! Symbol-precise throughout; the replacement substitutes a literal, so there is
//! no shadowing hazard. Anything we cannot prove is left untouched.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_span::{GetSpan, Span};
use oxc_syntax::scope::ScopeFlags;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::{build_scoping, is_inlineable_literal};

pub struct GlobalConstInline;

impl Pass for GlobalConstInline {
    fn name(&self) -> &'static str {
        "global-const-inline"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        let mut census = Census::new(&scoping, allocator);
        census.visit_program(program);

        let accepted = analyze(program, &scoping, &census, allocator);

        if std::env::var("GCI_DEBUG").is_ok() {
            // Coverage, so "didn't fire" is never mistaken for "covered everything":
            // how many single-literal-write candidates were inlinable vs. left
            // (writer not in a top-level init function, or an unsafe read ordering).
            let single_lit = census
                .value_writes
                .iter()
                .filter(|(s, n)| {
                    **n == 1 && census.literal.contains_key(s) && !census.disqualified.contains(s)
                })
                .count();
            eprintln!(
                "[gci] inlined={} of single-literal-write candidates={} (rest: nested/non-top init or unprovable order)",
                accepted.len(),
                single_lit,
            );
        }

        if accepted.is_empty() {
            return false;
        }

        let mut replacer = Replacer {
            accepted: &accepted,
            changed: false,
        };
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        replacer.changed
    }
}

// ---------------------------------------------------------------------------
// Census (read-only)
// ---------------------------------------------------------------------------

struct Census<'a, 's> {
    scoping: &'s Scoping,
    allocator: &'a Allocator,
    fn_stack: Vec<ScopeId>,
    cond_depth: usize,

    value_writes: HashMap<SymbolId, usize>,
    literal: HashMap<SymbolId, Expression<'a>>,
    disqualified: HashSet<SymbolId>,
    /// Symbol → the function scope of its single *unconditional* literal write.
    write_fn: HashMap<SymbolId, ScopeId>,
    reads: HashMap<SymbolId, Vec<(ScopeId, Span)>>,

    /// Function scope → the symbol it is bound to.
    fn_scope_symbol: HashMap<ScopeId, SymbolId>,
    /// `(caller function scope, callee symbol)` for every resolved call.
    raw_calls: Vec<(ScopeId, SymbolId)>,
    call_count: HashMap<SymbolId, usize>,
}

impl<'a, 's> Census<'a, 's> {
    fn new(scoping: &'s Scoping, allocator: &'a Allocator) -> Self {
        Census {
            scoping,
            allocator,
            fn_stack: Vec::new(),
            cond_depth: 0,
            value_writes: HashMap::new(),
            literal: HashMap::new(),
            disqualified: HashSet::new(),
            write_fn: HashMap::new(),
            reads: HashMap::new(),
            fn_scope_symbol: HashMap::new(),
            raw_calls: Vec::new(),
            call_count: HashMap::new(),
        }
    }

    fn cur_fn(&self) -> ScopeId {
        *self.fn_stack.last().expect("inside program")
    }

    fn ref_sym(&self, id: &IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = id.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }

    fn record_value_write(&mut self, sym: SymbolId, literal_rhs: Option<&Expression<'a>>) {
        *self.value_writes.entry(sym).or_default() += 1;
        match literal_rhs {
            Some(rhs) => {
                self.literal.insert(sym, rhs.clone_in(self.allocator));
                if self.cond_depth == 0 {
                    self.write_fn.insert(sym, self.cur_fn());
                }
            }
            // A non-literal value write leaves nothing to inline.
            None => {
                self.disqualified.insert(sym);
            }
        }
    }

    fn in_conditional(&mut self, f: impl FnOnce(&mut Self)) {
        self.cond_depth += 1;
        f(self);
        self.cond_depth -= 1;
    }

    fn enter_fn(&mut self, scope: Option<ScopeId>, f: impl FnOnce(&mut Self)) {
        match scope {
            Some(scope) => {
                self.fn_stack.push(scope);
                let saved = std::mem::replace(&mut self.cond_depth, 0);
                f(self);
                self.cond_depth = saved;
                self.fn_stack.pop();
            }
            None => f(self),
        }
    }
}

impl<'a, 's> Visit<'a> for Census<'a, 's> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.fn_stack.push(self.scoping.root_scope_id());
        walk::walk_program(self, program);
        self.fn_stack.pop();
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: ScopeFlags) {
        if let (Some(scope), Some(id)) = (func.scope_id.get(), &func.id) {
            if let Some(sym) = id.symbol_id.get() {
                self.fn_scope_symbol.insert(scope, sym);
            }
        }
        self.enter_fn(func.scope_id.get(), |s| walk::walk_function(s, func, flags));
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        self.enter_fn(arrow.scope_id.get(), |s| {
            walk::walk_arrow_function_expression(s, arrow)
        });
    }

    fn visit_variable_declarator(&mut self, node: &VariableDeclarator<'a>) {
        match &node.id {
            BindingPattern::BindingIdentifier(id) => {
                if let (Some(sym), Some(init)) = (id.symbol_id.get(), &node.init) {
                    match init {
                        Expression::FunctionExpression(f) => {
                            if let Some(s) = f.scope_id.get() {
                                self.fn_scope_symbol.insert(s, sym);
                            }
                        }
                        Expression::ArrowFunctionExpression(a) => {
                            if let Some(s) = a.scope_id.get() {
                                self.fn_scope_symbol.insert(s, sym);
                            }
                        }
                        _ => {}
                    }
                    let lit = is_inlineable_literal(init).then_some(init);
                    self.record_value_write(sym, lit);
                }
            }
            other => collect_pattern_symbols(other, &mut self.disqualified),
        }
        walk::walk_variable_declarator(self, node);
    }

    fn visit_assignment_expression(&mut self, node: &AssignmentExpression<'a>) {
        use oxc_syntax::operator::AssignmentOperator;
        match &node.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                if let Some(sym) = self.ref_sym(id) {
                    if node.operator == AssignmentOperator::Assign {
                        let lit = is_inlineable_literal(&node.right).then_some(&node.right);
                        self.record_value_write(sym, lit);
                    } else {
                        self.disqualified.insert(sym);
                    }
                }
            }
            AssignmentTarget::ArrayAssignmentTarget(_)
            | AssignmentTarget::ObjectAssignmentTarget(_) => {
                let mut c = RefCollector {
                    scoping: self.scoping,
                    out: &mut self.disqualified,
                };
                c.visit_assignment_target(&node.left);
            }
            _ => {}
        }
        walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &UpdateExpression<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &node.argument {
            if let Some(sym) = self.ref_sym(id) {
                self.disqualified.insert(sym);
            }
        }
        walk::walk_update_expression(self, node);
    }

    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &node.callee {
            if let Some(sym) = self.ref_sym(callee) {
                *self.call_count.entry(sym).or_default() += 1;
                self.raw_calls.push((self.cur_fn(), sym));
            }
        }
        walk::walk_call_expression(self, node);
    }

    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if let Some(rid) = id.reference_id.get() {
            let r = self.scoping.get_reference(rid);
            if r.is_read() {
                if let Some(sym) = r.symbol_id() {
                    let cur = self.cur_fn();
                    self.reads.entry(sym).or_default().push((cur, id.span));
                }
            }
        }
    }

    // Conditional/looping contexts: a write under any of these is not a single
    // unconditional write. Wrapping the whole node is sound (at worst conservative).
    fn visit_if_statement(&mut self, node: &IfStatement<'a>) {
        self.in_conditional(|s| walk::walk_if_statement(s, node));
    }
    fn visit_while_statement(&mut self, node: &WhileStatement<'a>) {
        self.in_conditional(|s| walk::walk_while_statement(s, node));
    }
    fn visit_do_while_statement(&mut self, node: &DoWhileStatement<'a>) {
        self.in_conditional(|s| walk::walk_do_while_statement(s, node));
    }
    fn visit_for_statement(&mut self, node: &ForStatement<'a>) {
        if let Some(ForStatementInit::VariableDeclaration(d)) = &node.init {
            collect_decl_symbols(d, &mut self.disqualified);
        }
        self.in_conditional(|s| walk::walk_for_statement(s, node));
    }
    fn visit_for_in_statement(&mut self, node: &ForInStatement<'a>) {
        collect_for_head(&node.left, self.scoping, &mut self.disqualified);
        self.in_conditional(|s| walk::walk_for_in_statement(s, node));
    }
    fn visit_for_of_statement(&mut self, node: &ForOfStatement<'a>) {
        collect_for_head(&node.left, self.scoping, &mut self.disqualified);
        self.in_conditional(|s| walk::walk_for_of_statement(s, node));
    }
    fn visit_switch_statement(&mut self, node: &SwitchStatement<'a>) {
        self.in_conditional(|s| walk::walk_switch_statement(s, node));
    }
    fn visit_try_statement(&mut self, node: &TryStatement<'a>) {
        self.in_conditional(|s| walk::walk_try_statement(s, node));
    }
    fn visit_with_statement(&mut self, node: &WithStatement<'a>) {
        self.in_conditional(|s| walk::walk_with_statement(s, node));
    }
    fn visit_conditional_expression(&mut self, node: &ConditionalExpression<'a>) {
        self.in_conditional(|s| walk::walk_conditional_expression(s, node));
    }
    fn visit_logical_expression(&mut self, node: &LogicalExpression<'a>) {
        // `a && (X = …)` writes conditionally; wrapping the whole node is sound.
        self.in_conditional(|s| walk::walk_logical_expression(s, node));
    }
}

/// Collects the symbols referenced anywhere in an assignment target (for
/// destructuring disqualification).
struct RefCollector<'o, 's> {
    scoping: &'s Scoping,
    out: &'o mut HashSet<SymbolId>,
}
impl<'a, 'o, 's> Visit<'a> for RefCollector<'o, 's> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if let Some(rid) = id.reference_id.get() {
            if let Some(sym) = self.scoping.get_reference(rid).symbol_id() {
                self.out.insert(sym);
            }
        }
    }
}

fn collect_pattern_symbols(pat: &BindingPattern<'_>, out: &mut HashSet<SymbolId>) {
    struct P<'o>(&'o mut HashSet<SymbolId>);
    impl<'a, 'o> Visit<'a> for P<'o> {
        fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
            if let Some(sym) = id.symbol_id.get() {
                self.0.insert(sym);
            }
        }
    }
    P(out).visit_binding_pattern(pat);
}

fn collect_decl_symbols(decl: &VariableDeclaration<'_>, out: &mut HashSet<SymbolId>) {
    struct P<'o>(&'o mut HashSet<SymbolId>);
    impl<'a, 'o> Visit<'a> for P<'o> {
        fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
            if let Some(sym) = id.symbol_id.get() {
                self.0.insert(sym);
            }
        }
    }
    P(out).visit_variable_declaration(decl);
}

fn collect_for_head(left: &ForStatementLeft<'_>, scoping: &Scoping, out: &mut HashSet<SymbolId>) {
    match left {
        ForStatementLeft::VariableDeclaration(d) => collect_decl_symbols(d, out),
        ForStatementLeft::AssignmentTargetIdentifier(id) => {
            if let Some(rid) = id.reference_id.get() {
                if let Some(sym) = scoping.get_reference(rid).symbol_id() {
                    out.insert(sym);
                }
            }
        }
        _ => {
            let mut c = RefCollector { scoping, out };
            c.visit_assignment_target_maybe(left);
        }
    }
}

impl<'o, 's> RefCollector<'o, 's> {
    fn visit_assignment_target_maybe(&mut self, left: &ForStatementLeft<'_>) {
        // Destructuring for-head targets; visit as assignment targets.
        match left {
            ForStatementLeft::ArrayAssignmentTarget(_)
            | ForStatementLeft::ObjectAssignmentTarget(_) => {
                if let Some(t) = left.as_assignment_target() {
                    self.visit_assignment_target(t);
                }
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Analysis: decide which symbols are soundly inlinable
// ---------------------------------------------------------------------------

/// A region whose top-level statements may host the unconditional init calls.
struct Region<'b, 'a> {
    scope: ScopeId,
    stmts: &'b [Statement<'a>],
}

fn analyze<'a>(
    program: &Program<'a>,
    scoping: &Scoping,
    census: &Census<'a, '_>,
    allocator: &'a Allocator,
) -> HashMap<SymbolId, Expression<'a>> {
    // Reverse map: symbol → its function scope (for resolving callees).
    let sym_to_scope: HashMap<SymbolId, ScopeId> = census
        .fn_scope_symbol
        .iter()
        .map(|(scope, sym)| (*sym, *scope))
        .collect();

    // Call graph: caller function scope → callee function scopes.
    let mut edges: HashMap<ScopeId, HashSet<ScopeId>> = HashMap::new();
    for (caller, callee_sym) in &census.raw_calls {
        if let Some(callee_scope) = sym_to_scope.get(callee_sym) {
            edges.entry(*caller).or_default().insert(*callee_scope);
        }
    }

    // Regions: the program body and every top-level IIFE body.
    let mut regions: Vec<Region> = vec![Region {
        scope: scoping.root_scope_id(),
        stmts: &program.body,
    }];
    for stmt in &program.body {
        if let Some((scope, body)) = iife_region(stmt) {
            regions.push(Region { scope, stmts: body });
        }
    }

    let mut accepted: HashMap<SymbolId, Expression<'a>> = HashMap::new();

    for (&sym, &writes) in &census.value_writes {
        if writes != 1 || census.disqualified.contains(&sym) || !census.literal.contains_key(&sym) {
            continue;
        }
        let Some(&wf) = census.write_fn.get(&sym) else {
            continue; // write was conditional or not in a function
        };
        // The writer function must be called exactly once, unconditionally, as a
        // direct child of a region.
        let Some(&wf_sym) = census.fn_scope_symbol.get(&wf) else {
            continue; // anonymous writer fn → no name to call by
        };
        if census.call_count.get(&wf_sym).copied() != Some(1) {
            continue;
        }
        let Some((region, c)) = find_init_call(&regions, scoping, wf_sym) else {
            continue;
        };

        // Setup-reachable functions: closure of everything called by region
        // statements at index ≤ c.
        let reachable = setup_reachable(region, c, scoping, &edges, &sym_to_scope);

        // Every read must happen-after the write.
        let reads = census.reads.get(&sym).map(Vec::as_slice).unwrap_or(&[]);
        let safe = reads.iter().all(|&(ef, span)| {
            if ef == region.scope {
                // Top-level read of S: must be in a statement after the init call.
                match child_index(region.stmts, span) {
                    Some(idx) => idx > c,
                    None => false,
                }
            } else {
                // Read inside a nested function: safe iff that function cannot run
                // during setup.
                !reachable.contains(&ef)
            }
        });
        if safe {
            accepted.insert(sym, census.literal[&sym].clone_in(allocator));
        }
    }

    accepted
}

/// Find the single init call `F();` for `wf_sym`: a bare, no-arg call that is a
/// direct child of some region. Returns the region and the call's statement index.
fn find_init_call<'b, 'a>(
    regions: &'b [Region<'b, 'a>],
    scoping: &Scoping,
    wf_sym: SymbolId,
) -> Option<(&'b Region<'b, 'a>, usize)> {
    for region in regions {
        for (i, stmt) in region.stmts.iter().enumerate() {
            if let Some(callee) = bare_call_callee(stmt) {
                if let Some(rid) = callee.reference_id.get() {
                    if scoping.get_reference(rid).symbol_id() == Some(wf_sym) {
                        return Some((region, i));
                    }
                }
            }
        }
    }
    None
}

/// Transitive set of function scopes invoked while executing `region`'s statements
/// at index ≤ `c` (the setup prefix).
fn setup_reachable(
    region: &Region<'_, '_>,
    c: usize,
    scoping: &Scoping,
    edges: &HashMap<ScopeId, HashSet<ScopeId>>,
    sym_to_scope: &HashMap<SymbolId, ScopeId>,
) -> HashSet<ScopeId> {
    // Roots: every function called anywhere within statements 0..=c.
    let mut roots: Vec<ScopeId> = Vec::new();
    let mut collector = CallCollector {
        scoping,
        sym_to_scope,
        out: &mut roots,
    };
    for stmt in region.stmts.iter().take(c + 1) {
        collector.visit_statement(stmt);
    }

    let mut reachable: HashSet<ScopeId> = HashSet::new();
    let mut stack = roots;
    while let Some(scope) = stack.pop() {
        if reachable.insert(scope) {
            if let Some(callees) = edges.get(&scope) {
                stack.extend(callees.iter().copied());
            }
        }
    }
    reachable
}

struct CallCollector<'o, 's> {
    scoping: &'s Scoping,
    sym_to_scope: &'s HashMap<SymbolId, ScopeId>,
    out: &'o mut Vec<ScopeId>,
}
impl<'a, 'o, 's> Visit<'a> for CallCollector<'o, 's> {
    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        if let Expression::Identifier(callee) = &node.callee {
            if let Some(rid) = callee.reference_id.get() {
                if let Some(sym) = self.scoping.get_reference(rid).symbol_id() {
                    if let Some(scope) = self.sym_to_scope.get(&sym) {
                        self.out.push(*scope);
                    }
                }
            }
        }
        walk::walk_call_expression(self, node);
    }
}

/// If `stmt` is `Ident();` (bare, no arguments), return the callee identifier.
fn bare_call_callee<'b, 'a>(stmt: &'b Statement<'a>) -> Option<&'b IdentifierReference<'a>> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::CallExpression(call) = &es.expression else {
        return None;
    };
    if !call.arguments.is_empty() {
        return None;
    }
    match &call.callee {
        Expression::Identifier(id) => Some(id),
        _ => None,
    }
}

/// Index of the direct-child statement of `stmts` whose span contains `span`.
fn child_index(stmts: &[Statement<'_>], span: Span) -> Option<usize> {
    stmts
        .iter()
        .position(|s| s.span().start <= span.start && span.end <= s.span().end)
}

/// If `stmt` is a top-level IIFE expression statement, return its body scope and
/// statements.
fn iife_region<'b, 'a>(stmt: &'b Statement<'a>) -> Option<(ScopeId, &'b [Statement<'a>])> {
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
        Expression::FunctionExpression(f) => {
            let scope = f.scope_id.get()?;
            f.body.as_ref().map(|b| (scope, b.statements.as_slice()))
        }
        Expression::ArrowFunctionExpression(a) => {
            let scope = a.scope_id.get()?;
            Some((scope, a.body.statements.as_slice()))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Replacement
// ---------------------------------------------------------------------------

struct Replacer<'a, 'm> {
    accepted: &'m HashMap<SymbolId, Expression<'a>>,
    changed: bool,
}

impl<'a, 'm> Traverse<'a, ()> for Replacer<'a, 'm> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Expression::Identifier(idref) = expr else {
            return;
        };
        let Some(rid) = idref.reference_id.get() else {
            return;
        };
        let reference = ctx.scoping().get_reference(rid);
        if !reference.is_read() {
            return;
        }
        let Some(sym) = reference.symbol_id() else {
            return;
        };
        if let Some(val) = self.accepted.get(&sym) {
            *expr = val.clone_in(ctx.ast.allocator);
            self.changed = true;
        }
    }
}
