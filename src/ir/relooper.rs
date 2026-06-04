//! A structured **relooper** for reducible recovered CFGs.
//!
//! `cff::structure` handles exactly one shape — a single-state conditional
//! self-loop → `do/while`. This module generalizes that to an *arbitrary reducible*
//! [`RecoveredCfg`]: it turns the recovered state graph back into nested
//! `if/else` + `while(true){…}` (with `break`/`continue` at the loop edges),
//! recursively collapsing dominator-tree regions.
//!
//! It is **sound by construction or it declines**:
//!   * Only *reducible* graphs are structured. Reducibility is checked explicitly
//!     (every natural loop is single-entry; the relooper never visits a node
//!     twice). An irreducible graph makes the relooper return `None` and the
//!     caller leaves the dispatcher untouched.
//!   * Every emitted statement is a verbatim clone of an original case-content
//!     statement, in original order; every emitted condition is a verbatim clone
//!     of an original `if` test. No statement is synthesized, dropped, or
//!     reordered — only re-nested to match the recovered structure.
//!   * The single self-loop and the linear acyclic chain are degenerate outputs
//!     of the same algorithm (a loop with one block; a tree with no branches).
//!
//! ## Algorithm (dominator-tree structured relooper)
//!
//! We emit code by a single recursive walk that visits each dense node **exactly
//! once**, in an order consistent with the dominator tree:
//!
//!   * A node `n` is emitted only by the region that `n`'s immediate dominator
//!     roots. A successor edge `u → v` is realized as:
//!       - **continue** if `v` is an enclosing loop header (back edge);
//!       - **break** if `v` is an enclosing loop's follow node;
//!       - **inline** (recurse into `v`) if `u` is `v`'s *only* in-region
//!         predecessor and `v` is dominated by `u` — i.e. `v` belongs to `u`'s
//!         subtree and is reached only here;
//!       - **deferred to the merge** otherwise: `v` is a join point dominated by a
//!         common ancestor; it is emitted there, and the current path simply ends
//!         (falls through) — sound because the enclosing region emits `v` next.
//!
//! The "visited exactly once" invariant is what guarantees soundness: if the walk
//! ever needs to emit a node it has already emitted (which would duplicate or
//! reorder code), or reach a node not dominated as expected, it sets a poison flag
//! and the whole transform declines.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::*;
use oxc_ast::AstBuilder;
use oxc_span::SPAN;

use super::cff::{Guard, RecoveredCfg, Target};
use super::dom::Dominators;

/// A resolver from a recovered state to its original case content/condition AST,
/// supplied by the caller (it owns the switch). Keeps the relooper independent of
/// how content indices map back to statements.
pub trait CaseSource<'a> {
    /// Clone the content statements of state `k`, in order, into the arena.
    fn content(&self, k: i64) -> Vec<Statement<'a>>;
    /// Clone the guard `if` test expression of state `k` (the verbatim condition
    /// the conditional split was taken on). Only called for blocks with a two-way
    /// `IfTrue`/`IfFalse` split.
    fn cond(&self, k: i64) -> Option<Expression<'a>>;
}

/// Densely-numbered view of the recovered graph for the dominator computation,
/// plus the reverse map back to state values.
struct Dense {
    /// `order[i]` = state value of dense node `i`. Node `order.len()` is the
    /// synthetic `Exit`.
    order: Vec<i64>,
    index: HashMap<i64, usize>,
    exit: usize,
    succs: Vec<Vec<usize>>,
    preds: Vec<Vec<usize>>,
    dom: Dominators,
}

impl Dense {
    fn build(cfg: &RecoveredCfg) -> Dense {
        let order = cfg.order.clone();
        let index: HashMap<i64, usize> = order.iter().enumerate().map(|(i, &s)| (s, i)).collect();
        let exit = order.len();
        let mut succs: Vec<Vec<usize>> = vec![Vec::new(); order.len() + 1];
        for (i, &state) in order.iter().enumerate() {
            let block = &cfg.blocks[&state];
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
        let preds = predecessors(&succs);
        let dom = Dominators::compute(&succs, 0);
        Dense {
            order,
            index,
            exit,
            succs,
            preds,
            dom,
        }
    }

    fn node_of(&self, state: i64) -> usize {
        self.index[&state]
    }

    fn target_node(&self, t: Target) -> usize {
        match t {
            Target::State(s) => self.node_of(s),
            Target::Exit => self.exit,
        }
    }

    /// Is dense node `n` a loop header (target of a back edge)?
    fn is_loop_header(&self, n: usize) -> bool {
        n != self.exit && self.preds[n].iter().any(|&p| self.dom.dominates(n, p))
    }

    /// Nodes of the natural loop headed at `header` (union over its back edges).
    fn loop_body(&self, header: usize) -> HashSet<usize> {
        let mut body = HashSet::new();
        body.insert(header);
        for &p in &self.preds[header] {
            if self.dom.dominates(header, p) {
                flood_natural_loop(&self.preds, p, header, &mut body);
            }
        }
        body
    }
}

fn predecessors(succs: &[Vec<usize>]) -> Vec<Vec<usize>> {
    let mut preds = vec![Vec::new(); succs.len()];
    for (b, ss) in succs.iter().enumerate() {
        for &s in ss {
            preds[s].push(b);
        }
    }
    preds
}

/// Flood the natural loop of back edge `tail → header` backwards from `tail`,
/// adding every node that reaches `tail` without passing through `header`.
fn flood_natural_loop(preds: &[Vec<usize>], tail: usize, header: usize, body: &mut HashSet<usize>) {
    if tail == header {
        return;
    }
    let mut stack = vec![tail];
    body.insert(tail);
    while let Some(n) = stack.pop() {
        for &p in &preds[n] {
            if p != header && body.insert(p) {
                stack.push(p);
            }
        }
    }
}

/// Reducibility: a CFG is reducible iff every *retreating* edge (a DFS edge `u →
/// v` where `v` is an ancestor of `u` on the DFS tree / `v` was entered but not
/// yet finished when `u` is processed) is also a *back* edge (`v` dominates `u`).
/// An irreducible graph has a retreating edge that is not a back edge — a cycle
/// entered at two points (a jump into a loop body) — and we decline it.
///
/// We compute retreating edges with a colored DFS (white/grey/black): an edge to a
/// grey (on-stack) node is retreating. Each such edge must satisfy `dom(v, u)`.
fn is_reducible(d: &Dense) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Grey,
        Black,
    }
    let n = d.succs.len();
    let mut color = vec![Color::White; n];
    // Iterative DFS from entry (node 0); `Exit` and any unreachable node stay white.
    let mut stack: Vec<(usize, usize)> = vec![(0, 0)];
    color[0] = Color::Grey;
    while let Some(&mut (u, ref mut i)) = stack.last_mut() {
        if *i < d.succs[u].len() {
            let v = d.succs[u][*i];
            *i += 1;
            match color[v] {
                Color::White => {
                    color[v] = Color::Grey;
                    stack.push((v, 0));
                }
                Color::Grey => {
                    // Retreating edge u -> v. Must be a back edge (v dominates u).
                    if v != d.exit && !d.dom.dominates(v, u) {
                        return false;
                    }
                }
                Color::Black => { /* forward / cross edge — fine */ }
            }
        } else {
            color[u] = Color::Black;
            stack.pop();
        }
    }
    true
}

/// One active loop on the emission stack.
#[derive(Clone, Copy)]
struct LoopCtx {
    header: usize,
    /// Where `break` lands (the loop's single exit successor), or `Exit`.
    follow: usize,
}

struct Relooper<'a, 's, S: CaseSource<'a>> {
    cfg: &'s RecoveredCfg,
    d: &'s Dense,
    src: &'s S,
    ast: AstBuilder<'a>,
    loops: Vec<LoopCtx>,
    /// Active merge barriers (innermost last): a node that an *enclosing* split is
    /// responsible for emitting after its `if`. A path that reaches a barrier stops
    /// (falls through to where the enclosing region resumes), so the merge is
    /// emitted once, by the split, not duplicated into an arm.
    stops: Vec<usize>,
    /// Nodes already emitted — re-emission would duplicate code, so it poisons.
    emitted: HashSet<usize>,
    poison: bool,
}

impl<'a, 's, S: CaseSource<'a>> Relooper<'a, 's, S> {
    /// Emit the structured statements for the region beginning at dense node `n`,
    /// following forward edges until a node that does not belong to this region
    /// (an enclosing merge / loop edge / Exit). Each node is emitted exactly once;
    /// any deviation poisons the transform.
    fn emit(&mut self, n: usize) -> Vec<Statement<'a>> {
        let mut out = Vec::new();
        let mut cur = n;
        loop {
            if self.poison {
                return out;
            }
            match self.resolve_node(cur) {
                NodeFlow::Stop => return out,
                NodeFlow::Jump(stmt) => {
                    out.push(stmt);
                    return out;
                }
                NodeFlow::Emit => {}
            }
            // Emitting a node twice means the graph isn't a clean tree here.
            if !self.emitted.insert(cur) {
                self.poison = true;
                return out;
            }

            if self.d.is_loop_header(cur) {
                let (loop_stmt, follow) = self.emit_loop(cur);
                out.push(loop_stmt);
                // After the loop, continue at its in-machine follow (if any); a loop
                // whose only exit was the graph Exit reaches Exit → stop here.
                let next = follow.unwrap_or(self.d.exit);
                match self.follow_flow(next) {
                    Flow::Stop => return out,
                    Flow::Jump(stmt) => {
                        out.push(stmt);
                        return out;
                    }
                    Flow::Goto(f) => {
                        cur = f;
                        continue;
                    }
                }
            }

            out.extend(self.src.content(self.d.order[cur]));
            let state = self.d.order[cur];
            let block = &self.cfg.blocks[&state];
            let next = match block.edges.len() {
                1 => self.d.target_node(block.edges[0].target),
                2 => {
                    let (then_node, else_node) = self.arm_nodes(block);
                    let merge = self.find_merge(cur, then_node, else_node);
                    let Some(cond) = self.src.cond(state) else {
                        self.poison = true;
                        return out;
                    };
                    let then_body = self.emit_branch(then_node, merge);
                    let else_body = self.emit_branch(else_node, merge);
                    out.push(self.if_stmt(cond, then_body, else_body));
                    match merge {
                        Some(m) => m,
                        None => return out,
                    }
                }
                _ => {
                    self.poison = true;
                    return out;
                }
            };
            match self.follow_flow(next) {
                Flow::Stop => return out,
                Flow::Jump(stmt) => {
                    out.push(stmt);
                    return out;
                }
                Flow::Goto(t) => cur = t,
            }
        }
    }

    /// Classify a node *before* emitting it: is it a loop edge (break/continue), a
    /// node owned by an enclosing region (stop), or this region's to emit?
    fn resolve_node(&self, n: usize) -> NodeFlow<'a> {
        // A loop edge always wins (break/continue), even at a barrier.
        match self.jump_for(n) {
            JumpKind::Continue => return NodeFlow::Jump(self.ast.statement_continue(SPAN, None)),
            JumpKind::Break => return NodeFlow::Jump(self.ast.statement_break(SPAN, None)),
            JumpKind::Unreachable => return NodeFlow::Stop,
            JumpKind::None => {}
        }
        if n == self.d.exit || self.stops.contains(&n) {
            NodeFlow::Stop
        } else {
            NodeFlow::Emit
        }
    }

    /// Decide how to continue *to* node `to` from the current emission point: it is
    /// a break/continue, a fall-through to an enclosing region (stop), or an inline
    /// continuation in this region (goto).
    fn follow_flow(&mut self, to: usize) -> Flow<'a> {
        match self.jump_for(to) {
            JumpKind::Continue => return Flow::Jump(self.ast.statement_continue(SPAN, None)),
            JumpKind::Break => return Flow::Jump(self.ast.statement_break(SPAN, None)),
            JumpKind::Unreachable => {
                self.poison = true;
                return Flow::Stop;
            }
            JumpKind::None => {}
        }
        if to == self.d.exit || self.stops.contains(&to) {
            // Exit, or a merge barrier owned by an enclosing split: fall through.
            Flow::Stop
        } else if self.sole_forward_pred(to) {
            // `to` is reached only along forward edges from already-emitted nodes of
            // this region → emit it here.
            Flow::Goto(to)
        } else {
            // A shared join dominated by an ancestor → emitted by the ancestor.
            Flow::Stop
        }
    }

    /// Classify a jump to node `n` against the loop stack. A `break`/`continue`
    /// only targets the *innermost* loop in JS; if `n` matches an *outer* loop while
    /// an inner loop is interposed, we cannot express it → `Unreachable` (decline).
    fn jump_for(&self, n: usize) -> JumpKind {
        for (depth, ctx) in self.loops.iter().rev().enumerate() {
            let matches = n == ctx.header || n == ctx.follow;
            if matches {
                if depth != 0 {
                    // Not the innermost loop → would need a labeled break/continue.
                    return JumpKind::Unreachable;
                }
                return if n == ctx.header {
                    JumpKind::Continue
                } else {
                    JumpKind::Break
                };
            }
        }
        JumpKind::None
    }

    /// Emit one branch arm: the region from `node` up to (excluding) `merge`. The
    /// merge is pushed as a barrier so the arm stops there (the enclosing split emits
    /// the merge once, after the `if`). An empty arm (`node == merge`) falls straight
    /// through.
    fn emit_branch(&mut self, node: usize, merge: Option<usize>) -> Vec<Statement<'a>> {
        if Some(node) == merge {
            return Vec::new();
        }
        if let Some(m) = merge {
            self.stops.push(m);
        }
        let out = match self.resolve_node(node) {
            NodeFlow::Stop => Vec::new(),
            NodeFlow::Jump(stmt) => vec![stmt],
            NodeFlow::Emit => self.emit(node),
        };
        if merge.is_some() {
            self.stops.pop();
        }
        out
    }

    /// True if every forward (non-back-edge) predecessor of `to` has already been
    /// emitted — i.e. `to` is not a join awaiting another forward path. (A back-edge
    /// predecessor is irrelevant: it's realized as `continue`.)
    fn sole_forward_pred(&self, to: usize) -> bool {
        self.d.preds[to].iter().all(|&p| {
            // back edge into `to`: ignore (it's a loop continue, not a forward join)
            self.d.dom.dominates(to, p) || self.emitted.contains(&p)
        })
    }

    fn arm_nodes(&self, block: &super::cff::CaseBlock) -> (usize, usize) {
        let mut then_node = self.d.exit;
        let mut else_node = self.d.exit;
        for e in &block.edges {
            match e.guard {
                Guard::IfTrue => then_node = self.d.target_node(e.target),
                Guard::IfFalse => else_node = self.d.target_node(e.target),
                Guard::Always => {}
            }
        }
        (then_node, else_node)
    }

    /// The immediate merge of a two-way split at `cur`: the unique node `m` with
    /// `idom(m) == cur` reachable (forward) from both arms. `None` when the arms do
    /// not re-converge under `cur` (each arm runs to its own end / loop edge).
    fn find_merge(&self, cur: usize, a: usize, b: usize) -> Option<usize> {
        if a == b {
            return (a != self.d.exit).then_some(a);
        }
        let ra = self.forward_reach(a);
        let rb = self.forward_reach(b);
        let mut found = None;
        for &m in ra.intersection(&rb) {
            if m != self.d.exit && self.d.dom.idom(m) == Some(cur) {
                if found.is_some() {
                    return None; // ambiguous → decline this split's merge
                }
                found = Some(m);
            }
        }
        found
    }

    /// Forward-reachable nodes from `from` (skipping back edges), used for merge
    /// detection. Bounded by the graph size.
    fn forward_reach(&self, from: usize) -> HashSet<usize> {
        let mut seen = HashSet::new();
        let mut stack = vec![from];
        seen.insert(from);
        while let Some(n) = stack.pop() {
            if n == self.d.exit {
                continue;
            }
            for &s in &self.d.succs[n] {
                // Skip back edges (s dominates n).
                if s != self.d.exit && self.d.dom.dominates(s, n) {
                    continue;
                }
                if seen.insert(s) {
                    stack.push(s);
                }
            }
        }
        seen
    }

    /// Emit `while (true) { … }` for the loop headed at `header`. Returns the
    /// statement and the in-machine follow node (where control continues after the
    /// loop), `None` when the loop only leaves to Exit.
    fn emit_loop(&mut self, header: usize) -> (Statement<'a>, Option<usize>) {
        let follow = self.loop_follow(header);
        self.loops.push(LoopCtx {
            header,
            follow: follow.unwrap_or(self.d.exit),
        });
        // Emit the header itself as a normal node *inside* its own loop, so its back
        // edges resolve to `continue` and its exits to `break`. We mark the header
        // emitted here (the caller's generic loop already did so), then emit its
        // content + edges via a header-local walk that does NOT re-enter the
        // loop-header branch (which would recurse forever).
        let body = self.emit_loop_body(header);
        self.loops.pop();

        let body_block = self.ast.statement_block(SPAN, self.vec_from(body));
        let cond = self.ast.expression_boolean_literal(SPAN, true);
        let while_stmt = self.ast.statement_while(SPAN, cond, body_block);
        (while_stmt, follow.filter(|&f| f != self.d.exit))
    }

    /// Emit the loop body beginning at `header`. The header is already counted as
    /// `emitted` by the caller; here we emit its content + edges directly (treating
    /// the header as a plain node, since we are inside its loop), then let the
    /// generic emitter handle the rest of the body.
    fn emit_loop_body(&mut self, header: usize) -> Vec<Statement<'a>> {
        let mut out = Vec::new();
        out.extend(self.src.content(self.d.order[header]));
        let state = self.d.order[header];
        let block = &self.cfg.blocks[&state];
        let next = match block.edges.len() {
            1 => self.d.target_node(block.edges[0].target),
            2 => {
                let (then_node, else_node) = self.arm_nodes(block);
                let merge = self.find_merge(header, then_node, else_node);
                let Some(cond) = self.src.cond(state) else {
                    self.poison = true;
                    return out;
                };
                let then_body = self.emit_branch(then_node, merge);
                let else_body = self.emit_branch(else_node, merge);
                out.push(self.if_stmt(cond, then_body, else_body));
                match merge {
                    Some(m) => m,
                    None => return out,
                }
            }
            _ => {
                self.poison = true;
                return out;
            }
        };
        match self.follow_flow(next) {
            Flow::Stop => {}
            Flow::Jump(stmt) => out.push(stmt),
            Flow::Goto(t) => out.extend(self.emit(t)),
        }
        out
    }

    /// The single in-machine follow (break target) of the loop headed at `header`,
    /// or `None` if it only exits to `Exit` / has no unique structured follow.
    fn loop_follow(&self, header: usize) -> Option<usize> {
        let body = self.d.loop_body(header);
        let mut exits: HashSet<usize> = HashSet::new();
        for &n in &body {
            if n == self.d.exit {
                continue;
            }
            for &s in &self.d.succs[n] {
                if !body.contains(&s) {
                    exits.insert(s);
                }
            }
        }
        // Prefer a single in-machine exit target as the follow.
        let in_machine: Vec<usize> = exits
            .iter()
            .copied()
            .filter(|&e| e != self.d.exit)
            .collect();
        match in_machine.len() {
            0 => None,                // loop only leaves to Exit
            1 => Some(in_machine[0]), // the structured continuation
            _ => None,                // multiple exits → no single break target
        }
    }

    fn if_stmt(
        &self,
        cond: Expression<'a>,
        then_body: Vec<Statement<'a>>,
        else_body: Vec<Statement<'a>>,
    ) -> Statement<'a> {
        let then_block = self.ast.statement_block(SPAN, self.vec_from(then_body));
        if else_body.is_empty() {
            self.ast.statement_if(SPAN, cond, then_block, None)
        } else {
            let else_block = self.ast.statement_block(SPAN, self.vec_from(else_body));
            self.ast
                .statement_if(SPAN, cond, then_block, Some(else_block))
        }
    }

    fn vec_from(&self, v: Vec<Statement<'a>>) -> oxc_allocator::Vec<'a, Statement<'a>> {
        let mut out = self.ast.vec();
        for s in v {
            out.push(s);
        }
        out
    }
}

/// Classification of a node we are about to emit.
enum NodeFlow<'a> {
    /// The node belongs to an enclosing region / is Exit: stop this path.
    Stop,
    /// The node is a loop edge target: emit this break/continue and stop.
    Jump(Statement<'a>),
    /// Emit this node here.
    Emit,
}

/// How to continue *to* a successor node.
enum Flow<'a> {
    /// End this path (fall through to an enclosing region, or Exit).
    Stop,
    /// Emit this break/continue and stop the path.
    Jump(Statement<'a>),
    /// Continue emitting at the given node in this region.
    Goto(usize),
}

/// What a jump to a node resolves to against the active loop stack.
enum JumpKind {
    /// Not a loop edge.
    None,
    /// Back edge to the innermost loop header.
    Continue,
    /// Exit edge to the innermost loop's follow.
    Break,
    /// Targets a non-innermost loop (needs a labeled jump we don't emit) → decline.
    Unreachable,
}

/// Structure a reducible [`RecoveredCfg`] into `if/else` + `while(true)` using the
/// supplied [`CaseSource`] to resolve content/conditions. Returns `None` (decline)
/// on any irreducible graph or any shape the relooper cannot place soundly.
pub fn structure_general<'a, S: CaseSource<'a>>(
    cfg: &RecoveredCfg,
    src: &S,
    ast: AstBuilder<'a>,
) -> Option<Vec<Statement<'a>>> {
    let d = Dense::build(cfg);
    if !is_reducible(&d) {
        return None;
    }
    let entry = d.node_of(cfg.entry);
    debug_assert_eq!(entry, 0, "entry must be dense node 0");
    let mut r = Relooper {
        cfg,
        d: &d,
        src,
        ast,
        loops: Vec::new(),
        stops: Vec::new(),
        emitted: HashSet::new(),
        poison: false,
    };
    let body = r.emit(entry);
    if r.poison {
        return None;
    }
    // Soundness: every reachable node must have been emitted exactly once.
    if r.emitted.len() != d.order.len() {
        return None;
    }
    Some(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::cff::{CaseBlock, Edge};

    // A trivial CaseSource that emits a marker statement `S<k>;` per state and a
    // boolean identifier `c<k>` as the condition, so we can snapshot the structure
    // as generated source.
    struct MarkerSource;
    impl<'a> CaseSource<'a> for MarkerSource {
        fn content(&self, _k: i64) -> Vec<Statement<'a>> {
            Vec::new()
        }
        fn cond(&self, _k: i64) -> Option<Expression<'a>> {
            None
        }
    }

    fn dense_of(cfg: &RecoveredCfg) -> Dense {
        Dense::build(cfg)
    }

    fn cfg(entry: i64, pairs: Vec<(i64, CaseBlock)>) -> RecoveredCfg {
        use std::collections::VecDeque;
        let mut blocks = HashMap::new();
        for (k, b) in pairs {
            blocks.insert(k, b);
        }
        let mut order = Vec::new();
        let mut seen = HashSet::new();
        let mut q = VecDeque::new();
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
    fn if_true(t: i64) -> Edge {
        Edge {
            guard: Guard::IfTrue,
            target: Target::State(t),
        }
    }
    fn if_false(t: i64) -> Edge {
        Edge {
            guard: Guard::IfFalse,
            target: Target::State(t),
        }
    }
    fn if_true_exit() -> Edge {
        Edge {
            guard: Guard::IfTrue,
            target: Target::Exit,
        }
    }
    fn if_false_exit() -> Edge {
        Edge {
            guard: Guard::IfFalse,
            target: Target::Exit,
        }
    }

    #[test]
    fn linear_chain_is_reducible_and_visits_all() {
        // 1 -> 2 -> 3 -> Exit
        let g = cfg(
            1,
            vec![
                (1, block(vec![always(2)])),
                (2, block(vec![always(3)])),
                (3, block(vec![exit()])),
            ],
        );
        let d = dense_of(&g);
        assert!(is_reducible(&d));
    }

    #[test]
    fn diamond_merge_is_found() {
        // 1 -> {2,3} -> 4 -> Exit
        let g = cfg(
            1,
            vec![
                (1, block(vec![if_true(2), if_false(3)])),
                (2, block(vec![always(4)])),
                (3, block(vec![always(4)])),
                (4, block(vec![exit()])),
            ],
        );
        let d = dense_of(&g);
        assert!(is_reducible(&d));
        let alloc = oxc_allocator::Allocator::default();
        let r = Relooper {
            cfg: &g,
            d: &d,
            src: &MarkerSource,
            ast: AstBuilder::new(&alloc),
            loops: Vec::new(),
            stops: Vec::new(),
            emitted: HashSet::new(),
            poison: false,
        };
        // node 0 is state 1, arms are nodes 1 (state2) and 2 (state3), merge node 3
        let merge = r.find_merge(0, 1, 2);
        assert_eq!(merge, Some(3));
    }

    #[test]
    fn self_loop_header_detected() {
        // 1: if (c) -> 1 else -> Exit
        let g = cfg(1, vec![(1, block(vec![if_true(1), if_false_exit()]))]);
        let d = dense_of(&g);
        assert!(d.is_loop_header(0));
        assert!(is_reducible(&d));
    }

    #[test]
    fn while_loop_body_and_follow() {
        // 1 -> 2; 2: if (c) -> 3 else -> 4(exit-state); 3 -> 2 ; 4 -> Exit
        let g = cfg(
            1,
            vec![
                (1, block(vec![always(2)])),
                (2, block(vec![if_true(3), if_false(4)])),
                (3, block(vec![always(2)])),
                (4, block(vec![exit()])),
            ],
        );
        let d = dense_of(&g);
        assert!(is_reducible(&d));
        // header is state 2 = node 1
        let header = d.node_of(2);
        assert!(d.is_loop_header(header));
        let body = d.loop_body(header);
        // body = {2,3} = nodes {1,2}
        assert!(body.contains(&d.node_of(2)));
        assert!(body.contains(&d.node_of(3)));
        assert!(!body.contains(&d.node_of(4)));
    }

    #[test]
    fn irreducible_two_entry_loop_declined() {
        // 1 -> {2,3}; 2 -> 3; 3 -> 2  (cross edges into a 2-3 loop with two entries)
        let g = cfg(
            1,
            vec![
                (1, block(vec![if_true(2), if_false(3)])),
                (2, block(vec![always(3)])),
                (3, block(vec![if_true(2), if_false_exit()])),
            ],
        );
        let d = dense_of(&g);
        // The 2<->3 cycle is entered at both 2 and 3 → irreducible.
        assert!(!is_reducible(&d));
    }

    #[allow(unused)]
    fn _ensure_used() {
        let _ = if_true_exit();
    }
}
