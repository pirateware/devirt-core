//! Control-flow unflattening for the integer-state `while`/`switch` dispatcher.
//!
//! Obfuscators flatten straight-line code into a state machine:
//! ```text
//! var s = 1;
//! while (s) { switch (s) { case 1: A(); s = 2; break; case 2: B(); s = 0; break; } }
//! ```
//! When the state variable is a private dispatch counter — assigned only integer
//! literals inside the cases, read only as the loop test and switch discriminant —
//! the machine is a fixed, acyclic chain we can *execute at compile time*: start at
//! the initializer, follow each case's transition, and emit the case bodies in the
//! order they would run. The result is the original linear code.
//!
//! Loop shapes handled (the state var `s` is a local initialized to an integer
//! literal in the preceding statement):
//!   * `while (<test>) { switch (s) { … } }`
//!   * `do { switch (s) { … } } while (<test>)` — body runs before the test.
//! `<test>` may be `s` (falsy exit), `true` (exit only via a terminal case), or a
//! comparison `s <op> K` (`!=`, `==`, `<`, `<=`, `>`, `>=`) against an integer
//! literal on either side. The exit is modelled exactly by `LoopTest::continues`,
//! so a non-zero terminal state (`while (s != 105)`) is handled, not just `0`.
//! Transitions may be `s = K` (absolute) or `s += K` / `s -= K` (relative); the
//! simulator tracks the concrete state so a relative step is fully determined.
//!
//! Soundness is by construction + heavy gating. We transform only when every check
//! below holds, and otherwise leave the construct untouched:
//!   * `s` is referenced nowhere but the initializer, the loop test, the
//!     `switch (s)` discriminant, and top-level transitions inside cases (so
//!     emitting bodies can't observe or alter dispatch state);
//!   * the switch has no `default` clause;
//!   * each visited case ends in `break`/`continue` (with exactly one top-level
//!     transition) or a `return`/`throw` terminator — never falls through, and
//!     contains no other top-level `break`/`continue`/`return`/`throw`;
//!   * the simulated state chain terminates (the loop test goes false) and never
//!     revisits a state (a revisit is a real loop, not a linear chain → abort).
//!
//! Because `s` is dispatch-only and the chain is acyclic and deterministic, the
//! emitted sequence runs the exact same statements in the exact same order.
//!
//! NOT handled (param-state): dispatchers whose state is a *function parameter*
//! (`function f(s, …) { while (s != K) switch (s) … }`, called `f(CONST, …)`) —
//! the dominant shape in samples 1 & 4. That needs an interprocedural specialize-
//! per-entry-constant transform; see issues/01-unflatten-too-narrow.md.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, BlockStatement, Expression, Program, SimpleAssignmentTarget,
    Statement, SwitchStatement,
};
use oxc_syntax::operator::{AssignmentOperator, BinaryOperator};
use oxc_ast::ast::VariableDeclarationKind;
use oxc_ast_visit::Visit;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;

#[derive(Default)]
pub struct Unflatten {
    changed: bool,
}

impl Pass for Unflatten {
    fn name(&self) -> &'static str {
        "unflatten"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        // No scoping needed: the analysis is structural/name-based within one
        // statement list.
        let scoping = crate::util::build_scoping(program);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

impl<'a> Traverse<'a, ()> for Unflatten {
    fn enter_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        // Find a `<state init>; while (s) { switch (s) {…} }` adjacent pair.
        let mut hit: Option<(usize, Vec<Statement<'a>>)> = None;
        for i in 0..stmts.len().saturating_sub(1) {
            let Some((s_name, init)) = state_init(&stmts[i]) else {
                continue;
            };
            let Some((switch, kind, test)) = detect_loop(&stmts[i + 1], &s_name) else {
                continue;
            };
            // `s` must not be referenced anywhere in this list except the init and
            // the loop construct (indices i, i+1).
            if !dispatch_only_in_list(stmts, i, &s_name) {
                continue;
            }
            let Some(cases) = parse_cases(switch, &s_name) else {
                continue;
            };
            let Some(seq) = simulate(init, &cases, &test, kind, ctx.ast.allocator) else {
                continue;
            };
            hit = Some((i, seq));
            break;
        }

        let Some((i, seq)) = hit else {
            return;
        };
        // Splice: replace the init (i) and while (i+1) with the linear sequence.
        // `seq` already holds arena clones, so move them in directly.
        let old = std::mem::replace(stmts, ctx.ast.vec());
        let mut seq = Some(seq);
        for (j, s) in old.into_iter().enumerate() {
            if j == i {
                continue; // drop the state initializer
            }
            if j == i + 1 {
                for st in seq.take().unwrap() {
                    stmts.push(st);
                }
                continue; // drop the while; sequence takes its place
            }
            stmts.push(s);
        }
        self.changed = true;
    }
}

/// `var s = <int>;` (single declarator) or `s = <int>;` → `(name, value)`.
fn state_init(stmt: &Statement<'_>) -> Option<(String, i64)> {
    match stmt {
        Statement::VariableDeclaration(vd) if vd.declarations.len() == 1 => {
            let d = &vd.declarations[0];
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                return None;
            };
            let v = int_literal(d.init.as_ref()?)?;
            Some((id.name.to_string(), v))
        }
        Statement::ExpressionStatement(es) => {
            let Expression::AssignmentExpression(a) = &es.expression else {
                return None;
            };
            if !matches!(a.operator, oxc_syntax::operator::AssignmentOperator::Assign) {
                return None;
            }
            let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
                return None;
            };
            let v = int_literal(&a.right)?;
            Some((id.name.to_string(), v))
        }
        _ => None,
    }
}

/// A non-negative or negative integer-valued numeric literal (`5`, `-1`).
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

/// Whether the loop tests its condition *before* the body (`while`/`for`) or
/// *after* (`do…while`, body runs at least once).
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum LoopKind {
    While,
    DoWhile,
}

/// The loop's continue condition as a predicate over the integer state. The loop
/// runs the switch while this holds and exits when it goes false. Modelling the
/// test exactly (rather than hard-coding the `while (s)` falsy exit) is what lets
/// us handle `while (s != K)`, `while (s < K)`, etc.
pub(crate) enum LoopTest {
    /// `while (s)` — continue while `s` is truthy (non-zero).
    Truthy,
    /// `while (true)` / `for (;;)` — never exits via the test (only via a
    /// terminal `return`/`throw` case).
    Always,
    /// `while (s <op> K)`.
    Cmp(BinaryOperator, i64),
}

impl LoopTest {
    /// Does the loop continue (run another switch iteration) at this state?
    fn continues(&self, state: i64) -> bool {
        match self {
            LoopTest::Truthy => state != 0,
            LoopTest::Always => true,
            LoopTest::Cmp(op, k) => match op {
                BinaryOperator::Inequality | BinaryOperator::StrictInequality => state != *k,
                BinaryOperator::Equality | BinaryOperator::StrictEquality => state == *k,
                BinaryOperator::LessThan => state < *k,
                BinaryOperator::LessEqualThan => state <= *k,
                BinaryOperator::GreaterThan => state > *k,
                BinaryOperator::GreaterEqualThan => state >= *k,
                _ => false,
            },
        }
    }
}

/// If `stmt` is `while (<test>) { switch (<s>) { … } }` or the `do…while` form,
/// return the switch, the loop kind, and the parsed test. The switch discriminant
/// must be the bare state identifier; the test must be a predicate over `s` (and
/// integer constants) so the exit is statically determined. No `default` clause.
pub(crate) fn detect_loop<'b, 'a>(
    stmt: &'b Statement<'a>,
    s: &str,
) -> Option<(&'b SwitchStatement<'a>, LoopKind, LoopTest)> {
    let (test, body, kind) = match stmt {
        Statement::WhileStatement(w) => (parse_loop_test(&w.test, s)?, &w.body, LoopKind::While),
        Statement::DoWhileStatement(d) => {
            (parse_loop_test(&d.test, s)?, &d.body, LoopKind::DoWhile)
        }
        // `for (init; test; update) { switch }` behaves like `while` (test before
        // body). The init/update must not assign the state — for a param-state
        // dispatcher they are no-ops (`for (s; s != K; s)`); we do not model a
        // for-update transition. A missing test means `for (;;)` → `Always`.
        Statement::ForStatement(f) => {
            if f.init.as_ref().is_some_and(|i| for_init_writes(i, s)) {
                return None;
            }
            if f.update.as_ref().is_some_and(|u| writes_in_expr(u, s)) {
                return None;
            }
            let test = match &f.test {
                Some(t) => parse_loop_test(t, s)?,
                None => LoopTest::Always,
            };
            (test, &f.body, LoopKind::While)
        }
        _ => return None,
    };
    let switch = match body {
        Statement::SwitchStatement(sw) => sw,
        Statement::BlockStatement(b) if b.body.len() == 1 => {
            let Statement::SwitchStatement(sw) = &b.body[0] else {
                return None;
            };
            sw
        }
        _ => return None,
    };
    if !is_ident(&switch.discriminant, s) {
        return None;
    }
    // No `default` clause (a fallback path we don't model).
    if switch.cases.iter().any(|c| c.test.is_none()) {
        return None;
    }
    Some((switch, kind, test))
}

/// Parse a loop test into a `LoopTest` predicate over state `s`. Accepts the bare
/// `s` (truthy), `true`, and a binary comparison of `s` against an integer literal
/// on either side. Anything else → `None` (we won't model it).
fn parse_loop_test(expr: &Expression<'_>, s: &str) -> Option<LoopTest> {
    if is_ident(expr, s) {
        return Some(LoopTest::Truthy);
    }
    if matches!(expr, Expression::BooleanLiteral(b) if b.value) {
        return Some(LoopTest::Always);
    }
    let Expression::BinaryExpression(b) = expr else {
        return None;
    };
    if !is_comparison(b.operator) {
        return None;
    }
    // `s <op> K` or `K <op> s` — normalise so state is on the left.
    if is_ident(&b.left, s) {
        let k = int_literal(&b.right)?;
        Some(LoopTest::Cmp(b.operator, k))
    } else if is_ident(&b.right, s) {
        let k = int_literal(&b.left)?;
        Some(LoopTest::Cmp(flip_comparison(b.operator), k))
    } else {
        None
    }
}

fn is_comparison(op: BinaryOperator) -> bool {
    matches!(
        op,
        BinaryOperator::Inequality
            | BinaryOperator::StrictInequality
            | BinaryOperator::Equality
            | BinaryOperator::StrictEquality
            | BinaryOperator::LessThan
            | BinaryOperator::LessEqualThan
            | BinaryOperator::GreaterThan
            | BinaryOperator::GreaterEqualThan
    )
}

/// Mirror a comparison so `K <op> s` becomes `s <flip(op)> K`.
fn flip_comparison(op: BinaryOperator) -> BinaryOperator {
    match op {
        BinaryOperator::LessThan => BinaryOperator::GreaterThan,
        BinaryOperator::LessEqualThan => BinaryOperator::GreaterEqualThan,
        BinaryOperator::GreaterThan => BinaryOperator::LessThan,
        BinaryOperator::GreaterEqualThan => BinaryOperator::LessEqualThan,
        other => other, // ==, ===, !=, !== are symmetric
    }
}

fn is_ident(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name == name)
}

/// How a case mutates the dispatch state in its trailing transition.
#[derive(Clone, Copy)]
enum Trans {
    /// `s = <int>` — jump to an absolute state.
    Abs(i64),
    /// `s += <int>` / `s -= <int>` — relative to the current state.
    Rel(i64),
}

impl Trans {
    fn apply(self, state: i64) -> i64 {
        match self {
            Trans::Abs(v) => v,
            Trans::Rel(d) => state + d,
        }
    }
}

/// What a case does once its body has run.
enum CaseEnd {
    /// Transition then re-enter the loop (trailing `break`/`continue`).
    Step(Trans),
    /// `return`/`throw`: the machine stops here.
    Terminal,
}

pub(crate) struct ParsedCase<'b, 'a> {
    emit: Vec<&'b Statement<'a>>,
    end: CaseEnd,
}

/// Parse every case into (emitted body, end behavior), keyed by its integer state.
/// Returns `None` (bail) on any shape we don't model soundly.
pub(crate) fn parse_cases<'b, 'a>(
    switch: &'b SwitchStatement<'a>,
    s: &str,
) -> Option<HashMap<i64, ParsedCase<'b, 'a>>> {
    let mut out = HashMap::new();
    for case in &switch.cases {
        let state = int_literal(case.test.as_ref()?)?;
        // Obfuscators wrap each case body in a block (`case X: { … } break;`). Inline
        // transparent blocks (no block-scoped declarations) so the transition and
        // terminator become visible at the case's top level.
        let mut flat: Vec<&Statement> = Vec::new();
        flatten_case(&case.consequent, &mut flat);
        // The first top-level terminator decides the case end; anything after it is
        // dead (e.g. a `break` after a `return`).
        let term_idx = flat.iter().position(|st| is_top_level_control(st))?;
        let (end, upto, terminal_incl) = match flat[term_idx] {
            Statement::BreakStatement(b) if b.label.is_none() => {
                let next = sole_transition(&flat[..term_idx], s)?;
                (CaseEnd::Step(next), term_idx, false)
            }
            Statement::ContinueStatement(c) if c.label.is_none() => {
                let next = sole_transition(&flat[..term_idx], s)?;
                (CaseEnd::Step(next), term_idx, false)
            }
            Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => {
                // Emit the terminator too (it exits the machine / function).
                (CaseEnd::Terminal, term_idx + 1, true)
            }
            _ => return None, // labeled break/continue → unmodeled
        };

        // Statements to emit: everything up to the terminator, minus the state
        // transition(s). Validate the remainder is dispatch-clean (no `s`, no stray
        // top-level control). For a `return`/`throw` end the terminator is the last
        // emitted statement and is exempt from the control check.
        let mut emit = Vec::new();
        for (k, &st) in flat[..upto].iter().enumerate() {
            let is_terminal_stmt = terminal_incl && k == upto - 1;
            if is_state_transition(st, s).is_some() {
                continue; // drop the transition assignment
            }
            if mentions(st, s) {
                return None; // body observes/alters dispatch state
            }
            if !is_terminal_stmt && is_top_level_control(st) {
                return None; // a second dispatch exit we don't model
            }
            emit.push(st);
        }
        if out.insert(state, ParsedCase { emit, end }).is_some() {
            return None; // duplicate case label
        }
    }
    Some(out)
}

/// Flatten a case consequent, inlining *transparent* blocks (those with no
/// block-scoped declarations — `let`/`const`/`class`/`function`). A `var` block is
/// transparent (var is function-scoped). An opaque block is pushed as one
/// statement (so a transition hidden inside it trips the `mentions` guard).
fn flatten_case<'b, 'a>(stmts: &'b [Statement<'a>], out: &mut Vec<&'b Statement<'a>>) {
    for st in stmts {
        if let Statement::BlockStatement(b) = st {
            if block_is_transparent(b) {
                flatten_case(&b.body, out);
                continue;
            }
        }
        out.push(st);
    }
}

/// A block is transparent (safe to inline into its parent) when it declares no
/// block-scoped bindings: only `var`, expressions, and control flow.
fn block_is_transparent(b: &BlockStatement<'_>) -> bool {
    !b.body.iter().any(|st| match st {
        Statement::ClassDeclaration(_) | Statement::FunctionDeclaration(_) => true,
        Statement::VariableDeclaration(vd) => vd.kind != VariableDeclarationKind::Var,
        _ => false,
    })
}

/// Among (flattened) top-level statements, find the state transitions. Require
/// exactly one (the next state); none or a non-top-level transition → `None`.
fn sole_transition(stmts: &[&Statement<'_>], s: &str) -> Option<Trans> {
    let mut found: Option<Trans> = None;
    for &st in stmts {
        if let Some(t) = is_state_transition(st, s) {
            if found.is_some() {
                return None; // more than one unconditional transition
            }
            found = Some(t);
        } else if writes_name(st, s) {
            return None; // a transition nested in control flow → branching dispatch
        }
    }
    found
}

/// `s = <int>;` / `s += <int>;` / `s -= <int>;` as a top-level statement → the
/// transition it encodes. A compound assignment is sound because the simulator
/// tracks the concrete current state, so `s += K` is fully determined.
fn is_state_transition(stmt: &Statement<'_>, s: &str) -> Option<Trans> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::AssignmentExpression(a) = &es.expression else {
        return None;
    };
    let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
        return None;
    };
    if id.name != s {
        return None;
    }
    let v = int_literal(&a.right)?;
    match a.operator {
        AssignmentOperator::Assign => Some(Trans::Abs(v)),
        AssignmentOperator::Addition => Some(Trans::Rel(v)),
        AssignmentOperator::Subtraction => Some(Trans::Rel(-v)),
        _ => None,
    }
}

fn is_top_level_control(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::BreakStatement(_)
            | Statement::ContinueStatement(_)
            | Statement::ReturnStatement(_)
            | Statement::ThrowStatement(_)
    )
}

/// Execute the dispatch chain from `init`, emitting case bodies in run order.
///
/// The loop kind decides when the exit test is checked: `while`/`for` test
/// *before* each switch, `do…while` *after* (so the body runs at least once).
/// `test.continues(state)` models the real loop condition exactly, so the emitted
/// sequence runs the same statements in the same order as the dispatcher.
pub(crate) fn simulate<'a>(
    init: i64,
    cases: &HashMap<i64, ParsedCase<'_, 'a>>,
    test: &LoopTest,
    kind: LoopKind,
    alloc: &'a Allocator,
) -> Option<Vec<Statement<'a>>> {
    let mut state = init;
    let mut visited: HashSet<i64> = HashSet::new();
    let mut seq: Vec<Statement<'a>> = Vec::new();
    // Bound iterations defensively (cycle detection already covers it).
    for _ in 0..100_000 {
        // `while`/`for`: the test gates entry into the body.
        if kind == LoopKind::While && !test.continues(state) {
            return Some(seq);
        }
        if !visited.insert(state) {
            return None; // revisited state → real loop, not a linear chain
        }
        let case = cases.get(&state)?; // no matching case + no default → unmodeled
        for st in &case.emit {
            // Deep-clone each emitted statement into the arena so it survives the
            // splice that replaces the original dispatcher.
            seq.push((*st).clone_in(alloc));
        }
        match case.end {
            CaseEnd::Step(t) => state = t.apply(state),
            CaseEnd::Terminal => return Some(seq),
        }
        // `do…while`: the body has run; the test now decides whether to continue.
        if kind == LoopKind::DoWhile && !test.continues(state) {
            return Some(seq);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Name-reference helpers (the pass is name-based, like decoder-lift)
// ---------------------------------------------------------------------------

/// True unless `s` is referenced only within `stmts[i]` (init) and `stmts[i+1]`
/// (the while). Any other reference means `s` is not a private dispatch var.
fn dispatch_only_in_list(stmts: &[Statement<'_>], i: usize, s: &str) -> bool {
    for (j, st) in stmts.iter().enumerate() {
        if j == i || j == i + 1 {
            continue;
        }
        if mentions(st, s) {
            return false;
        }
    }
    true
}

struct NameScan<'r> {
    name: &'r str,
    reads_or_writes: bool,
    writes: bool,
}

impl<'a, 'r> Visit<'a> for NameScan<'r> {
    fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
        if it.name == self.name {
            self.reads_or_writes = true;
        }
    }
    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        if it.name == self.name {
            self.reads_or_writes = true;
            self.writes = true;
        }
    }
}

/// Does `stmt` reference `name` at all (read or write)?
fn mentions(stmt: &Statement<'_>, name: &str) -> bool {
    let mut s = NameScan {
        name,
        reads_or_writes: false,
        writes: false,
    };
    s.visit_statement(stmt);
    s.reads_or_writes
}

/// A visitor that flags any assignment/update whose target identifier is `name`.
struct WriteScan<'r> {
    name: &'r str,
    found: bool,
}
impl<'a, 'r> Visit<'a> for WriteScan<'r> {
    fn visit_assignment_target(&mut self, t: &AssignmentTarget<'a>) {
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = t {
            if id.name == self.name {
                self.found = true;
            }
        }
        oxc_ast_visit::walk::walk_assignment_target(self, t);
    }
    fn visit_simple_assignment_target(&mut self, t: &SimpleAssignmentTarget<'a>) {
        if let SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = t {
            if id.name == self.name {
                self.found = true;
            }
        }
        oxc_ast_visit::walk::walk_simple_assignment_target(self, t);
    }
}

/// Does `stmt` write `name` (assignment/update/binding) anywhere within it?
fn writes_name(stmt: &Statement<'_>, name: &str) -> bool {
    let mut w = WriteScan { name, found: false };
    w.visit_statement(stmt);
    w.found
}

/// Does the `for`-update expression assign `name`?
fn writes_in_expr(expr: &Expression<'_>, name: &str) -> bool {
    let mut w = WriteScan { name, found: false };
    w.visit_expression(expr);
    w.found
}

/// Does the `for`-init clause (a declaration of `name`, or an assignment to it)
/// establish/overwrite the state? A bare-ident no-op (`for (s; …)`) does not.
fn for_init_writes(init: &oxc_ast::ast::ForStatementInit<'_>, name: &str) -> bool {
    use oxc_ast::ast::ForStatementInit as F;
    match init {
        F::VariableDeclaration(vd) => vd.declarations.iter().any(|d| {
            matches!(&d.id, BindingPattern::BindingIdentifier(id) if id.name == name)
        }),
        other => {
            // An expression init (`for (s = 1; …)` / `for (s; …)`).
            let mut w = WriteScan { name, found: false };
            if let Some(e) = other.as_expression() {
                w.visit_expression(e);
            }
            w.found
        }
    }
}
