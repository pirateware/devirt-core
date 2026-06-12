//! Whole-program write-once constant resolution.
//!
//! A symbol `X` resolves to a constant `C` when **all** hold (sufficient,
//! conservative — bail when unsure):
//!
//!  1. **Single value-establishing write.** Exactly one declarator-init or bare
//!     `X = <expr>` assignment program-wide; no compound/update/delete/
//!     destructuring/for-head write anywhere.
//!  2. **The write's RHS is constant.** Either an inlineable literal, *or* a
//!     **pure integer expression** over already-resolved symbols (`var b = a + 1`
//!     once `a` resolves). The pure-expression case is evaluated in a **bounded
//!     fixpoint** entirely in Rust over integer operands — deterministic by
//!     construction (no host state, no impure calls), so no sandbox/nondeterminism
//!     guard is needed. An impure / non-integer / not-yet-resolved RHS declines.
//!  3. **Every read happens-after the write** — proven by [`OrderingOracle`]. For a
//!     pure-expression `X` whose RHS reads `a`, that read is itself one of `a`'s
//!     reads, so `a`'s own happens-before proof already covers it.
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

/// Receiver-mutating method names. A call to one through a binding
/// (`a.push(…)`) mutates it, disqualifying its single-literal initializer from
/// being folded in. Mirrors the lists in `decoder_lift` / `ir::analysis` — kept
/// local so this module owns its own soundness reasoning.
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
];

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
// Pure integer expression classification + evaluation (B4)
// ---------------------------------------------------------------------------

/// Operators on integers that produce a deterministic integer and never call user
/// code or observe host state. (`/` is excluded — non-integer / NaN / Infinity
/// results are not inlineable integers; `**` likewise can overflow to non-integer.)
fn pure_int_binop(op: oxc_syntax::operator::BinaryOperator) -> bool {
    use oxc_syntax::operator::BinaryOperator::*;
    matches!(
        op,
        Addition
            | Subtraction
            | Multiplication
            | Remainder
            | BitwiseAnd
            | BitwiseOR
            | BitwiseXOR
            | ShiftLeft
            | ShiftRight
            | ShiftRightZeroFill
            | Equality
            | StrictEquality
            | Inequality
            | StrictInequality
            | LessThan
            | LessEqualThan
            | GreaterThan
            | GreaterEqualThan
    )
}

/// Whether `expr` is a **pure integer expression**: built only from numeric
/// literals, parentheses, the unary operators `-`/`~`/`+`, the binary operators in
/// [`pure_int_binop`], and **bare identifier reads** (whose symbols this resolver
/// may later prove constant). No calls, member access, strings, objects, logical
/// short-circuits, or host references — so evaluation is total and deterministic.
///
/// A bare identifier with no resolvable symbol (a free/global like `undefined`) is
/// rejected: we can only evaluate it if it resolves to a tracked integer.
fn is_pure_int_expr(expr: &Expression<'_>, scoping: &Scoping) -> bool {
    match expr {
        Expression::NumericLiteral(n) => n.value.fract() == 0.0 && n.value.is_finite(),
        Expression::ParenthesizedExpression(p) => is_pure_int_expr(&p.expression, scoping),
        Expression::Identifier(id) => id
            .reference_id
            .get()
            .and_then(|rid| scoping.get_reference(rid).symbol_id())
            .is_some(),
        Expression::UnaryExpression(u) => {
            use oxc_syntax::operator::UnaryOperator::*;
            matches!(u.operator, UnaryNegation | UnaryPlus | BitwiseNot)
                && is_pure_int_expr(&u.argument, scoping)
        }
        Expression::BinaryExpression(b) => {
            pure_int_binop(b.operator)
                && is_pure_int_expr(&b.left, scoping)
                && is_pure_int_expr(&b.right, scoping)
        }
        _ => false,
    }
}

/// Evaluate a pure integer expression, resolving each bare identifier through
/// `env` (the already-resolved symbol → integer map). Returns `None` if any
/// operand is unresolved or any sub-result is not a finite integer. Arithmetic
/// uses JS `f64` semantics then requires an integer result, so e.g. `5 % 2 == 1`
/// but a fractional/NaN/Infinity intermediate declines the whole expression.
fn eval_pure_int(
    expr: &Expression<'_>,
    scoping: &Scoping,
    env: &HashMap<SymbolId, i64>,
) -> Option<i64> {
    use oxc_syntax::operator::{BinaryOperator as B, UnaryOperator as U};
    match expr {
        Expression::NumericLiteral(n) if n.value.fract() == 0.0 && n.value.is_finite() => {
            Some(n.value as i64)
        }
        Expression::ParenthesizedExpression(p) => eval_pure_int(&p.expression, scoping, env),
        Expression::Identifier(id) => {
            let sym = id
                .reference_id
                .get()
                .and_then(|rid| scoping.get_reference(rid).symbol_id())?;
            env.get(&sym).copied()
        }
        Expression::UnaryExpression(u) => {
            let v = eval_pure_int(&u.argument, scoping, env)?;
            match u.operator {
                U::UnaryNegation => Some(-v),
                U::UnaryPlus => Some(v),
                U::BitwiseNot => Some(!(v as i32) as i64),
                _ => None,
            }
        }
        Expression::BinaryExpression(b) => {
            let l = eval_pure_int(&b.left, scoping, env)?;
            let r = eval_pure_int(&b.right, scoping, env)?;
            let bool_to_int = |x: bool| Some(x as i64);
            match b.operator {
                // Arithmetic uses f64 then re-checks integrality (JS semantics).
                B::Addition => f64_int(l as f64 + r as f64),
                B::Subtraction => f64_int(l as f64 - r as f64),
                B::Multiplication => f64_int(l as f64 * r as f64),
                B::Remainder => {
                    if r == 0 {
                        None // `x % 0` is NaN
                    } else {
                        f64_int((l as f64) % (r as f64))
                    }
                }
                // Bitwise/shift coerce to int32 in JS; mirror that exactly.
                B::BitwiseAnd => Some(((l as i32) & (r as i32)) as i64),
                B::BitwiseOR => Some(((l as i32) | (r as i32)) as i64),
                B::BitwiseXOR => Some(((l as i32) ^ (r as i32)) as i64),
                B::ShiftLeft => Some(((l as i32).wrapping_shl((r as u32) & 31)) as i64),
                B::ShiftRight => Some(((l as i32).wrapping_shr((r as u32) & 31)) as i64),
                B::ShiftRightZeroFill => Some(((l as u32).wrapping_shr((r as u32) & 31)) as i64),
                B::Equality | B::StrictEquality => bool_to_int(l == r),
                B::Inequality | B::StrictInequality => bool_to_int(l != r),
                B::LessThan => bool_to_int(l < r),
                B::LessEqualThan => bool_to_int(l <= r),
                B::GreaterThan => bool_to_int(l > r),
                B::GreaterEqualThan => bool_to_int(l >= r),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Accept an `f64` arithmetic result only if it is a finite integer.
fn f64_int(v: f64) -> Option<i64> {
    (v.fract() == 0.0 && v.is_finite()).then_some(v as i64)
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
    /// Symbol → its single write's RHS, when that RHS is a *pure integer
    /// expression* (not itself an inlineable literal). Resolved in a fixpoint once
    /// the symbols it reads resolve. The expression is stored cloned.
    pure_rhs: HashMap<SymbolId, Expression<'a>>,
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
            pure_rhs: HashMap::new(),
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

    /// Disqualify the *root binding* of a member expression (`a` for `a`, `a.b`,
    /// `a[i].c`) — the symbol whose value is mutated by a write/mutator through it.
    /// A base that is not rooted in a plain identifier (e.g. `f().x`) mutates
    /// nothing we track, so it is ignored.
    fn disqualify_member_base(&mut self, obj: &Expression<'a>) {
        let mut e = obj;
        loop {
            match e {
                Expression::Identifier(id) => {
                    if let Some(sym) = self.ref_sym(id) {
                        self.disqualified.insert(sym);
                    }
                    return;
                }
                Expression::StaticMemberExpression(m) => e = &m.object,
                Expression::ComputedMemberExpression(m) => e = &m.object,
                Expression::PrivateFieldExpression(m) => e = &m.object,
                _ => return,
            }
        }
    }

    fn record_value_write(&mut self, sym: SymbolId, rhs: &Expression<'a>) {
        *self.value_writes.entry(sym).or_default() += 1;
        if is_inlineable_literal(rhs) {
            self.literal.insert(sym, rhs.clone_in(self.allocator));
            if self.cond_depth == 0 {
                self.write_fn.insert(sym, self.cur_fn());
            }
        } else if is_pure_int_expr(rhs, self.scoping) {
            // A pure integer expression over literals / identifier reads. Kept for
            // the resolution fixpoint; the same ordering proof as a literal write
            // applies (the write must happen-before every read of `sym`).
            self.pure_rhs.insert(sym, rhs.clone_in(self.allocator));
            if self.cond_depth == 0 {
                self.write_fn.insert(sym, self.cur_fn());
            }
        } else {
            // A value write we can neither inline nor evaluate — nothing to do.
            self.disqualified.insert(sym);
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
                    self.record_value_write(sym, init);
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
                        self.record_value_write(sym, &node.right);
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
            // A member write (`a[k] = …`, `a.p = …`, compound or not) mutates `a`,
            // so its literal initializer is stale — disqualify the base binding.
            AssignmentTarget::StaticMemberExpression(m) => self.disqualify_member_base(&m.object),
            AssignmentTarget::ComputedMemberExpression(m) => self.disqualify_member_base(&m.object),
            AssignmentTarget::PrivateFieldExpression(m) => self.disqualify_member_base(&m.object),
            _ => {}
        }
        walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &UpdateExpression<'a>) {
        match &node.argument {
            SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                if let Some(sym) = self.ref_sym(id) {
                    self.disqualified.insert(sym);
                }
            }
            // `a[k]++` / `a.p++` mutates `a`.
            SimpleAssignmentTarget::StaticMemberExpression(m) => {
                self.disqualify_member_base(&m.object)
            }
            SimpleAssignmentTarget::ComputedMemberExpression(m) => {
                self.disqualify_member_base(&m.object)
            }
            _ => {}
        }
        walk::walk_update_expression(self, node);
    }

    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        match &node.callee {
            Expression::Identifier(callee) => {
                if let Some(sym) = self.ref_sym(callee) {
                    *self.call_count.entry(sym).or_default() += 1;
                    self.raw_calls.push((self.cur_fn(), sym));
                }
            }
            // A mutator method on a binding (`a.push(x)`, `a.splice(…)`, `m.set(…)`)
            // mutates its receiver, so the receiver's single literal initializer
            // (e.g. `const a = []`) is *stale* at later reads — disqualify it.
            // Without this, `const a = []; a.push(x); … a.length` folds `a` to the
            // empty literal and `a.length` to 0 (a soundness bug).
            Expression::StaticMemberExpression(m)
                if MUTATOR_METHODS.contains(&m.property.name.as_str()) =>
            {
                self.disqualify_member_base(&m.object);
            }
            Expression::ComputedMemberExpression(m) => match &m.expression {
                Expression::StringLiteral(s) if MUTATOR_METHODS.contains(&s.value.as_str()) => {
                    self.disqualify_member_base(&m.object)
                }
                // A statically-known non-mutator method name (`a["slice"]()`): no mutation.
                Expression::StringLiteral(_) => {}
                // Computed method name we can't read statically (`a[_dec("push")](x)`
                // before the decode lands) — could be a mutator; disqualify the base.
                _ => self.disqualify_member_base(&m.object),
            },
            _ => {}
        }
        walk::walk_call_expression(self, node);
    }

    fn visit_unary_expression(&mut self, node: &UnaryExpression<'a>) {
        // `delete a[k]` / `delete a.p` mutates `a`.
        if matches!(node.operator, oxc_syntax::operator::UnaryOperator::Delete) {
            self.disqualify_member_base(&node.argument);
        }
        walk::walk_unary_expression(self, node);
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

    // The shared per-symbol gate: a single write provably happening-before every
    // read. Identical for a literal write and a pure-expression write — the only
    // difference is *what value* the write establishes.
    let ordered_ok = |sym: SymbolId| -> bool {
        if census.value_writes.get(&sym).copied() != Some(1) || census.disqualified.contains(&sym) {
            return false;
        }
        let Some(&wf) = census.write_fn.get(&sym) else {
            return false; // write was conditional or not in a function
        };
        let Some(&wf_sym) = census.fn_scope_symbol.get(&wf) else {
            return false; // anonymous writer fn → no name to call by
        };
        if census.call_count.get(&wf_sym).copied() != Some(1) {
            return false;
        }
        let reads = census.reads.get(&sym).map(Vec::as_slice).unwrap_or(&[]);
        oracle.writes_before_all_reads(scoping, wf_sym, reads)
    };

    let mut accepted: HashMap<SymbolId, Expression<'a>> = HashMap::new();
    // Integer view of resolved symbols, the environment the pure-expr fixpoint
    // evaluates against (only integer-valued resolutions participate).
    let mut env: HashMap<SymbolId, i64> = HashMap::new();

    // Base case: literal writes (the original, narrow rule).
    for (&sym, lit) in &census.literal {
        if ordered_ok(sym) {
            accepted.insert(sym, lit.clone_in(allocator));
            if let Some(v) = expr_to_int(lit) {
                env.insert(sym, v);
            }
        }
    }

    // Fixpoint: a pure-integer-expression write resolves once every symbol it reads
    // is already resolved to an integer. Bounded by the number of pending symbols
    // (each pass resolves ≥1 or terminates), so it cannot loop. Soundness: the RHS
    // is total/deterministic (`is_pure_int_expr`), and `ordered_ok` proves the
    // write happens-before all reads of `sym`; the reads it performs of its operand
    // symbols are themselves covered by those operands' own happens-before proofs
    // (they only entered `env` via this same gate).
    let ast = oxc_ast::AstBuilder::new(allocator);
    let max_passes = census.pure_rhs.len() + 1;
    for _ in 0..max_passes {
        let mut changed = false;
        for (&sym, rhs) in &census.pure_rhs {
            if env.contains_key(&sym) {
                continue; // already resolved
            }
            if !ordered_ok(sym) {
                continue;
            }
            if let Some(v) = eval_pure_int(rhs, scoping, &env) {
                env.insert(sym, v);
                accepted.insert(sym, int_lit(v, ast));
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    accepted
}

/// Build a numeric-literal expression for an integer (negative values become a
/// `-` unary over a non-negative literal, the canonical AST shape).
fn int_lit<'a>(v: i64, ast: oxc_ast::AstBuilder<'a>) -> Expression<'a> {
    use oxc_syntax::number::NumberBase;
    if v < 0 {
        let inner =
            ast.expression_numeric_literal(Span::default(), (-v) as f64, None, NumberBase::Decimal);
        ast.expression_unary(
            Span::default(),
            oxc_syntax::operator::UnaryOperator::UnaryNegation,
            inner,
        )
    } else {
        ast.expression_numeric_literal(Span::default(), v as f64, None, NumberBase::Decimal)
    }
}
