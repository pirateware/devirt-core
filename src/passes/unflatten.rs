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
//!
//! `<test>` may be `s` (falsy exit), `true` (exit only via a terminal case), or a
//! comparison `s <op> K` (`!=`, `==`, `<`, `<=`, `>`, `>=`) against an integer
//! literal on either side. The exit is modelled exactly by `LoopTest::continues`,
//! so a non-zero terminal state (`while (s != 105)`) is handled, not just `0`.
//! Transitions may be `s = K` (absolute), `s += K` / `s -= K` (additive),
//! `s *= K` (multiplicative), or `s = next[s]` / `s = next[K]` (a lookup into a
//! *provably-immutable* const integer table `next`, gated by
//! `mutable_aggregate_tables`); the simulator tracks the concrete state so any such
//! step is fully determined. A cycle (a revisited state, e.g. `s *= 1`) aborts.
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
use std::rc::Rc;

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::VariableDeclarationKind;
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, BlockStatement, Expression, Program, SimpleAssignmentTarget,
    Statement, SwitchStatement,
};
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_syntax::operator::{AssignmentOperator, BinaryOperator};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;

#[derive(Default)]
pub struct Unflatten {
    changed: bool,
    /// Resolved, provably-immutable const integer tables (name → values), used to
    /// model `s = next[…]` lookup transitions. Computed once per `run` from the
    /// whole program under the `mutable_aggregate_tables` gate.
    tables: LookupTables,
}

impl Pass for Unflatten {
    fn name(&self) -> &'static str {
        "unflatten"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        // Scoping is needed by the immutability gate (it resolves member-writes /
        // aliasing to symbols). The unflatten analysis itself is name-based.
        let scoping = crate::util::build_scoping(program);
        self.tables = resolve_const_int_tables(program, &scoping);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

/// Resolve every top-level (and nested-function) binding whose single value is a
/// const integer-array literal AND which is provably immutable per
/// `mutable_aggregate_tables` (never reassigned, member-written, `.push`-ed, or
/// read bare into a mutable alias). The resulting `name → values` map is the only
/// thing the lookup-transition form trusts; a table absent from it is declined.
///
/// Soundness: a name present here is write-once to a literal int array and never
/// mutated anywhere in the program, so indexing it at compile time yields exactly
/// the value the runtime would read. Object-map tables are NOT resolved (array
/// form only, per the milestone); a non-integer element makes the binding absent.
pub(crate) fn resolve_const_int_tables(program: &Program<'_>, scoping: &Scoping) -> LookupTables {
    let mutable = crate::ir::mutable_aggregate_tables(program, scoping);
    let mut collector = TableCollector {
        out: HashMap::new(),
        seen_twice: HashSet::new(),
    };
    collector.visit_program(program);
    collector
        .out
        .into_iter()
        .filter(|(name, _)| !mutable.contains(name) && !collector.seen_twice.contains(name))
        .map(|(name, values)| (name, Rc::new(values)))
        .collect()
}

/// Collects bindings initialized to a const int-array literal. A name initialized
/// more than once (across any scope) is recorded in `seen_twice` and dropped:
/// even though `mutable_aggregate_tables` is symbol-precise, this collector is
/// name-keyed (matching the unflattener's name-based lookups), so two distinct
/// same-named const arrays in different scopes must not be conflated.
struct TableCollector {
    out: HashMap<String, Vec<i64>>,
    seen_twice: HashSet<String>,
}

impl<'a> oxc_ast_visit::Visit<'a> for TableCollector {
    fn visit_variable_declarator(&mut self, decl: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &decl.id {
            if let Some(init) = &decl.init {
                if let Some(values) = const_int_array(init) {
                    let name = id.name.to_string();
                    if self.out.insert(name.clone(), values).is_some() {
                        self.seen_twice.insert(name);
                    }
                }
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, decl);
    }
}

/// An array literal all of whose elements are integer literals → their values.
/// Holes, spreads, or any non-integer element → `None` (not a usable lookup table).
fn const_int_array(expr: &Expression<'_>) -> Option<Vec<i64>> {
    let Expression::ArrayExpression(arr) = expr else {
        return None;
    };
    let mut out = Vec::with_capacity(arr.elements.len());
    for el in &arr.elements {
        let oxc_ast::ast::ArrayExpressionElement::SpreadElement(_) = el else {
            // A non-spread element: it must be an integer-literal expression (a hole
            // — `Elision` — is `None` here and rejects the whole table).
            let e = el.as_expression()?;
            out.push(int_literal(e)?);
            continue;
        };
        return None; // spread → unmodellable
    }
    Some(out)
}

impl<'a> Traverse<'a, ()> for Unflatten {
    fn enter_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        // Find a `<state init> … while (s) { switch (s) {…} }` pair. The init and
        // the dispatcher loop need NOT be adjacent: the state var may have a
        // declaration or two between them (`var s = 1; var acc = 0; while(s){…}`),
        // as long as `s` is referenced *only* in the init and the loop. That
        // "dispatch-only" invariant — the same one the adjacent form relies on —
        // is what keeps the relaxation sound: statements between the init and the
        // loop cannot observe or alter `s`, so the simulated state still holds when
        // the loop is entered, and they stay in place before the linear sequence.
        let mut hit: Option<(usize, usize, Vec<Statement<'a>>)> = None;
        for i in 0..stmts.len() {
            // The state initializer may be an integer (the primary engine) or a
            // string/boolean literal (`sval`). Both share the dispatch-only gate.
            let s_name = match init_state_name(&stmts[i]) {
                Some(n) => n,
                None => continue,
            };
            // `s` must appear in exactly one OTHER statement — the dispatcher.
            let others: Vec<usize> = (0..stmts.len())
                .filter(|&k| k != i && mentions(&stmts[k], &s_name))
                .collect();
            if others.len() != 1 {
                continue;
            }
            let j = others[0];
            if j <= i {
                continue; // the loop must run after the initializer
            }
            // Try the integer engine first; fall back to the string/boolean engine.
            let seq = simulate_int_at(
                &stmts[i],
                &stmts[j],
                &s_name,
                &self.tables,
                ctx.ast.allocator,
            )
            .or_else(|| simulate_sval_at(&stmts[i], &stmts[j], &s_name, ctx.ast.allocator));
            let Some(seq) = seq else {
                continue;
            };
            hit = Some((i, j, seq));
            break;
        }

        let Some((i, j, seq)) = hit else {
            return;
        };
        // Splice: drop the state initializer (i) and replace the dispatcher loop
        // (j) with the linear sequence. Statements between i and j (and elsewhere)
        // are preserved in order. `seq` already holds arena clones.
        let old = std::mem::replace(stmts, ctx.ast.vec());
        let mut seq = Some(seq);
        for (k, s) in old.into_iter().enumerate() {
            if k == i {
                continue; // drop the state initializer
            }
            if k == j {
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

/// The dispatch-state variable name if `stmt` initializes one to an integer,
/// string, or boolean literal. Used to find dispatch candidates before deciding
/// which engine (integer vs. string/boolean) applies.
fn init_state_name(stmt: &Statement<'_>) -> Option<String> {
    if let Some((name, _)) = state_init(stmt) {
        return Some(name);
    }
    sval_init(stmt).map(|(name, _)| name)
}

/// Run the integer engine on the (init, dispatcher) statement pair, returning the
/// linearized sequence. `None` if the integer engine declines.
fn simulate_int_at<'a>(
    init_stmt: &Statement<'a>,
    loop_stmt: &Statement<'a>,
    s_name: &str,
    tables: &LookupTables,
    alloc: &'a Allocator,
) -> Option<Vec<Statement<'a>>> {
    let (_, init) = state_init(init_stmt)?;
    let (switch, kind, test) = detect_loop(loop_stmt, s_name)?;
    let cases = parse_cases(switch, s_name, tables)?;
    // The local form replaces only the dispatcher; statements after it stay in the
    // list, so the exit mode is irrelevant here.
    let (seq, _exit) = simulate(init, &cases, &test, kind, alloc)?;
    Some(seq)
}

/// `var s = <int>;` (single declarator) or `s = <int>;` → `(name, value)`.
pub(crate) fn state_init(stmt: &Statement<'_>) -> Option<(String, i64)> {
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
    pub(crate) fn continues(&self, state: i64) -> bool {
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

/// A resolved, provably-immutable const integer table keyed by name, used to model
/// `s = next[…]` lookup transitions. Threaded into `parse_cases` by the caller,
/// which resolves the tables and applies the immutability gate
/// (`mutable_aggregate_tables`). An empty map disables the lookup form entirely
/// (the caller has no const tables) — a sound decline.
pub(crate) type LookupTables = HashMap<String, Rc<Vec<i64>>>;

/// How a lookup transition (`s = next[idx]`) computes its index into the table.
#[derive(Clone, Copy)]
enum LookupIndex {
    /// `s = next[s]` — index by the current dispatch state.
    State,
    /// `s = next[<int>]` — index by a fixed constant.
    Const(i64),
}

/// How a case mutates the dispatch state in its trailing transition.
#[derive(Clone)]
enum Trans {
    /// `s = <int>` — jump to an absolute state.
    Abs(i64),
    /// `s += <int>` / `s -= <int>` — relative to the current state.
    Rel(i64),
    /// `s *= <int>` — multiplicative relative to the current state.
    Mul(i64),
    /// `s = next[s]` / `s = next[<int>]` — the next state is read from a
    /// provably-immutable const integer table. The table values are resolved and
    /// baked in at parse time; the index is the current state or a constant. This
    /// subsumes `Abs` as the degenerate constant-index case, but we keep `Abs`
    /// distinct so the common literal-transition path needs no table.
    Lookup {
        table: Rc<Vec<i64>>,
        index: LookupIndex,
    },
}

impl Trans {
    /// Compute the next state from the current one. Returns `None` only for a
    /// lookup whose index is out of the table's bounds — a JS `undefined` read we
    /// will not model, so the simulator declines the whole transform.
    fn apply(&self, state: i64) -> Option<i64> {
        match self {
            Trans::Abs(v) => Some(*v),
            Trans::Rel(d) => Some(state + d),
            Trans::Mul(f) => Some(state * f),
            Trans::Lookup { table, index } => {
                let i = match index {
                    LookupIndex::State => state,
                    LookupIndex::Const(k) => *k,
                };
                // Out of range (incl. negative) → `undefined` in JS → decline.
                let idx = usize::try_from(i).ok()?;
                table.get(idx).copied()
            }
        }
    }
}

/// What a case does once its body has run.
enum CaseEnd {
    /// Transition then re-enter the loop (trailing `break`/`continue`).
    Step(Trans),
    /// `return`/`throw`: the machine stops here.
    Terminal,
    /// No `break`/terminator: control falls through into the *textually next*
    /// case's body. The payload is that successor case's (integer) state label, so
    /// the simulator continues there. A case that falls off the end of the switch
    /// (no successor) is not modelled — `parse_cases` declines.
    FallThrough(i64),
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
    tables: &LookupTables,
) -> Option<HashMap<i64, ParsedCase<'b, 'a>>> {
    let mut out = HashMap::new();
    // Each case's integer label, in textual order, so a fall-through case (no
    // `break`) can name its successor's state. Computed up front; a non-integer
    // label anywhere makes the whole switch unmodellable.
    let mut labels: Vec<i64> = Vec::with_capacity(switch.cases.len());
    for case in &switch.cases {
        labels.push(int_literal(case.test.as_ref()?)?);
    }
    for (ci, case) in switch.cases.iter().enumerate() {
        let state = labels[ci];
        // Obfuscators wrap each case body in a block (`case X: { … } break;`). Inline
        // transparent blocks (no block-scoped declarations) so the transition and
        // terminator become visible at the case's top level.
        let mut flat: Vec<&Statement> = Vec::new();
        flatten_case(&case.consequent, &mut flat);
        // The first top-level terminator decides the case end; anything after it is
        // dead (e.g. a `break` after a `return`). A case with *no* top-level control
        // statement falls through into the textually-next case's body.
        let Some(term_idx) = flat.iter().position(|st| is_top_level_control(st)) else {
            // Fall-through: emit this case's body, then continue at the next case.
            let Some(&next_state) = labels.get(ci + 1) else {
                return None; // falls off the end of the switch — unmodelled
            };
            // The body must be dispatch-clean (no mention of `s`): there is no
            // transition to drop here, so any reference/write to `s` is unmodelled.
            let mut emit = Vec::new();
            for &st in &flat {
                if mentions(st, s) {
                    return None;
                }
                emit.push(st);
            }
            if out
                .insert(
                    state,
                    ParsedCase {
                        emit,
                        end: CaseEnd::FallThrough(next_state),
                    },
                )
                .is_some()
            {
                return None; // duplicate case label
            }
            continue;
        };
        let (end, upto, terminal_incl) = match flat[term_idx] {
            Statement::BreakStatement(b) if b.label.is_none() => {
                let next = sole_transition(&flat[..term_idx], s, tables)?;
                (CaseEnd::Step(next), term_idx, false)
            }
            Statement::ContinueStatement(c) if c.label.is_none() => {
                let next = sole_transition(&flat[..term_idx], s, tables)?;
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
            if is_state_transition(st, s, tables).is_some() {
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
pub(crate) fn flatten_case<'b, 'a>(stmts: &'b [Statement<'a>], out: &mut Vec<&'b Statement<'a>>) {
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
fn sole_transition(stmts: &[&Statement<'_>], s: &str, tables: &LookupTables) -> Option<Trans> {
    let mut found: Option<Trans> = None;
    for &st in stmts {
        if let Some(t) = is_state_transition(st, s, tables) {
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

/// `s = <int>;` / `s += <int>;` / `s -= <int>;` / `s = next[s|<int>];` as a
/// top-level statement → the transition it encodes. A compound assignment is sound
/// because the simulator tracks the concrete current state, so `s += K` is fully
/// determined; a lookup is sound because `next` is a resolved const-immutable table
/// (gated by the caller via `mutable_aggregate_tables`) and the simulator computes
/// the concrete index.
fn is_state_transition(stmt: &Statement<'_>, s: &str, tables: &LookupTables) -> Option<Trans> {
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
    // `s = next[idx]` lookup form (absolute assignment only).
    if matches!(a.operator, AssignmentOperator::Assign) {
        if let Some(t) = parse_lookup_rhs(&a.right, s, tables) {
            return Some(t);
        }
    }
    let v = int_literal(&a.right)?;
    match a.operator {
        AssignmentOperator::Assign => Some(Trans::Abs(v)),
        AssignmentOperator::Addition => Some(Trans::Rel(v)),
        AssignmentOperator::Subtraction => Some(Trans::Rel(-v)),
        AssignmentOperator::Multiplication => Some(Trans::Mul(v)),
        _ => None,
    }
}

/// The RHS of an assignment `s = <rhs>` as a lookup transition `next[s]` /
/// `next[<int>]`, where `next` is a *resolved const-immutable* table (present in
/// `tables`). The index is the bare state identifier or an integer constant.
/// Anything else (non-member, computed-key expression, unknown/mutable table,
/// non-state non-const index) → `None` (decline).
fn parse_lookup_rhs(rhs: &Expression<'_>, s: &str, tables: &LookupTables) -> Option<Trans> {
    let Expression::ComputedMemberExpression(m) = rhs else {
        return None;
    };
    // The object must be a bare identifier naming a resolved const table.
    let Expression::Identifier(obj) = &m.object else {
        return None;
    };
    let table = tables.get(obj.name.as_str())?.clone();
    // The index is `s` (the current state) or an integer literal.
    let index = if is_ident(&m.expression, s) {
        LookupIndex::State
    } else {
        LookupIndex::Const(int_literal(&m.expression)?)
    };
    Some(Trans::Lookup { table, index })
}

pub(crate) fn is_top_level_control(stmt: &Statement<'_>) -> bool {
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
/// How the simulated machine left the loop.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum ExitMode {
    /// The loop test went false — control falls through past the loop.
    Test,
    /// A `return`/`throw` case fired — control leaves the enclosing function.
    Terminal,
}

pub(crate) fn simulate<'a>(
    init: i64,
    cases: &HashMap<i64, ParsedCase<'_, 'a>>,
    test: &LoopTest,
    kind: LoopKind,
    alloc: &'a Allocator,
) -> Option<(Vec<Statement<'a>>, ExitMode)> {
    let mut state = init;
    let mut visited: HashSet<i64> = HashSet::new();
    let mut seq: Vec<Statement<'a>> = Vec::new();
    // Bound iterations defensively (cycle detection already covers it).
    for _ in 0..100_000 {
        // `while`/`for`: the test gates entry into the body.
        if kind == LoopKind::While && !test.continues(state) {
            return Some((seq, ExitMode::Test));
        }
        if !visited.insert(state) {
            return None; // revisited state → real loop, not a linear chain
        }
        // Run the case at `state`, following any `break`-less fall-through into the
        // textually-next case *within the same switch iteration* (the loop test is
        // NOT re-checked between fall-through cases — control never returns to the
        // dispatcher until a `break`/terminator). A fall-through chain is bounded by
        // the distinct case labels; a step/terminal/cycle ends it.
        let mut ft_seen: HashSet<i64> = HashSet::new();
        loop {
            if !ft_seen.insert(state) {
                return None; // fall-through revisiting a label → not a linear chain
            }
            let case = cases.get(&state)?; // no matching case + no default → unmodeled
            for st in &case.emit {
                // Deep-clone each emitted statement into the arena so it survives the
                // splice that replaces the original dispatcher.
                seq.push((*st).clone_in(alloc));
            }
            match &case.end {
                CaseEnd::Step(t) => {
                    state = t.apply(state)?;
                    break;
                }
                CaseEnd::Terminal => return Some((seq, ExitMode::Terminal)),
                CaseEnd::FallThrough(next) => {
                    // Continue into the successor case body; mark it visited at the
                    // outer level too so a later `break` back to it is still a cycle.
                    if !visited.insert(*next) {
                        return None;
                    }
                    state = *next;
                }
            }
        }
        // `do…while`: the body has run; the test now decides whether to continue.
        if kind == LoopKind::DoWhile && !test.continues(state) {
            return Some((seq, ExitMode::Test));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Name-reference helpers (the pass is name-based, like decoder-lift)
// ---------------------------------------------------------------------------

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
pub(crate) fn mentions(stmt: &Statement<'_>, name: &str) -> bool {
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
        F::VariableDeclaration(vd) => vd
            .declarations
            .iter()
            .any(|d| matches!(&d.id, BindingPattern::BindingIdentifier(id) if id.name == name)),
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

// ---------------------------------------------------------------------------
// String / boolean state engine (`sval`)
// ---------------------------------------------------------------------------
//
// The integer engine above models a dispatch state that is an integer. Some
// obfuscators use a *string* (or *boolean*) state variable instead: the cases are
// `case "label":` and transitions are `s = "next"`. The recovery is identical —
// the chain is still acyclic, deterministic, and dispatch-only — so we mirror the
// integer engine over a hashable `StateVal`. Transitions are absolute only
// (`+=`/`*=` are meaningless on string labels here); the loop test compares the
// state against a literal with `==`/`!=`/`===`/`!==` or is the bare `s` (truthy).
//
// Soundness gates are exactly the integer engine's: `s` referenced only in the
// init, loop test, switch discriminant, and top-level transitions; no `default`;
// each visited case ends in `break`/`continue` (one transition) or `return`/
// `throw`, or falls through to the textual successor; the chain terminates and
// never revisits a state. A *semantic* switch on external data (e.g. a function
// parameter) never matches: there is no dispatch-only local string initializer.

/// A non-integer dispatch state value: a string or boolean. (Integers go through
/// the dedicated `i64` engine, which also supports compound transitions.)
#[derive(Clone, PartialEq, Eq, Hash)]
enum StateVal {
    Str(String),
    Bool(bool),
}

impl StateVal {
    /// Truthiness for the `while (s)` falsy-exit test. `""` and `false` are falsy.
    fn is_truthy(&self) -> bool {
        match self {
            StateVal::Str(s) => !s.is_empty(),
            StateVal::Bool(b) => *b,
        }
    }
}

/// A string/boolean literal expression → `StateVal`.
fn sval_literal(expr: &Expression<'_>) -> Option<StateVal> {
    match expr {
        Expression::StringLiteral(s) => Some(StateVal::Str(s.value.to_string())),
        Expression::BooleanLiteral(b) => Some(StateVal::Bool(b.value)),
        _ => None,
    }
}

/// `var s = <str|bool>;` (single declarator) or `s = <str|bool>;` → `(name, val)`.
fn sval_init(stmt: &Statement<'_>) -> Option<(String, StateVal)> {
    match stmt {
        Statement::VariableDeclaration(vd) if vd.declarations.len() == 1 => {
            let d = &vd.declarations[0];
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                return None;
            };
            let v = sval_literal(d.init.as_ref()?)?;
            Some((id.name.to_string(), v))
        }
        Statement::ExpressionStatement(es) => {
            let Expression::AssignmentExpression(a) = &es.expression else {
                return None;
            };
            if !matches!(a.operator, AssignmentOperator::Assign) {
                return None;
            }
            let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
                return None;
            };
            let v = sval_literal(&a.right)?;
            Some((id.name.to_string(), v))
        }
        _ => None,
    }
}

/// The loop continue-condition over a string/boolean state.
enum SvalTest {
    /// `while (s)` — continue while truthy.
    Truthy,
    /// `while (true)` — exit only via a terminal case.
    Always,
    /// `while (s ==/!= LIT)`. `strict` records `===`/`!==` (vs `==`/`!=`); `eq`
    /// records `==`/`===` (vs `!=`/`!==`). The literal is a string/boolean.
    Cmp {
        eq: bool,
        strict: bool,
        lit: StateVal,
    },
}

impl SvalTest {
    /// Does the loop continue at `state`? `None` ⇒ the comparison's JS coercion is
    /// not soundly modellable (a *loose* compare across different types), so the
    /// caller must decline the whole transform rather than guess.
    fn continues(&self, state: &StateVal) -> Option<bool> {
        match self {
            SvalTest::Truthy => Some(state.is_truthy()),
            SvalTest::Always => Some(true),
            SvalTest::Cmp { eq, strict, lit } => {
                let same_variant = matches!(
                    (state, lit),
                    (StateVal::Str(_), StateVal::Str(_)) | (StateVal::Bool(_), StateVal::Bool(_))
                );
                // Strict `===`/`!==`: different variants are never equal (no
                // coercion). Same-variant strict/loose reduce to value equality
                // (string↔string and bool↔bool have no `==` coercion gap).
                if *strict || same_variant {
                    let equal = state == lit;
                    Some(if *eq { equal } else { !equal })
                } else {
                    // Loose `==`/`!=` across types (e.g. `"" == false` is *true* in
                    // JS) — don't model it; decline.
                    None
                }
            }
        }
    }
}

/// Parse a loop test over a string/boolean state. Accepts bare `s` (truthy),
/// `true`, and `s ==/!=/===/!== LIT` (literal on either side). `None` otherwise.
fn parse_sval_test(expr: &Expression<'_>, s: &str) -> Option<SvalTest> {
    if is_ident(expr, s) {
        return Some(SvalTest::Truthy);
    }
    if matches!(expr, Expression::BooleanLiteral(b) if b.value) {
        return Some(SvalTest::Always);
    }
    let Expression::BinaryExpression(b) = expr else {
        return None;
    };
    let (eq, strict) = match b.operator {
        BinaryOperator::Equality => (true, false),
        BinaryOperator::StrictEquality => (true, true),
        BinaryOperator::Inequality => (false, false),
        BinaryOperator::StrictInequality => (false, true),
        _ => return None,
    };
    let lit = if is_ident(&b.left, s) {
        sval_literal(&b.right)?
    } else if is_ident(&b.right, s) {
        sval_literal(&b.left)?
    } else {
        return None;
    };
    Some(SvalTest::Cmp { eq, strict, lit })
}

/// `s = <str|bool literal>;` as a top-level statement → the next state. (Only
/// absolute transitions are meaningful for string/boolean labels.)
fn sval_transition(stmt: &Statement<'_>, s: &str) -> Option<StateVal> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::AssignmentExpression(a) = &es.expression else {
        return None;
    };
    if !matches!(a.operator, AssignmentOperator::Assign) {
        return None;
    }
    let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
        return None;
    };
    if id.name != s {
        return None;
    }
    sval_literal(&a.right)
}

/// The sole top-level transition among `stmts` (exactly one; a transition nested
/// in control flow → branching dispatch → `None`).
fn sole_sval_transition(stmts: &[&Statement<'_>], s: &str) -> Option<StateVal> {
    let mut found: Option<StateVal> = None;
    for &st in stmts {
        if let Some(t) = sval_transition(st, s) {
            if found.is_some() {
                return None;
            }
            found = Some(t);
        } else if writes_name(st, s) {
            return None;
        }
    }
    found
}

/// Per-case end behavior for the string/boolean engine (mirrors `CaseEnd`).
enum SvalEnd {
    Step(StateVal),
    Terminal,
    FallThrough(StateVal),
}

struct SvalCase<'b, 'a> {
    emit: Vec<&'b Statement<'a>>,
    end: SvalEnd,
}

/// Parse every case into (emitted body, end), keyed by its string/boolean label.
fn parse_sval_cases<'b, 'a>(
    switch: &'b SwitchStatement<'a>,
    s: &str,
) -> Option<HashMap<StateVal, SvalCase<'b, 'a>>> {
    let mut out: HashMap<StateVal, SvalCase> = HashMap::new();
    let mut labels: Vec<StateVal> = Vec::with_capacity(switch.cases.len());
    for case in &switch.cases {
        labels.push(sval_literal(case.test.as_ref()?)?);
    }
    for (ci, case) in switch.cases.iter().enumerate() {
        let state = labels[ci].clone();
        let mut flat: Vec<&Statement> = Vec::new();
        flatten_case(&case.consequent, &mut flat);
        let Some(term_idx) = flat.iter().position(|st| is_top_level_control(st)) else {
            // Fall-through into the textually-next case.
            let Some(next_state) = labels.get(ci + 1).cloned() else {
                return None; // off the end of the switch
            };
            let mut emit = Vec::new();
            for &st in &flat {
                if mentions(st, s) {
                    return None;
                }
                emit.push(st);
            }
            if out
                .insert(
                    state,
                    SvalCase {
                        emit,
                        end: SvalEnd::FallThrough(next_state),
                    },
                )
                .is_some()
            {
                return None;
            }
            continue;
        };
        let (end, upto, terminal_incl) = match flat[term_idx] {
            Statement::BreakStatement(b) if b.label.is_none() => {
                let next = sole_sval_transition(&flat[..term_idx], s)?;
                (SvalEnd::Step(next), term_idx, false)
            }
            Statement::ContinueStatement(c) if c.label.is_none() => {
                let next = sole_sval_transition(&flat[..term_idx], s)?;
                (SvalEnd::Step(next), term_idx, false)
            }
            Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => {
                (SvalEnd::Terminal, term_idx + 1, true)
            }
            _ => return None,
        };
        let mut emit = Vec::new();
        for (k, &st) in flat[..upto].iter().enumerate() {
            let is_terminal_stmt = terminal_incl && k == upto - 1;
            if sval_transition(st, s).is_some() {
                continue;
            }
            if mentions(st, s) {
                return None;
            }
            if !is_terminal_stmt && is_top_level_control(st) {
                return None;
            }
            emit.push(st);
        }
        if out.insert(state, SvalCase { emit, end }).is_some() {
            return None; // duplicate case label
        }
    }
    Some(out)
}

/// Run the string/boolean engine on the (init, dispatcher) statement pair.
fn simulate_sval_at<'a>(
    init_stmt: &Statement<'a>,
    loop_stmt: &Statement<'a>,
    s_name: &str,
    alloc: &'a Allocator,
) -> Option<Vec<Statement<'a>>> {
    let (_, init) = sval_init(init_stmt)?;
    // Reuse the integer `detect_loop` only for its structural shape extraction
    // (while/do/for + single switch + no-default + discriminant is `s`); but the
    // test must be parsed over `StateVal`, so re-derive it here from the loop node.
    let (test, body, kind) = match loop_stmt {
        Statement::WhileStatement(w) => {
            (parse_sval_test(&w.test, s_name)?, &w.body, LoopKind::While)
        }
        Statement::DoWhileStatement(d) => (
            parse_sval_test(&d.test, s_name)?,
            &d.body,
            LoopKind::DoWhile,
        ),
        Statement::ForStatement(f) => {
            if f.init.as_ref().is_some_and(|i| for_init_writes(i, s_name)) {
                return None;
            }
            if f.update.as_ref().is_some_and(|u| writes_in_expr(u, s_name)) {
                return None;
            }
            let test = match &f.test {
                Some(t) => parse_sval_test(t, s_name)?,
                None => SvalTest::Always,
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
    if !is_ident(&switch.discriminant, s_name) {
        return None;
    }
    if switch.cases.iter().any(|c| c.test.is_none()) {
        return None; // no `default`
    }
    let cases = parse_sval_cases(switch, s_name)?;

    // Simulate, mirroring the integer `simulate` (including fall-through chains and
    // cycle detection).
    let mut state = init;
    let mut visited: HashSet<StateVal> = HashSet::new();
    let mut seq: Vec<Statement<'a>> = Vec::new();
    for _ in 0..100_000 {
        if kind == LoopKind::While && !test.continues(&state)? {
            return Some(seq);
        }
        if !visited.insert(state.clone()) {
            return None; // revisited → real loop
        }
        let mut ft_seen: HashSet<StateVal> = HashSet::new();
        loop {
            if !ft_seen.insert(state.clone()) {
                return None;
            }
            let case = cases.get(&state)?;
            for st in &case.emit {
                seq.push((*st).clone_in(alloc));
            }
            match &case.end {
                SvalEnd::Step(next) => {
                    state = next.clone();
                    break;
                }
                SvalEnd::Terminal => return Some(seq),
                SvalEnd::FallThrough(next) => {
                    if !visited.insert(next.clone()) {
                        return None;
                    }
                    state = next.clone();
                }
            }
        }
        if kind == LoopKind::DoWhile && !test.continues(&state)? {
            return Some(seq);
        }
    }
    None
}
