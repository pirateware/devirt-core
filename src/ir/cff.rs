//! Recovered control-flow graph for flattened `while`/`switch` dispatchers
//! (Plan P2, read-only modeling step).
//!
//! The linear unflattener (`passes::unflatten`) handles the *acyclic, deterministic*
//! dispatcher — a state machine whose `simulate` walk never branches and never
//! revisits a state. It declines (soundly) on the two shapes that dominate real
//! control-flow flattening:
//!
//!   * **conditional transitions** — `case 1: …; if (c) s = 2; else s = 3; break;`
//!     (the recovered `if`), and
//!   * **back edges** — a state revisited via a transition (the recovered loop).
//!
//! This module *recovers the graph* without yet re-emitting structured AST: it
//! lifts a dispatcher to a [`RecoveredCfg`] whose **nodes are state values** and
//! whose **edges are the exact transitions**, classified by guard
//! (`Always`/`If`/`Else`/`Exit`). Back edges are then found via the dominator tree
//! over the recovered graph (reusing [`super::dom`]).
//!
//! It is **read-only**: nothing here mutates the program. Recovery is exact by
//! construction — a node is "the statements that run in state K", an edge is "the
//! state transition that K performs" — so the graph *is* the state machine, with
//! nothing dropped or reordered. Anything we cannot model exactly ⇒ the builder
//! returns `None` and the caller leaves the dispatcher untouched (sound no-op).
//!
//! The structuring step that turns a [`RecoveredCfg`] back into `if`/`while` lives
//! in `structure.rs` (Plan P2/M2); this is the substrate it consumes.

use std::collections::{HashMap, HashSet};

use oxc_allocator::CloneIn;
use oxc_ast::ast::*;
use oxc_ast::AstBuilder;
use oxc_span::SPAN;
use oxc_syntax::operator::{AssignmentOperator, UnaryOperator};

use super::dom::Dominators;

/// Where a transition goes: a concrete state, or out of the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    State(i64),
    /// The loop test went false, or a `return`/`throw` left the function.
    Exit,
}

/// The condition under which an edge is taken. Conditions are the *original*
/// `if` tests, preserved verbatim (referenced by node address, never synthesized).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guard {
    /// Unconditional successor.
    Always,
    /// Taken when the recorded `if` condition is truthy.
    IfTrue,
    /// The `else` arm of the same `if`.
    IfFalse,
}

/// One transition out of a case block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edge {
    pub guard: Guard,
    pub target: Target,
}

/// A recovered case: the non-dispatch statements that run in this state, plus the
/// transitions out of it. `content`/`cond` are recorded as AST node addresses into
/// the original tree (stable; the structuring step splices them back by identity).
pub struct CaseBlock {
    /// Indices into the original case's flattened statement list that are *content*
    /// (everything that is not a dispatch transition). Kept as indices so the
    /// read-only model holds no AST borrow; structuring resolves them.
    pub content: Vec<usize>,
    /// The conditional guard's statement index (the `if`), when this block exits
    /// via an `IfTrue`/`IfFalse` pair. `None` for an unconditional/terminal exit.
    pub cond_stmt: Option<usize>,
    /// Outgoing edges (1 for unconditional/terminal, 2 for a conditional split).
    pub edges: Vec<Edge>,
}

/// The recovered state machine.
pub struct RecoveredCfg {
    pub entry: i64,
    /// state value → recovered block.
    pub blocks: HashMap<i64, CaseBlock>,
    /// Deterministic state ordering (entry-reachable, discovery order) — the basis
    /// for the dense block numbering used by the dominator computation.
    pub order: Vec<i64>,
}

impl RecoveredCfg {
    /// Dense successor list over `order` indexing, with a synthetic `Exit` node at
    /// the end. Used to compute dominators / back edges.
    ///
    /// The relooper (`super::relooper`) builds its own dense view with predecessors;
    /// these helpers remain the canonical back-edge oracle exercised by the unit
    /// tests below and available for diagnostics.
    #[allow(dead_code)]
    fn dense_succs(&self) -> (Vec<Vec<usize>>, usize) {
        let index: HashMap<i64, usize> = self
            .order
            .iter()
            .enumerate()
            .map(|(i, &s)| (s, i))
            .collect();
        let exit = self.order.len();
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); self.order.len() + 1];
        for (i, &state) in self.order.iter().enumerate() {
            let block = &self.blocks[&state];
            for e in &block.edges {
                let to = match e.target {
                    Target::State(s) => *index.get(&s).unwrap_or(&exit),
                    Target::Exit => exit,
                };
                if !succs[i].contains(&to) {
                    succs[i].push(to);
                }
            }
        }
        (succs, exit)
    }

    /// The back edges `(from_state, to_state)` of the recovered graph — transitions
    /// `u → v` where `v` dominates `u`. Each back edge is a recovered loop.
    #[allow(dead_code)]
    pub fn back_edges(&self) -> Vec<(i64, i64)> {
        let (succs, _exit) = self.dense_succs();
        let entry_idx = 0usize; // entry is always order[0]
        let dom = Dominators::compute(&succs, entry_idx);
        let mut out = Vec::new();
        for (i, &state) in self.order.iter().enumerate() {
            for s in &succs[i] {
                if *s < self.order.len() && dom.dominates(*s, i) {
                    out.push((state, self.order[*s]));
                }
            }
        }
        out
    }

    /// Whether the recovered graph contains any loop (back edge).
    #[allow(dead_code)]
    pub fn has_loop(&self) -> bool {
        !self.back_edges().is_empty()
    }
}

// ---------------------------------------------------------------------------
// Builder: dispatcher AST → RecoveredCfg
// ---------------------------------------------------------------------------

/// A flattened integer literal (`5`, `-1`).
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

/// `s = <int>` as a top-level expression statement → the absolute target state.
/// (P2-v1 models only absolute transitions in conditional arms; relative steps
/// inside branches are left to the linear simulator's enumeration and decline
/// here.)
fn abs_transition(stmt: &Statement<'_>, s: &str) -> Option<i64> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::AssignmentExpression(a) = &es.expression else {
        return None;
    };
    if a.operator != AssignmentOperator::Assign {
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

/// True if `stmt` writes `s` anywhere within it (used to reject content that
/// touches dispatch state in unmodeled ways).
fn writes_state(stmt: &Statement<'_>, s: &str) -> bool {
    use oxc_ast_visit::Visit;
    struct W<'r> {
        s: &'r str,
        found: bool,
    }
    impl<'a> Visit<'a> for W<'_> {
        fn visit_assignment_target(&mut self, t: &AssignmentTarget<'a>) {
            if let AssignmentTarget::AssignmentTargetIdentifier(id) = t {
                if id.name == self.s {
                    self.found = true;
                }
            }
            oxc_ast_visit::walk::walk_assignment_target(self, t);
        }
    }
    let mut w = W { s, found: false };
    w.visit_statement(stmt);
    w.found
}

/// True if `stmt` references `s` (read or write) anywhere.
fn mentions_state(stmt: &Statement<'_>, s: &str) -> bool {
    use oxc_ast_visit::Visit;
    struct M<'r> {
        s: &'r str,
        found: bool,
    }
    impl<'a> Visit<'a> for M<'_> {
        fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
            if id.name == self.s {
                self.found = true;
            }
        }
    }
    let mut m = M { s, found: false };
    m.visit_statement(stmt);
    m.found
}

/// Parse one switch case's flattened statements into a [`CaseBlock`]. Returns
/// `None` on any shape not modeled exactly.
///
/// Recognized exits (the last meaningful statement of the case):
///   * `s = K; break;`                              → one `Always` edge to K
///   * `if (c) s = A; else s = B; break;`           → `IfTrue`→A, `IfFalse`→B
///   * `if (c) s = A; break;` (no else, falls through to a trailing `s = B`)
///   * `return`/`throw`                             → one `Exit` edge
///   * `break`/`continue` with a preceding sole `s = K` (the linear shape)
fn parse_case_block(
    flat: &[&Statement<'_>],
    s: &str,
    continues: &dyn Fn(i64) -> bool,
) -> Option<CaseBlock> {
    // Find the terminator (first top-level break/continue/return/throw).
    let term_idx = flat.iter().position(|st| is_top_level_control(st))?;

    // Content = everything before the terminator that is neither a transition nor
    // the conditional dispatch `if`. Collect transitions/conditional as edges.
    let mut content: Vec<usize> = Vec::new();
    let mut edges: Vec<Edge> = Vec::new();
    let mut cond_stmt: Option<usize> = None;

    // Classify the terminator first.
    let terminal = matches!(
        flat[term_idx],
        Statement::ReturnStatement(_) | Statement::ThrowStatement(_)
    );

    // A transition to state `k` leaves the machine iff the loop test goes false
    // there (`s = 0` under `while (s)`); otherwise it is an in-machine edge.
    let target_of = |k: i64| {
        if continues(k) {
            Target::State(k)
        } else {
            Target::Exit
        }
    };

    for (k, &st) in flat.iter().enumerate().take(term_idx) {
        if let Some(t) = abs_transition(st, s) {
            // Unconditional absolute transition.
            edges.push(Edge {
                guard: Guard::Always,
                target: target_of(t),
            });
            continue;
        }
        if let Some((cond_true, cond_false)) = conditional_transition(st, s, &target_of) {
            cond_stmt = Some(k);
            edges.push(Edge {
                guard: Guard::IfTrue,
                target: cond_true,
            });
            edges.push(Edge {
                guard: Guard::IfFalse,
                target: cond_false,
            });
            continue;
        }
        if writes_state(st, s) || mentions_state(st, s) {
            return None; // content observes/alters state in an unmodeled way
        }
        if is_top_level_control(st) {
            return None; // an unmodeled second exit
        }
        content.push(k);
    }

    if terminal {
        // Emit the terminator as content; its edge is Exit.
        content.push(term_idx);
        edges.push(Edge {
            guard: Guard::Always,
            target: Target::Exit,
        });
        return Some(CaseBlock {
            content,
            cond_stmt,
            edges,
        });
    }

    // Non-terminal (break/continue): there must be at least one transition edge.
    // A bare break with no transition means the loop test is re-checked at the
    // unchanged state; if the test still continues there we cannot model it (would
    // spin), so require at least one edge.
    if edges.is_empty() {
        return None;
    }
    Some(CaseBlock {
        content,
        cond_stmt,
        edges,
    })
}

/// `if (cond) s = A; else s = B;` (each arm exactly one absolute transition) →
/// `(target A, target B)`, each mapped through `target_of` (so an exit-state
/// transition becomes `Exit`). Also accepts an arm that is a `{ s = X }` block.
fn conditional_transition(
    stmt: &Statement<'_>,
    s: &str,
    target_of: &dyn Fn(i64) -> Target,
) -> Option<(Target, Target)> {
    let Statement::IfStatement(iff) = stmt else {
        return None;
    };
    let then_t = arm_transition(&iff.consequent, s, target_of)?;
    let else_t = match &iff.alternate {
        Some(alt) => arm_transition(alt, s, target_of)?,
        None => return None, // no else → not a clean two-way split (v1 declines)
    };
    Some((then_t, else_t))
}

/// A branch arm that is exactly one absolute transition `s = K` (optionally
/// wrapped in a single-statement block) → its target (via `target_of`).
fn arm_transition(
    stmt: &Statement<'_>,
    s: &str,
    target_of: &dyn Fn(i64) -> Target,
) -> Option<Target> {
    match stmt {
        Statement::BlockStatement(b) if b.body.len() == 1 => {
            abs_transition(&b.body[0], s).map(target_of)
        }
        other => abs_transition(other, s).map(target_of),
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

/// Flatten a case consequent, inlining transparent (no block-scoped decl) blocks.
fn flatten_case<'b, 'a>(stmts: &'b [Statement<'a>], out: &mut Vec<&'b Statement<'a>>) {
    for st in stmts {
        if let Statement::BlockStatement(b) = st {
            let transparent = !b.body.iter().any(|st| {
                matches!(
                    st,
                    Statement::ClassDeclaration(_) | Statement::FunctionDeclaration(_)
                ) || matches!(st, Statement::VariableDeclaration(vd)
                        if vd.kind != VariableDeclarationKind::Var)
            });
            if transparent {
                flatten_case(&b.body, out);
                continue;
            }
        }
        out.push(st);
    }
}

/// Build a [`RecoveredCfg`] from a switch dispatcher over state `s`, entered at
/// `entry`. `loop_test_exit` indicates the loop has a falsy/relational exit test
/// (so a state with no transition exits the machine). Returns `None` if any case
/// is not modeled exactly.
pub fn build(
    switch: &SwitchStatement<'_>,
    s: &str,
    entry: i64,
    continues: &dyn Fn(i64) -> bool,
) -> Option<RecoveredCfg> {
    let mut blocks: HashMap<i64, CaseBlock> = HashMap::new();
    for case in &switch.cases {
        let state = int_literal(case.test.as_ref()?)?;
        let mut flat: Vec<&Statement> = Vec::new();
        flatten_case(&case.consequent, &mut flat);
        let block = parse_case_block(&flat, s, continues)?;
        if blocks.insert(state, block).is_some() {
            return None; // duplicate label
        }
    }
    if !blocks.contains_key(&entry) {
        return None;
    }

    // Discovery order from entry (BFS over State edges) — defines block numbering.
    let mut order: Vec<i64> = Vec::new();
    let mut seen: HashSet<i64> = HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(entry);
    seen.insert(entry);
    while let Some(st) = queue.pop_front() {
        order.push(st);
        if let Some(b) = blocks.get(&st) {
            for e in &b.edges {
                if let Target::State(t) = e.target {
                    if blocks.contains_key(&t) && seen.insert(t) {
                        queue.push_back(t);
                    }
                }
            }
        }
    }
    // Keep only entry-reachable blocks (unreachable cases are dead).
    blocks.retain(|k, _| seen.contains(k));

    Some(RecoveredCfg {
        entry,
        blocks,
        order,
    })
}

// ---------------------------------------------------------------------------
// Structuring (Plan P2/M2): RecoveredCfg → structured AST
// ---------------------------------------------------------------------------

/// Structure a recovered self-loop into a `do { … } while (cond)`.
///
/// v1 handles exactly the shape the linear `simulate` declines on but which is a
/// single recovered loop: **one** entry state whose only transition is a
/// conditional split with one arm back to itself and the other to `Exit`:
///
/// ```text
/// case E: <content>; if (cond) s = E; else s = <exit>; break;   (while/do over s)
/// ```
///
/// → `do { <content> } while (cond)`  (or `while (!cond)` when the self-arm is the
/// `else`). Sound: the body runs at least once (the machine always enters state E
/// since the loop test continues there), the content runs in the same order each
/// iteration, and the loop repeats exactly while the original transition pointed
/// back to E. Returns `None` (decline → no transform) on any other shape.
pub fn structure<'a>(
    switch: &SwitchStatement<'a>,
    s: &str,
    cfg: &RecoveredCfg,
    ast: AstBuilder<'a>,
) -> Option<Vec<Statement<'a>>> {
    // Exactly one recovered state, and it is the entry.
    if cfg.order.len() != 1 || cfg.order[0] != cfg.entry {
        return None;
    }
    let block = cfg.blocks.get(&cfg.entry)?;
    // A single conditional split: IfTrue + IfFalse, one to self, one to Exit.
    if block.edges.len() != 2 {
        return None;
    }
    let cond_idx = block.cond_stmt?;
    let (mut if_true, mut if_false) = (None, None);
    for e in &block.edges {
        match e.guard {
            Guard::IfTrue => if_true = Some(e.target),
            Guard::IfFalse => if_false = Some(e.target),
            Guard::Always => return None, // a stray unconditional edge → not this shape
        }
    }
    let (if_true, if_false) = (if_true?, if_false?);
    // One arm loops back to the entry, the other exits.
    let self_on_true = match (if_true, if_false) {
        (Target::State(t), Target::Exit) if t == cfg.entry => true,
        (Target::Exit, Target::State(t)) if t == cfg.entry => false,
        _ => return None,
    };

    // Re-flatten the entry case to resolve content/cond by index.
    let case = switch
        .cases
        .iter()
        .find(|c| c.test.as_ref().and_then(int_literal) == Some(cfg.entry))?;
    let mut flat: Vec<&Statement> = Vec::new();
    flatten_case(&case.consequent, &mut flat);

    // The conditional guard's test expression (preserved verbatim).
    let Statement::IfStatement(iff) = flat.get(cond_idx)? else {
        return None;
    };
    let mut cond = iff.test.clone_in(ast.allocator);
    if !self_on_true {
        // Self-arm is the `else`: loop while the condition is false.
        cond = ast.expression_unary(SPAN, UnaryOperator::LogicalNot, cond);
    }

    // Body = the content statements, in order, cloned into the arena.
    let mut body = ast.vec();
    for &i in &block.content {
        // Content must never reference the dispatch var (parse guaranteed it).
        debug_assert!(!mentions_state(flat[i], s));
        body.push(flat[i].clone_in(ast.allocator));
    }
    let body_block = ast.statement_block(SPAN, body);
    let do_while = ast.statement_do_while(SPAN, body_block, cond);
    Some(vec![do_while])
}

// ---------------------------------------------------------------------------
// General structuring (Plan A2): RecoveredCfg → if/else + while via the relooper
// ---------------------------------------------------------------------------

/// A [`super::relooper::CaseSource`] backed by the original switch: it resolves a
/// recovered state's content statements and guard condition straight from the
/// case AST, cloning verbatim into the arena. This is the bridge from the
/// read-only recovered model to re-emitted structured AST — every statement and
/// condition the relooper places comes through here, so nothing is synthesized.
struct SwitchSource<'b, 'a> {
    switch: &'b SwitchStatement<'a>,
    s: &'b str,
    cfg: &'b RecoveredCfg,
    ast: AstBuilder<'a>,
}

impl<'b, 'a> SwitchSource<'b, 'a> {
    /// Re-flatten the case for state `k` (same flattening the builder used), so the
    /// stored content/cond indices resolve to the exact statements.
    fn flat_case(&self, k: i64) -> Option<Vec<&'b Statement<'a>>> {
        let case = self
            .switch
            .cases
            .iter()
            .find(|c| c.test.as_ref().and_then(int_literal) == Some(k))?;
        let mut flat: Vec<&Statement> = Vec::new();
        flatten_case(&case.consequent, &mut flat);
        Some(flat)
    }
}

impl<'b, 'a> super::relooper::CaseSource<'a> for SwitchSource<'b, 'a> {
    fn content(&self, k: i64) -> Vec<Statement<'a>> {
        let (Some(block), Some(flat)) = (self.cfg.blocks.get(&k), self.flat_case(k)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for &i in &block.content {
            if let Some(st) = flat.get(i) {
                // The builder guaranteed content never mentions the dispatch var.
                debug_assert!(!mentions_state(st, self.s));
                out.push(st.clone_in(self.ast.allocator));
            }
        }
        out
    }

    fn cond(&self, k: i64) -> Option<Expression<'a>> {
        let block = self.cfg.blocks.get(&k)?;
        let cond_idx = block.cond_stmt?;
        let flat = self.flat_case(k)?;
        let Statement::IfStatement(iff) = flat.get(cond_idx)? else {
            return None;
        };
        Some(iff.test.clone_in(self.ast.allocator))
    }
}

/// Structure an arbitrary **reducible** recovered dispatcher into `if/else` +
/// `while(true){…}` via the general relooper (Plan A2). The condition emitted for a
/// two-way split is the verbatim original `if` test, oriented for the `IfTrue` arm
/// (the relooper places `IfTrue` as the `then` branch, `IfFalse` as the `else`).
///
/// Declines (`None`) on any irreducible graph or any shape the relooper cannot
/// place soundly (the caller then leaves the dispatcher untouched).
pub fn structure_general<'a>(
    switch: &SwitchStatement<'a>,
    s: &str,
    cfg: &RecoveredCfg,
    ast: AstBuilder<'a>,
) -> Option<Vec<Statement<'a>>> {
    let src = SwitchSource {
        switch,
        s,
        cfg,
        ast,
    };
    super::relooper::structure_general(cfg, &src, ast)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a RecoveredCfg by hand (bypassing AST) to unit-test the graph algos.
    fn block(edges: Vec<Edge>) -> CaseBlock {
        CaseBlock {
            content: Vec::new(),
            cond_stmt: None,
            edges,
        }
    }
    fn always(t: i64) -> Edge {
        Edge {
            guard: Guard::Always,
            target: Target::State(t),
        }
    }
    fn exit() -> Edge {
        Edge {
            guard: Guard::Always,
            target: Target::Exit,
        }
    }
    fn cfg(entry: i64, pairs: Vec<(i64, CaseBlock)>) -> RecoveredCfg {
        let mut blocks = HashMap::new();
        for (k, b) in pairs {
            blocks.insert(k, b);
        }
        // recompute order via BFS like build()
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        let mut q = std::collections::VecDeque::new();
        q.push_back(entry);
        seen.insert(entry);
        while let Some(s) = q.pop_front() {
            order.push(s);
            if let Some(b) = blocks.get(&s) {
                for e in &b.edges {
                    if let Target::State(t) = e.target {
                        if blocks.contains_key(&t) && seen.insert(t) {
                            q.push_back(t);
                        }
                    }
                }
            }
        }
        RecoveredCfg {
            entry,
            blocks,
            order,
        }
    }

    #[test]
    fn linear_chain_has_no_loop() {
        // 1 -> 2 -> 3 -> Exit
        let g = cfg(
            1,
            vec![
                (1, block(vec![always(2)])),
                (2, block(vec![always(3)])),
                (3, block(vec![exit()])),
            ],
        );
        assert_eq!(g.order, vec![1, 2, 3]);
        assert!(!g.has_loop());
        assert!(g.back_edges().is_empty());
    }

    #[test]
    fn self_loop_is_detected() {
        // 1: if (c) -> 1 else -> Exit   (a do/while self-loop)
        let g = cfg(
            1,
            vec![(
                1,
                block(vec![
                    Edge {
                        guard: Guard::IfTrue,
                        target: Target::State(1),
                    },
                    Edge {
                        guard: Guard::IfFalse,
                        target: Target::Exit,
                    },
                ]),
            )],
        );
        assert!(g.has_loop());
        assert_eq!(g.back_edges(), vec![(1, 1)]);
    }

    #[test]
    fn branch_diamond_no_loop() {
        // 1 -> {2,3} -> 4 -> Exit  (an if/else that re-merges; reducible, acyclic)
        let g = cfg(
            1,
            vec![
                (
                    1,
                    block(vec![
                        Edge {
                            guard: Guard::IfTrue,
                            target: Target::State(2),
                        },
                        Edge {
                            guard: Guard::IfFalse,
                            target: Target::State(3),
                        },
                    ]),
                ),
                (2, block(vec![always(4)])),
                (3, block(vec![always(4)])),
                (4, block(vec![exit()])),
            ],
        );
        assert!(!g.has_loop());
        assert_eq!(g.order.len(), 4);
    }

    #[test]
    fn while_loop_back_edge() {
        // 1 -> 2; 2: if (c) -> 3 else -> Exit; 3 -> 2  (a while with body 3)
        let g = cfg(
            1,
            vec![
                (1, block(vec![always(2)])),
                (
                    2,
                    block(vec![
                        Edge {
                            guard: Guard::IfTrue,
                            target: Target::State(3),
                        },
                        Edge {
                            guard: Guard::IfFalse,
                            target: Target::Exit,
                        },
                    ]),
                ),
                (3, block(vec![always(2)])),
            ],
        );
        assert!(g.has_loop());
        // The back edge is 3 -> 2 (2 dominates 3).
        assert_eq!(g.back_edges(), vec![(3, 2)]);
    }
}
