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
//! Soundness is by construction + heavy gating. We transform only when every check
//! below holds, and otherwise leave the construct untouched:
//!   * the state var `s` is referenced nowhere but the initializer, the `while (s)`
//!     test, the `switch (s)` discriminant, and top-level `s = <int>` transitions
//!     inside cases (so emitting bodies can't observe or alter dispatch state);
//!   * the loop is `while (s) { switch (s) { … } }` with no `default` clause;
//!   * each visited case ends in `break`/`continue` (with exactly one top-level
//!     integer transition) or a `return`/`throw` terminator — never falls through,
//!     and contains no other top-level `break`/`continue`/`return`/`throw`;
//!   * the simulated state chain terminates: state `0` (falsy) exits the loop, and
//!     revisiting a state aborts the whole rewrite (a real loop, not a chain).
//!
//! Because `s` is dispatch-only and the chain is acyclic and deterministic, the
//! emitted sequence runs the exact same statements in the exact same order.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, Expression, Program, SimpleAssignmentTarget, Statement,
    SwitchStatement,
};
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
            let Some(switch) = while_switch(&stmts[i + 1], &s_name) else {
                continue;
            };
            // `s` must not be referenced anywhere in this list except the init and
            // the while construct (indices i, i+1).
            if !dispatch_only_in_list(stmts, i, &s_name) {
                continue;
            }
            let Some(cases) = parse_cases(switch, &s_name) else {
                continue;
            };
            let Some(seq) = simulate(init, &cases, ctx.ast.allocator) else {
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

/// If `stmt` is `while (<s>) { switch (<s>) { … } }` with no `default`, return the
/// switch. The test and discriminant must both be the bare identifier `s` (so the
/// loop exits exactly when `s` becomes `0`).
fn while_switch<'b, 'a>(stmt: &'b Statement<'a>, s: &str) -> Option<&'b SwitchStatement<'a>> {
    let Statement::WhileStatement(w) = stmt else {
        return None;
    };
    if !is_ident(&w.test, s) {
        return None;
    }
    let switch = match &w.body {
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
    Some(switch)
}

fn is_ident(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name == name)
}

/// What a case does once its body has run.
enum CaseEnd {
    /// Transition to a state then re-enter the loop (trailing `break`/`continue`).
    Goto(i64),
    /// `return`/`throw`: the machine stops here.
    Terminal,
}

struct ParsedCase<'b, 'a> {
    emit: Vec<&'b Statement<'a>>,
    end: CaseEnd,
}

/// Parse every case into (emitted body, end behavior), keyed by its integer state.
/// Returns `None` (bail) on any shape we don't model soundly.
fn parse_cases<'b, 'a>(
    switch: &'b SwitchStatement<'a>,
    s: &str,
) -> Option<HashMap<i64, ParsedCase<'b, 'a>>> {
    let mut out = HashMap::new();
    for case in &switch.cases {
        let state = int_literal(case.test.as_ref()?)?;
        let body = &case.consequent;
        if body.is_empty() {
            return None; // empty case → fall-through; unmodeled
        }
        let last = &body[body.len() - 1];
        let (end, drop_last) = match last {
            Statement::BreakStatement(b) if b.label.is_none() => {
                // Need exactly one top-level `s = <int>` transition.
                let next = sole_transition(&body[..body.len() - 1], s)?;
                (CaseEnd::Goto(next), true)
            }
            Statement::ContinueStatement(c) if c.label.is_none() => {
                let next = sole_transition(&body[..body.len() - 1], s)?;
                (CaseEnd::Goto(next), true)
            }
            Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => {
                (CaseEnd::Terminal, false)
            }
            _ => return None, // fall-through or unmodeled terminator
        };

        // Statements to emit: everything except the trailing break/continue and
        // any top-level `s = <int>` transition. Validate the remainder is
        // dispatch-clean (no `s`, no stray top-level break/continue/return/throw).
        // For a `return`/`throw` terminator, the terminator is the last emitted
        // statement and is exempt from the top-level-control check.
        let upto = if drop_last {
            body.len() - 1
        } else {
            body.len()
        };
        let mut emit = Vec::new();
        for (k, st) in body[..upto].iter().enumerate() {
            let is_terminal_stmt = !drop_last && k == upto - 1;
            if is_state_transition(st, s).is_some() {
                continue; // drop the transition assignment (dead before a return)
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

/// Among top-level statements, find `s = <int>` transitions. Require exactly one
/// (the next state); none or a non-top-level transition → `None` (bail).
fn sole_transition(stmts: &[Statement<'_>], s: &str) -> Option<i64> {
    let mut found: Option<i64> = None;
    for st in stmts {
        if let Some(v) = is_state_transition(st, s) {
            if found.is_some() {
                return None; // more than one unconditional transition
            }
            found = Some(v);
        } else if writes_name(st, s) {
            return None; // a transition nested in control flow → branching dispatch
        }
    }
    found
}

/// `s = <int>;` as a top-level statement → the assigned value.
fn is_state_transition(stmt: &Statement<'_>, s: &str) -> Option<i64> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::AssignmentExpression(a) = &es.expression else {
        return None;
    };
    if !matches!(a.operator, oxc_syntax::operator::AssignmentOperator::Assign) {
        return None;
    }
    let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
        return None;
    };
    if id.name != s {
        return None;
    }
    int_literal(&a.right)
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
fn simulate<'a>(
    init: i64,
    cases: &HashMap<i64, ParsedCase<'_, 'a>>,
    alloc: &'a Allocator,
) -> Option<Vec<Statement<'a>>> {
    let mut state = init;
    let mut visited: HashSet<i64> = HashSet::new();
    let mut seq: Vec<Statement<'a>> = Vec::new();
    // Bound iterations defensively (cycle detection already covers it).
    for _ in 0..10_000 {
        if state == 0 {
            return Some(seq); // `while (s)` exits on falsy state
        }
        if !visited.insert(state) {
            return None; // revisited state → real loop, not a linear chain
        }
        let case = cases.get(&state)?; // no matching case + no default → infinite no-op
        for st in &case.emit {
            // Deep-clone each emitted statement into the arena so it survives the
            // splice that replaces the original dispatcher.
            seq.push((*st).clone_in(alloc));
        }
        match case.end {
            CaseEnd::Goto(next) => state = next,
            CaseEnd::Terminal => return Some(seq),
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

/// Does `stmt` write `name` (assignment/update/binding) anywhere within it?
fn writes_name(stmt: &Statement<'_>, name: &str) -> bool {
    struct W<'r> {
        name: &'r str,
        found: bool,
    }
    impl<'a, 'r> Visit<'a> for W<'r> {
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
    let mut w = W { name, found: false };
    w.visit_statement(stmt);
    w.found
}
