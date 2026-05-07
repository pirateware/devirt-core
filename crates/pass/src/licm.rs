//! Loop-invariant code motion — hoist pure instructions whose
//! operands are all defined outside a loop body up into the loop's
//! pre-header.
//!
//! ## Scope (v1: PURE-only)
//!
//! Only instructions whose effect is exactly `Effect::PURE` are
//! hoisted. `MAY_THROW`-bearing insts (generic arithmetic, coercions,
//! `Throws`-tier loads, etc.) are deliberately out of scope until
//! issue #34 lands per-instruction exception edges. Without those,
//! moving a possibly-throwing inst from inside a loop body to the
//! pre-header changes exception-occurrence semantics (a loop that
//! never executes its body must not surface the throw). #34's per-
//! site edges give us the soundness gate to expand LICM's effect set
//! later.
//!
//! Heap reads (`READS_HEAP`) are also excluded. A pure read still
//! needs alias analysis to prove no intervening loop-body write
//! shadows the load — that's `ForwardLoadsPass`'s job. LICM v1 keeps
//! the rule trivially safe: `effect == PURE` and nothing else.
//!
//! ## Algorithm
//!
//! 1. Build dominators by iterative dataflow over reverse-postorder.
//! 2. For each block `B` with a successor `H` such that `H` dominates
//!    `B`, mark `(B → H)` as a back-edge and `H` as a loop header.
//! 3. For each loop header `H`, the natural-loop body is the set of
//!    blocks that both (a) are dominated by `H` and (b) can reach a
//!    latch (back-edge source). We compute it by reverse-BFS from the
//!    latches, stopping at `H`.
//! 4. The pre-header is the unique non-back-edge predecessor of `H`.
//!    If `H` has zero or more than one such predecessor, we skip the
//!    loop (no edge-splitting in v1).
//! 5. Walk the loop body in body-block order. An instruction is hoist
//!    eligible iff:
//!    - Its effect is exactly `Effect::PURE`.
//!    - Every `Operand::Value` operand is defined OUTSIDE the loop
//!      body (definitions in the pre-header or any earlier dominator
//!      are fine; arg / capture / const operands are trivially
//!      outside). A definition in any block of the loop body —
//!      including the header itself — is loop-defined.
//!    - The result Value is not `captured_by_passthrough` (those are
//!      pinned by the passthrough renamer; reordering them across
//!      block boundaries would risk the source name not flowing where
//!      the renamer expects).
//! 6. Move the inst from its source block's instruction list to the
//!    pre-header's instruction list, just before the pre-header's
//!    terminator. Operand references survive the move untouched —
//!    Value indices are stable, and `ValueDefKind::Inst(iid)` doesn't
//!    record a containing block.
//!
//! Iterate within a single pass run until no further moves are found:
//! a hoisted inst becomes an outside-loop definition, which can
//! unblock further hoists in the same loop.
//!
//! ## What this is not
//!
//! - Not edge-splitting. If the header has multiple non-back-edge
//!   preds, we leave the loop alone. A separate pre-header-creation
//!   ticket can land later.
//! - Not partial-loop-invariant code motion (PRE).
//! - Not nested-loop-aware in any sophisticated sense; a loop-body
//!   inst whose operands are all defined outside ITS loop body is
//!   hoisted to ITS pre-header. An outer loop's LICM run on the next
//!   iteration of the pass can hoist the result further if it is
//!   invariant w.r.t. the outer loop.

use std::collections::HashSet;

use ir::error::Error;
use ir::ssa::{
    BlockId, Effect, FuncId, InstId, LoweredBody, Module, Operand, Value, ValueDefKind,
    walk_inst_operands,
};

use crate::FunctionPass;

pub struct LicmPass;

impl FunctionPass for LicmPass {
    fn name(&self) -> &'static str {
        "licm"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let pinned: HashSet<u32> =
            func.captured_by_passthrough.iter().copied().collect();
        let body = func.lowered_mut();
        let n = body.blocks.len();
        if n == 0 {
            return Ok(false);
        }

        // Compute dominators once. Loops/back-edges are derived from
        // them; the loop bodies and pre-headers don't change as we
        // hoist (we only move instructions, not blocks or edges), so
        // a single dominator computation is sufficient for the whole
        // pass run.
        let dom = compute_dominators(body);
        let loops = find_loops(body, &dom);

        // Hoist insts inside a fixed-point loop: each round, walk every
        // (loop, loop_body, pre_header) triple and move any
        // freshly-eligible inst. Bounded by the total number of insts
        // in the function — every iteration that returns `changed`
        // moves at least one inst.
        let max_rounds = body.instructions.len().saturating_add(1);
        let mut total_changed = false;
        for _ in 0..max_rounds {
            let mut round_changed = false;
            for lp in &loops {
                round_changed |= hoist_one_loop(body, lp, &pinned);
            }
            if !round_changed {
                break;
            }
            total_changed = true;
        }

        Ok(total_changed)
    }
}

// ============================================================================
// Dominators
// ============================================================================

/// Iterative dominator computation. Returns `dom[i] = Some(idom)` for
/// every reachable block; `None` for unreachable blocks (which we
/// then treat as "not dominated by anyone for LICM's purposes" and
/// effectively skip — no loop in unreachable code is interesting).
///
/// The algorithm is the Cooper/Harvey/Kennedy iterative idom one,
/// simplified: we keep a `dom_set` per block (set of all dominators)
/// and intersect over predecessors until fixed point. O(n^2) memory in
/// the worst case but corpus-scale functions are tiny; the simplicity
/// is worth the constant.
fn compute_dominators(body: &LoweredBody) -> Vec<Option<HashSet<u32>>> {
    let n = body.blocks.len();
    let entry = body.entry.0 as usize;
    let mut dom: Vec<Option<HashSet<u32>>> = vec![None; n];
    if entry >= n {
        return dom;
    }
    // Reachability sweep — only reachable blocks get a `dom_set`.
    let mut reachable = vec![false; n];
    let mut stack = vec![entry];
    reachable[entry] = true;
    while let Some(bi) = stack.pop() {
        for (s, _) in body.blocks[bi].terminator.successors() {
            let si = s.0 as usize;
            if si < n && !reachable[si] {
                reachable[si] = true;
                stack.push(si);
            }
        }
    }
    // Initialize: entry dominates itself; every other reachable block
    // gets the full set (which monotonically shrinks under
    // intersection).
    let full: HashSet<u32> = (0..n).filter(|i| reachable[*i]).map(|i| i as u32).collect();
    for i in 0..n {
        if !reachable[i] {
            continue;
        }
        if i == entry {
            let mut s = HashSet::new();
            s.insert(entry as u32);
            dom[i] = Some(s);
        } else {
            dom[i] = Some(full.clone());
        }
    }

    let mut changed = true;
    let mut iters = 0usize;
    let max_iters = (n.saturating_mul(n)).saturating_add(64);
    while changed {
        iters += 1;
        if iters > max_iters {
            // Defensive: should not occur for monotone intersection on
            // a finite lattice.
            break;
        }
        changed = false;
        for bi in 0..n {
            if !reachable[bi] || bi == entry {
                continue;
            }
            // Intersect dom-sets of all reachable predecessors.
            let preds = &body.blocks[bi].predecessors;
            let mut new_set: Option<HashSet<u32>> = None;
            for &p in preds {
                let pi = p.0 as usize;
                if pi >= n || !reachable[pi] {
                    continue;
                }
                let pset = match &dom[pi] {
                    Some(s) => s,
                    None => continue,
                };
                new_set = Some(match new_set {
                    None => pset.clone(),
                    Some(acc) => acc.intersection(pset).copied().collect(),
                });
            }
            let mut new_set = new_set.unwrap_or_default();
            new_set.insert(bi as u32);
            if dom[bi].as_ref() != Some(&new_set) {
                dom[bi] = Some(new_set);
                changed = true;
            }
        }
    }
    dom
}

fn dominates(dom: &[Option<HashSet<u32>>], a: BlockId, b: BlockId) -> bool {
    // `a` dominates `b` iff `a` is in `b`'s dom set.
    match dom.get(b.0 as usize).and_then(|s| s.as_ref()) {
        Some(s) => s.contains(&a.0),
        None => false,
    }
}

// ============================================================================
// Loop discovery
// ============================================================================

struct Loop {
    header: BlockId,
    pre_header: BlockId,
    /// Blocks that belong to the loop body, INCLUDING the header
    /// itself. Stored as a HashSet for O(1) membership checks during
    /// the hoist walk.
    body: HashSet<u32>,
    /// Body in stable block-order — used to walk insts deterministically.
    body_order: Vec<BlockId>,
}

fn find_loops(body: &LoweredBody, dom: &[Option<HashSet<u32>>]) -> Vec<Loop> {
    let n = body.blocks.len();
    // Group back-edges by header.
    let mut header_latches: std::collections::HashMap<u32, Vec<BlockId>> =
        std::collections::HashMap::new();
    for bi in 0..n {
        let from = BlockId(bi as u32);
        for (succ, _) in body.blocks[bi].terminator.successors() {
            // Back-edge: target dominates source.
            if dominates(dom, succ, from) {
                header_latches.entry(succ.0).or_default().push(from);
            }
        }
    }

    let mut loops: Vec<Loop> = Vec::new();
    for (h_idx, latches) in header_latches {
        let header = BlockId(h_idx);
        // Collect natural-loop body: reverse-BFS from latches,
        // bounded by the header.
        let mut body_set: HashSet<u32> = HashSet::new();
        body_set.insert(h_idx);
        let mut work: Vec<BlockId> = latches.clone();
        for &l in &latches {
            body_set.insert(l.0);
        }
        while let Some(b) = work.pop() {
            if b == header {
                continue;
            }
            for &p in &body.blocks[b.0 as usize].predecessors {
                if body_set.insert(p.0) {
                    work.push(p);
                }
            }
        }

        // Pre-header detection: the unique header-predecessor that is
        // NOT in the loop body. Multiple non-body preds → no
        // pre-header in v1, skip the loop.
        let mut outside_preds: Vec<BlockId> = Vec::new();
        for &p in &body.blocks[h_idx as usize].predecessors {
            if !body_set.contains(&p.0) {
                outside_preds.push(p);
            }
        }
        if outside_preds.len() != 1 {
            continue;
        }
        let pre_header = outside_preds[0];

        // Stable body order — ascending block index keeps the hoist
        // walk deterministic across runs.
        let mut body_order: Vec<BlockId> =
            body_set.iter().map(|i| BlockId(*i)).collect();
        body_order.sort_by_key(|b| b.0);

        loops.push(Loop {
            header,
            pre_header,
            body: body_set,
            body_order,
        });
    }
    // Sort loops by header index for determinism.
    loops.sort_by_key(|l| l.header.0);
    loops
}

// ============================================================================
// Hoisting
// ============================================================================

/// One sweep over `lp.body` looking for hoist-eligible insts; moves
/// them into `lp.pre_header` (just before its terminator). Returns
/// `true` iff anything moved.
fn hoist_one_loop(body: &mut LoweredBody, lp: &Loop, pinned: &HashSet<u32>) -> bool {
    let mut changed = false;
    for &b in &lp.body_order {
        // Snapshot inst ids — we'll be modifying the source block's
        // instructions vector.
        let iids: Vec<InstId> = body.blocks[b.0 as usize].instructions.clone();
        // Track which indices to drop from the source block.
        let mut to_drop: Vec<InstId> = Vec::new();
        for iid in iids {
            if !is_hoist_eligible(body, iid, lp, pinned) {
                continue;
            }
            // Move: remove from source block (collected at the end),
            // append to pre-header before its terminator.
            body.blocks[lp.pre_header.0 as usize]
                .instructions
                .push(iid);
            to_drop.push(iid);
            changed = true;
        }
        if !to_drop.is_empty() {
            let drop_set: HashSet<u32> = to_drop.iter().map(|i| i.0).collect();
            body.blocks[b.0 as usize]
                .instructions
                .retain(|i| !drop_set.contains(&i.0));
        }
    }
    changed
}

fn is_hoist_eligible(
    body: &LoweredBody,
    iid: InstId,
    lp: &Loop,
    pinned: &HashSet<u32>,
) -> bool {
    let inst = &body.instructions[iid.0 as usize];
    // PURE-only. No MAY_THROW, no READS_HEAP, no WRITES_HEAP, etc.
    if inst.kind.effect() != Effect::PURE {
        return false;
    }
    // Pinned by the passthrough renamer — leave in place.
    let v_owner = find_value_for_inst_opt(body, iid);
    if let Some(v) = v_owner
        && pinned.contains(&v.0)
    {
        return false;
    }
    // All Value operands must be defined outside the loop body.
    let mut all_invariant = true;
    walk_inst_operands(&inst.kind, |op| {
        if let Operand::Value(v) = op
            && value_defined_in_loop(body, *v, lp)
        {
            all_invariant = false;
        }
    });
    all_invariant
}

/// True iff `v`'s defining instruction (or block-param block) sits
/// inside the loop body. Args / captures / `Removed` slots return
/// false (they are loop-invariant by construction — the function
/// signature doesn't change between iterations).
fn value_defined_in_loop(body: &LoweredBody, v: Value, lp: &Loop) -> bool {
    let def = match body.values.get(v.0 as usize) {
        Some(d) => d,
        None => return false,
    };
    match &def.kind {
        ValueDefKind::Inst(iid) => {
            // Find the block that currently lists this inst.
            for bi in lp.body.iter().copied() {
                let blk = &body.blocks[bi as usize];
                if blk.instructions.iter().any(|i| i.0 == iid.0) {
                    return true;
                }
            }
            false
        }
        ValueDefKind::BlockParam { block, .. } => lp.body.contains(&block.0),
        ValueDefKind::Arg(_) | ValueDefKind::Removed => false,
    }
}

/// `find_value_for_inst` panics if the inst has no owning Value
/// (i.e. void instructions like StoreProp). For LICM we already gate
/// on PURE effect — none of those are void today — but keep the
/// option-returning variant local to be defensive against future
/// PURE void insts.
fn find_value_for_inst_opt(body: &LoweredBody, iid: InstId) -> Option<Value> {
    for (i, def) in body.values.iter().enumerate() {
        if let ValueDefKind::Inst(j) = &def.kind
            && j.0 == iid.0
        {
            return Some(Value(i as u32));
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        BlockId, Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity,
        Terminator,
    };

    /// Build a simple single-basic-block loop:
    ///
    ///   entry (pre-header) -> header
    ///   header: <insts...>; cbr cond, header, exit
    ///   exit:   ret
    ///
    /// `build_body` populates the header with PURE arithmetic; the
    /// caller chooses the cond.
    fn build_simple_loop<F>(
        num_args: u32,
        build_body: F,
    ) -> (Module, FuncId, BlockId, BlockId, BlockId)
    where
        F: FnOnce(&mut FunctionBuilder<'_>, BlockId),
    {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", num_args);
        let entry = b.new_block();
        let header = b.new_block();
        let exit = b.new_block();

        b.switch_to_block(entry);
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![],
        });

        b.switch_to_block(header);
        build_body(&mut b, header);
        let cond = if num_args > 0 {
            Operand::Value(b.body().params[0])
        } else {
            Operand::Const(Constant::Bool(true))
        };
        b.set_terminator(Terminator::CondBranch {
            cond,
            then_b: header,
            then_args: vec![],
            else_b: exit,
            else_args: vec![],
        });

        b.switch_to_block(exit);
        b.set_terminator(Terminator::Return { val: None });

        let id = b.finish();
        module.functions[id.0 as usize].lowered_mut().entry = entry;
        module.functions[id.0 as usize]
            .lowered_mut()
            .rebuild_predecessors();
        (module, id, entry, header, exit)
    }

    fn block_inst_kinds<'a>(
        module: &'a Module,
        fid: FuncId,
        block: BlockId,
    ) -> Vec<&'a InstKind> {
        let body = module.functions[fid.0 as usize].lowered();
        body.blocks[block.0 as usize]
            .instructions
            .iter()
            .map(|iid| &body.instructions[iid.0 as usize].kind)
            .collect()
    }

    /// `for (...) { return a + b; }` — the AddInt32 of two args is
    /// loop-invariant and PURE; should hoist into the entry block.
    #[test]
    fn hoists_pure_add_out_of_loop() {
        let (mut module, fid, entry, header, _exit) =
            build_simple_loop(2, |b, _header| {
                let a = b.body().params[0];
                let bb = b.body().params[1];
                b.add_inst(InstKind::AddInt32 {
                    lhs: Operand::Value(a),
                    rhs: Operand::Value(bb),
                });
            });

        // Sanity: AddInt32 starts in header.
        assert!(matches!(
            block_inst_kinds(&module, fid, header).as_slice(),
            [InstKind::AddInt32 { .. }]
        ));

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(LicmPass));
        pm.run(&mut module, fid).unwrap();

        // After: AddInt32 must have moved to entry; header is empty.
        assert!(matches!(
            block_inst_kinds(&module, fid, entry).as_slice(),
            [InstKind::AddInt32 { .. }]
        ));
        assert!(block_inst_kinds(&module, fid, header).is_empty());
        // Validation must still pass.
        module.validate().expect("post-LICM IR must validate");
    }

    /// An inst whose operand is loop-defined must NOT be hoisted —
    /// even if itself is PURE.
    #[test]
    fn does_not_hoist_when_operand_is_loop_defined() {
        let (mut module, fid, _entry, header, _exit) =
            build_simple_loop(1, |b, _header| {
                let a = b.body().params[0];
                // First add: invariant (arg+arg) — will hoist.
                let inv = b
                    .add_inst(InstKind::AddInt32 {
                        lhs: Operand::Value(a),
                        rhs: Operand::Value(a),
                    })
                    .unwrap();
                // Second add: depends on the loop-defined `inv`'s
                // post-hoist position. Once `inv` hoists, this becomes
                // invariant too — and the fixed-point inner loop will
                // hoist it on the next round.
                //
                // To test the "operand IS loop-defined" branch, we
                // need a value that stays loop-defined regardless.
                // A block-param defined by the header itself is the
                // canonical example, but our simple builder doesn't
                // add header params. Instead we use an instruction
                // that is itself NOT hoist-eligible: a heap load.
                let key = b.intern("x");
                let alloc = b.add_inst(InstKind::AllocObject).unwrap();
                // The alloc is PURE; will hoist.
                let load = b
                    .add_inst(InstKind::LoadProp {
                        obj: Operand::Value(alloc),
                        key,
                        purity: Purity::Throws,
                    })
                    .unwrap();
                // The load (Throws) is NOT eligible → stays in
                // header. An inst depending on it (PURE not) must
                // also stay.
                b.add_inst(InstKind::AddInt32 {
                    lhs: Operand::Value(load),
                    rhs: Operand::Value(inv),
                });
            });

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(LicmPass));
        pm.run(&mut module, fid).unwrap();

        // Header must still contain BOTH the LoadProp (ineligible:
        // not PURE) AND the second AddInt32 (operand is loop-defined).
        let header_kinds = block_inst_kinds(&module, fid, header);
        let has_load = header_kinds
            .iter()
            .any(|k| matches!(k, InstKind::LoadProp { .. }));
        let has_dep_add = header_kinds.iter().any(|k| {
            matches!(
                k,
                InstKind::AddInt32 {
                    lhs: Operand::Value(_),
                    rhs: Operand::Value(_),
                }
            )
        });
        assert!(has_load, "Throws LoadProp must not be hoisted");
        assert!(
            has_dep_add,
            "AddInt32 depending on loop-defined LoadProp must not be hoisted"
        );
        module.validate().expect("post-LICM IR must validate");
    }

    /// MAY_THROW inst with all-invariant operands — must NOT hoist
    /// (PURE-only restriction; see #34).
    #[test]
    fn does_not_hoist_may_throw_even_with_invariant_operands() {
        let (mut module, fid, _entry, header, _exit) =
            build_simple_loop(2, |b, _header| {
                let a = b.body().params[0];
                let bb = b.body().params[1];
                // AddGeneric is MAY_THROW; operands are args
                // (definitively outside loop body). Must stay.
                b.add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(a),
                    rhs: Operand::Value(bb),
                });
            });

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(LicmPass));
        pm.run(&mut module, fid).unwrap();

        let header_kinds = block_inst_kinds(&module, fid, header);
        assert!(
            header_kinds
                .iter()
                .any(|k| matches!(k, InstKind::AddGeneric { .. })),
            "AddGeneric (MAY_THROW) must stay in the loop until #34"
        );
        module.validate().expect("post-LICM IR must validate");
    }

    /// Nested loops: an inst invariant w.r.t. the inner loop hoists
    /// to the inner pre-header, then on a subsequent round becomes
    /// hoistable w.r.t. the outer loop.
    ///
    /// CFG:
    ///   entry -> outer_h
    ///   outer_h: cbr cond, inner_h, outer_exit
    ///   inner_h: <add a,a>; cbr cond, inner_h, outer_h
    ///   outer_exit: ret
    ///
    /// outer pre-header = entry; inner pre-header = outer_h.
    /// The AddInt32 of args is invariant w.r.t. BOTH loops. After
    /// the first round it lands in outer_h (inner pre-header); after
    /// the second round it lands in entry (outer pre-header).
    #[test]
    fn nested_loops_hoist_to_outermost_pre_header() {
        let mut module = Module::new();
        let fid = {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let entry = b.new_block();
            let outer_h = b.new_block();
            let inner_h = b.new_block();
            let outer_exit = b.new_block();

            let a = b.body().params[0];

            b.switch_to_block(entry);
            b.set_terminator(Terminator::Branch {
                target: outer_h,
                args: vec![],
            });

            b.switch_to_block(outer_h);
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(a),
                then_b: inner_h,
                then_args: vec![],
                else_b: outer_exit,
                else_args: vec![],
            });

            b.switch_to_block(inner_h);
            b.add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(a),
                rhs: Operand::Value(a),
            });
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(a),
                then_b: inner_h,
                then_args: vec![],
                else_b: outer_h,
                else_args: vec![],
            });

            b.switch_to_block(outer_exit);
            b.set_terminator(Terminator::Return { val: None });

            let id = b.finish();
            module.functions[id.0 as usize].lowered_mut().entry = entry;
            module.functions[id.0 as usize]
                .lowered_mut()
                .rebuild_predecessors();
            id
        };

        // Locate blocks by BlockId(0..3) — `new_block` allocates in
        // the order entry, outer_h, inner_h, outer_exit.
        let entry = BlockId(0);
        let inner_h = BlockId(2);

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(LicmPass));
        pm.run(&mut module, fid).unwrap();

        // The AddInt32 must have travelled all the way out to entry.
        let entry_has_add = block_inst_kinds(&module, fid, entry)
            .iter()
            .any(|k| matches!(k, InstKind::AddInt32 { .. }));
        let inner_has_add = block_inst_kinds(&module, fid, inner_h)
            .iter()
            .any(|k| matches!(k, InstKind::AddInt32 { .. }));
        assert!(
            entry_has_add,
            "AddInt32 must hoist all the way to entry across nested loops"
        );
        assert!(
            !inner_has_add,
            "AddInt32 must no longer be in the inner loop body"
        );
        module.validate().expect("post-LICM IR must validate");
    }

    /// Termination smoke test: a self-loop with no hoist-eligible
    /// insts must terminate without spinning the inner fixed-point
    /// loop.
    #[test]
    fn no_hang_when_nothing_is_hoistable() {
        let (mut module, fid, _entry, _header, _exit) =
            build_simple_loop(1, |b, _header| {
                let a = b.body().params[0];
                // AddGeneric is MAY_THROW → never eligible.
                b.add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(a),
                    rhs: Operand::Value(a),
                });
            });

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(LicmPass));
        // PassManager's iteration cap (default 64) catches infinite
        // loops loudly. A converging LICM run must not trip it.
        pm.run(&mut module, fid).expect("LICM must terminate");
        module.validate().expect("post-LICM IR must validate");
    }

    /// Loop with multiple non-back-edge predecessors at the header
    /// has no unique pre-header in v1 → LICM skips it cleanly,
    /// instead of crashing or splitting an edge.
    #[test]
    fn loop_with_no_unique_pre_header_is_skipped() {
        let mut module = Module::new();
        let fid = {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let entry = b.new_block();
            let alt = b.new_block();
            let header = b.new_block();
            let exit = b.new_block();

            let a = b.body().params[0];

            b.switch_to_block(entry);
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(a),
                then_b: header,
                then_args: vec![],
                else_b: alt,
                else_args: vec![],
            });

            b.switch_to_block(alt);
            b.set_terminator(Terminator::Branch {
                target: header,
                args: vec![],
            });

            b.switch_to_block(header);
            // Eligible inst — but no pre-header to hoist to.
            b.add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(a),
                rhs: Operand::Value(a),
            });
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(a),
                then_b: header,
                then_args: vec![],
                else_b: exit,
                else_args: vec![],
            });

            b.switch_to_block(exit);
            b.set_terminator(Terminator::Return { val: None });

            let id = b.finish();
            module.functions[id.0 as usize].lowered_mut().entry = entry;
            module.functions[id.0 as usize]
                .lowered_mut()
                .rebuild_predecessors();
            id
        };
        let header = BlockId(2);

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(LicmPass));
        let _ = pm.run(&mut module, fid).unwrap();

        // AddInt32 must remain in the header (no pre-header chosen).
        assert!(
            block_inst_kinds(&module, fid, header)
                .iter()
                .any(|k| matches!(k, InstKind::AddInt32 { .. })),
            "must not hoist when there's no unique pre-header"
        );
        module.validate().expect("post-LICM IR must validate");
    }
}
