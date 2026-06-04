//! Whole-program write-once constant resolution.
//!
//! A symbol `X` resolves to a constant `C` when **all** hold (sufficient,
//! conservative — bail when unsure):
//!
//!  1. **Single value-establishing write.** Exactly one declarator-init or bare
//!     `X = <expr>` assignment program-wide; no compound/update/delete/
//!     destructuring/for-head write anywhere.
//!  2. **The write's RHS is an inlineable literal.**
//!  3. **Every read happens-after the write** — proven by [`OrderingOracle`].
//!
//! (Value-can't-be-mutated-through-an-alias is automatic here: the resolved value
//! is always an immutable scalar literal, so aliasing is moot — that condition
//! matters only for the aggregate/array axis, handled separately.)
//!
//! This is the engine behind `global_const_inline` (which materializes resolved
//! reads to literals + DCEs the dead table) and, via [`ConstResolver::resolve_int`],
//! the unflatteners (which fold symbolic `case X:` labels and `f(X, …)` entries
//! without mutating the AST first).

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_span::Span;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;

use crate::ir::ordering::OrderingOracle;
use crate::util::is_inlineable_literal;

/// A whole-program map from symbol to its resolved constant literal.
pub struct ConstResolver<'a> {
    accepted: HashMap<SymbolId, Expression<'a>>,
    /// For `GCI_DEBUG`: how many symbols had a single literal write and were not
    /// disqualified (the denominator of the inlined/candidate ratio).
    single_lit_candidates: usize,
}

impl<'a> ConstResolver<'a> {
    /// Build the resolver: census the program, then accept every symbol whose
    /// single literal write provably happens-before all its reads.
    pub fn build(program: &Program<'a>, scoping: &Scoping, allocator: &'a Allocator) -> Self {
        let mut census = Census::new(scoping, allocator);
        census.visit_program(program);

        let single_lit_candidates = census
            .value_writes
            .iter()
            .filter(|(s, n)| {
                **n == 1 && census.literal.contains_key(s) && !census.disqualified.contains(s)
            })
            .count();

        let accepted = analyze(program, scoping, &census, allocator);
        ConstResolver {
            accepted,
            single_lit_candidates,
        }
    }

    /// The resolved literal for `sym`, if any. (Consumed by the unflatteners in S3.)
    #[allow(dead_code)]
    pub fn lit(&self, sym: SymbolId) -> Option<&Expression<'a>> {
        self.accepted.get(&sym)
    }

    /// The full accepted map (for materialization clients like `global_const_inline`).
    pub fn accepted(&self) -> &HashMap<SymbolId, Expression<'a>> {
        &self.accepted
    }

    /// Count of single-literal-write candidates (debug/metrics).
    pub fn single_lit_candidates(&self) -> usize {
        self.single_lit_candidates
    }

    /// Resolve an expression that is an integer literal **or** a bare reference to
    /// a resolved symbol (including `-x`) to its concrete integer value. The
    /// drop-in replacement for a local `int_literal` that also sees cross-function
    /// constants. Returns `None` when the expression is not statically an integer.
    /// (Consumed by the unflatteners in S3.)
    #[allow(dead_code)]
    pub fn resolve_int(&self, expr: &Expression<'_>, scoping: &Scoping) -> Option<i64> {
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
                self.resolve_int(&u.argument, scoping).map(|v| -v)
            }
            Expression::Identifier(id) => {
                let sym = id
                    .reference_id
                    .get()
                    .and_then(|rid| scoping.get_reference(rid).symbol_id())?;
                expr_to_int(self.accepted.get(&sym)?)
            }
            _ => None,
        }
    }
}

/// Read an integer out of a resolved literal expression (the only shapes the
/// resolver ever stores are inlineable literals).
#[allow(dead_code)]
fn expr_to_int(expr: &Expression<'_>) -> Option<i64> {
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
            expr_to_int(&u.argument).map(|v| -v)
        }
        _ => None,
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
// Analysis: decide which symbols are soundly resolvable
// ---------------------------------------------------------------------------

fn analyze<'a>(
    program: &Program<'a>,
    scoping: &Scoping,
    census: &Census<'a, '_>,
    allocator: &'a Allocator,
) -> HashMap<SymbolId, Expression<'a>> {
    let oracle =
        OrderingOracle::build(program, scoping, &census.fn_scope_symbol, &census.raw_calls);

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

        // Every read must happen-after the write (the ordering proof).
        let reads = census.reads.get(&sym).map(Vec::as_slice).unwrap_or(&[]);
        if oracle.writes_before_all_reads(scoping, wf_sym, reads) {
            accepted.insert(sym, census.literal[&sym].clone_in(allocator));
        }
    }

    accepted
}
