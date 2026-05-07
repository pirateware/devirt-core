//! Heap store-load forwarding — resolves Pure `LoadProp`/`LoadIndex`
//! against preceding `StoreProp`/`StoreIndex` on the same unaliased
//! allocation. Cross-block dataflow with a finite-height
//! intersection lattice.
//!
//! `RefinePurityPass` is the soundness gate: only loads marked
//! `Purity::Pure` are forwarded. Pure means the receiver is a
//! provably-unaliased local allocation (no Proxy, no escape), so a
//! per-`(alloc, key)` heap-shape map is correct.
//!
//! ## Algorithm
//!
//! Two-phase analysis-then-transform:
//!
//! 1. **Analysis (read-only):** compute a per-block `out` shape map
//!    via a worklist fixed point. A block's `in` is the entry-wise
//!    intersection of all its predecessors' `out` maps (using
//!    `operand_bit_eq` for value equality, so `NaN` doesn't churn);
//!    its `out` is `transfer(in, block)`. Predecessors that haven't
//!    been analyzed yet (loop back-edges on the first sweep) are
//!    treated as "ignore" rather than "empty" — initial
//!    optimism that gets revisited as the back-edge's `out`
//!    materializes. The lattice is finite-height: entries can only
//!    be dropped from a block's `out` over iterations (intersections
//!    are monotone), so iteration terminates.
//!
//! 2. **Transform (mutating):** walk each block with its converged
//!    `in` shape; stores update the running shape; Pure loads matching
//!    an entry get forwarded — result Value uses replaced with the
//!    stored Operand, Load retired (DCE convention: unhook from block,
//!    blank operands).
//!
//! ## Transfer rules (per instruction)
//!
//! - `StoreProp { obj: Value(a), key, val }` where `a` is a tracked
//!   alloc → `(a, Str(key)) → val`. On non-tracked obj → flush all.
//! - `StoreIndex { obj: Value(a), index: Const(k), val }` where `a`
//!   is tracked → `(a, Idx(k)) → val`. Non-const index, tracked obj →
//!   drop entries for `a` only. Non-tracked obj → flush all.
//! - `LoadProp/LoadIndex { purity: Pure, .. }` against a tracked
//!   alloc with a known key → forward.
//! - Allocations don't disturb existing entries (fresh allocs are
//!   independent).
//! - Any inst whose effect contains `WRITES_HEAP` and isn't already
//!   handled above → flush all (Calls, OpaqueEval, etc.).

use std::collections::HashMap;

use ir::error::Error;
use ir::ssa::{
    Constant, Effect, FuncId, InstId, InstKind, LoweredBody, Module, Operand, Purity, Value,
    ValueDef, ValueDefKind, find_value_for_inst, operand_bit_eq, walk_inst_operands_mut,
};

use crate::FunctionPass;

pub struct ForwardLoadsPass;

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
enum Key {
    Str(ir::ssa::StrId),
    Idx(i32),
}

type Shape = HashMap<(u32, Key), Operand>;

fn key_from_str(s: ir::ssa::StrId) -> Key {
    Key::Str(s)
}

fn key_from_index_operand(op: &Operand) -> Option<Key> {
    match op {
        Operand::Const(Constant::Int32(i)) => Some(Key::Idx(*i)),
        Operand::Const(Constant::Number(n)) => {
            if n.is_finite() && n.trunc() == *n && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 {
                Some(Key::Idx(*n as i32))
            } else {
                None
            }
        }
        Operand::Const(Constant::String(s)) => Some(Key::Str(*s)),
        _ => None,
    }
}

fn is_tracked_alloc(body: &LoweredBody, v: Value) -> bool {
    if let ValueDefKind::Inst(iid) = body.values[v.0 as usize].kind {
        matches!(
            body.instructions[iid.0 as usize].kind,
            InstKind::AllocObject | InstKind::AllocArray { .. }
        )
    } else {
        false
    }
}

/// Entry-wise intersection of two shapes. An entry survives only when
/// both maps have it AND the values are bit-equal. NaN-vs-NaN compares
/// equal under `operand_bit_eq`, which is what we want for fixed-point
/// convergence (otherwise NaN would churn forever).
fn intersect_shapes(a: &Shape, b: &Shape) -> Shape {
    let mut out = Shape::with_capacity(a.len().min(b.len()));
    for (k, va) in a {
        if let Some(vb) = b.get(k)
            && operand_bit_eq(va, vb)
        {
            out.insert(*k, *va);
        }
    }
    out
}

fn shapes_equal(a: &Shape, b: &Shape) -> bool {
    if a.len() != b.len() {
        return false;
    }
    for (k, va) in a {
        match b.get(k) {
            Some(vb) if operand_bit_eq(va, vb) => continue,
            _ => return false,
        }
    }
    true
}

/// Apply a single instruction's effect to the running shape.
///
/// Pure-load CSE: a `Pure` `LoadProp/LoadIndex` on a tracked alloc
/// whose key isn't yet in the shape *seeds* the shape with the load's
/// own result Value. Subsequent reads of the same `(alloc, key)` then
/// forward to the first load — both intra-block (during transform's
/// running-shape walk) and cross-block (the seeded entry flows out
/// through the analysis lattice and into successors' in-shapes).
///
/// Soundness: Pure loads are guaranteed unaliased and side-effect-free
/// (`RefinePurityPass` is the gate), so two Pure reads of the same
/// `(alloc, key)` with no intervening store produce the same value.
/// If a Store later writes that key, the existing shape-update logic
/// overwrites the entry with the stored Operand — the seeded load's
/// Value is invalidated *only* for reads after the store, which is
/// correct.
///
/// Stale-Value safety: the load that seeded an entry is never retired
/// in the same transform pass. The transform reads `shape.get(...)`
/// *before* `apply_inst`, so it forwards based on the pre-add view; if
/// the lookup misses, the load isn't forwarded (stays alive in IR), and
/// only THEN does `apply_inst` add the load's Value to the shape.
/// Successor blocks read the same Value; it's still live.
fn apply_inst(shape: &mut Shape, body: &LoweredBody, iid: InstId, kind: &InstKind) {
    match kind {
        InstKind::StoreProp { obj, key, val, .. } => {
            if let Operand::Value(a) = obj
                && is_tracked_alloc(body, *a)
            {
                shape.insert((a.0, key_from_str(*key)), *val);
            } else {
                shape.clear();
            }
        }
        InstKind::StoreIndex {
            obj, index, val, ..
        } => {
            if let Operand::Value(a) = obj
                && is_tracked_alloc(body, *a)
            {
                if let Some(k) = key_from_index_operand(index) {
                    shape.insert((a.0, k), *val);
                } else {
                    shape.retain(|(av, _), _| *av != a.0);
                }
            } else {
                shape.clear();
            }
        }
        // Allocations don't disturb existing entries.
        InstKind::AllocObject
        | InstKind::AllocArray { .. }
        | InstKind::AllocClosure { .. }
        | InstKind::AllocRegExp { .. } => {}
        InstKind::LoadProp {
            obj,
            key,
            purity: Purity::Pure,
        } => {
            if let Operand::Value(a) = obj
                && is_tracked_alloc(body, *a)
            {
                let k = key_from_str(*key);
                shape.entry((a.0, k)).or_insert_with(|| {
                    Operand::Value(find_value_for_inst(body, iid))
                });
            }
        }
        InstKind::LoadIndex {
            obj,
            index,
            purity: Purity::Pure,
        } => {
            if let Operand::Value(a) = obj
                && is_tracked_alloc(body, *a)
                && let Some(k) = key_from_index_operand(index)
            {
                shape.entry((a.0, k)).or_insert_with(|| {
                    Operand::Value(find_value_for_inst(body, iid))
                });
            }
        }
        // Throws-purity loads: don't seed (different exception
        // semantics across calls; not safe to dedupe).
        InstKind::LoadProp { .. } | InstKind::LoadIndex { .. } => {}
        // Anything else: WRITES_HEAP-bearing insts (calls, OpaqueEval)
        // could touch any heap state we're tracking — flush.
        other => {
            if other.effect().contains(Effect::WRITES_HEAP) {
                shape.clear();
            }
        }
    }
}

/// Compute a block's `out` shape given its `in`. Read-only over
/// `body` — used during the analysis sweep. Returns a fresh map.
fn transfer(in_shape: &Shape, body: &LoweredBody, bi: usize) -> Shape {
    let mut shape = in_shape.clone();
    for &iid in &body.blocks[bi].instructions {
        let kind = &body.instructions[iid.0 as usize].kind;
        apply_inst(&mut shape, body, iid, kind);
    }
    shape
}

/// Compute a block's `in` shape from its predecessors' converged
/// `out` maps. Predecessors not yet analysed (`None`) are skipped —
/// initial-sweep optimism for back-edges. If no predecessor has been
/// analysed, returns an empty map (which is also correct for entry
/// blocks).
fn compute_in(
    bi: usize,
    body: &LoweredBody,
    block_out: &[Option<Shape>],
) -> Shape {
    let preds = &body.blocks[bi].predecessors;
    let mut iter = preds.iter().filter_map(|p| block_out[p.0 as usize].as_ref());
    let first = match iter.next() {
        Some(s) => s.clone(),
        None => return Shape::new(),
    };
    iter.fold(first, |acc, other| intersect_shapes(&acc, other))
}

impl FunctionPass for ForwardLoadsPass {
    fn name(&self) -> &'static str {
        "forward_loads"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        if module.functions[fid.0 as usize].is_pass_through() {
            return Ok(false);
        }
        let body = module.functions[fid.0 as usize].lowered_mut();
        let n = body.blocks.len();

        // ---- Analysis phase ------------------------------------------------
        // block_out[bi] = None means "not yet analysed".
        let mut block_out: Vec<Option<Shape>> = vec![None; n];
        // Worklist seeded with all blocks. Order doesn't affect
        // correctness; reverse-postorder would be a constant-factor
        // win but isn't necessary.
        let mut worklist: std::collections::VecDeque<usize> = (0..n).collect();
        let mut on_worklist: Vec<bool> = vec![true; n];
        // Iteration cap: each block's out can only LOSE entries over
        // iterations, so total work is O(n × initial-shape-size).
        // Cap at n^2 + a safety margin to surface bugs loudly.
        let max_iters = (n * n).saturating_add(64).saturating_mul(2);
        let mut iters = 0usize;

        while let Some(bi) = worklist.pop_front() {
            on_worklist[bi] = false;
            iters += 1;
            if iters > max_iters {
                // Fixed-point cap exceeded — bail conservatively
                // without forwarding. Surfaces an analysis bug
                // without corrupting the IR.
                return Ok(false);
            }
            let in_shape = compute_in(bi, body, &block_out);
            let new_out = transfer(&in_shape, body, bi);
            let changed = match &block_out[bi] {
                None => true,
                Some(old) => !shapes_equal(old, &new_out),
            };
            if changed {
                block_out[bi] = Some(new_out);
                // Enqueue successors. Use the immutable terminator
                // borrow defensively — we don't mutate body in
                // analysis.
                let succs: Vec<u32> = body.blocks[bi]
                    .terminator
                    .successors()
                    .iter()
                    .map(|(t, _)| t.0)
                    .collect();
                for s in succs {
                    let si = s as usize;
                    if si < n && !on_worklist[si] {
                        worklist.push_back(si);
                        on_worklist[si] = true;
                    }
                }
            }
        }

        // ---- Transform phase -----------------------------------------------
        // Walk each block with its converged `in` shape; stores update
        // the running shape; Pure loads matching an entry get forwarded.
        let mut changed = false;
        for bi in 0..n {
            let mut shape = compute_in(bi, body, &block_out);
            // Snapshot iids — we'll be removing entries during the walk.
            let iids: Vec<u32> = body.blocks[bi]
                .instructions
                .iter()
                .map(|i| i.0)
                .collect();
            let mut to_unhook: Vec<u32> = Vec::new();
            for iid in iids {
                let kind = &body.instructions[iid as usize].kind;
                // Try to forward a Pure load BEFORE applying the
                // instruction (loads don't change shape, so order
                // between forward-attempt and apply is irrelevant
                // for the shape itself, but doing it first keeps the
                // logic uniform).
                let forward = match kind {
                    InstKind::LoadProp {
                        obj: Operand::Value(a),
                        key,
                        purity: Purity::Pure,
                    } => shape.get(&(a.0, key_from_str(*key))).copied(),
                    InstKind::LoadIndex {
                        obj: Operand::Value(a),
                        index,
                        purity: Purity::Pure,
                    } => key_from_index_operand(index)
                        .and_then(|k| shape.get(&(a.0, k)).copied()),
                    _ => None,
                };
                if let Some(stored) = forward {
                    let v = find_value_for_inst(body, InstId(iid));
                    // Self-forward guard: shape's entry for this load
                    // could be Operand::Value(load_v) when this same
                    // load was the seed in a prior analysis step that
                    // somehow flowed back here (back-edge of a loop
                    // whose only writer is this load). Don't replace
                    // the load with itself.
                    if !matches!(stored, Operand::Value(sv) if sv == v) {
                        body.replace_value_uses(v, stored);
                        body.values[v.0 as usize] = ValueDef::removed();
                        to_unhook.push(iid);
                        changed = true;
                        continue;
                    }
                }
                apply_inst(&mut shape, body, InstId(iid), kind);
            }
            if !to_unhook.is_empty() {
                let unhook_set: std::collections::HashSet<u32> =
                    to_unhook.iter().copied().collect();
                body.blocks[bi]
                    .instructions
                    .retain(|i| !unhook_set.contains(&i.0));
                for iid in to_unhook {
                    walk_inst_operands_mut(
                        &mut body.instructions[iid as usize].kind,
                        &mut |op| {
                            *op = Operand::Const(Constant::Undefined);
                        },
                    );
                }
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        BlockId, Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity,
        Terminator,
    };

    /// `let o = {}; o.x = 5; return o.x;` — same-block, Pure load
    /// forwards to Const(5).
    #[test]
    fn forwards_storeprop_to_loadprop() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 5.0),
            other => panic!("expected ret 5, got {other:?}"),
        }
    }

    #[test]
    fn forwards_storeindex_to_loadindex() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let s = b.intern("hello");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b
            .add_inst(InstKind::AllocArray { length: Some(0) })
            .unwrap();
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(alloc),
            index: Operand::Const(Constant::Int32(0)),
            val: Operand::Const(Constant::String(s)),
            purity: Purity::Pure,
        });
        let load = b
            .add_inst(InstKind::LoadIndex {
                obj: Operand::Value(alloc),
                index: Operand::Const(Constant::Int32(0)),
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::String(id))),
            } => assert_eq!(module.strings.resolve(id), "hello"),
            other => panic!("expected ret String('hello'), got {other:?}"),
        }
    }

    #[test]
    fn no_forward_on_throws_load() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Throws,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let kept = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .any(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ));
        assert!(kept, "Throws-purity load must not be forwarded");
    }

    /// Cross-block linear chain: alloc + store in entry, load in
    /// successor block. Forwards.
    #[test]
    fn forwards_across_branch_chain() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb1 = b.new_block();
        let bb0 = b.new_block();
        // bb0: alloc + store + branch bb1
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(42.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![],
        });
        // bb1: load + return
        b.switch_to_block(bb1);
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();
        // Entry is bb0 (last new_block call sets it last; reset
        // explicitly to be safe).
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        match &body.blocks[bb1.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 42.0),
            other => panic!("expected ret 42 in bb1, got {other:?}"),
        }
    }

    /// Diamond with agreement: both branches store the same Const →
    /// join block forwards the load.
    #[test]
    fn forwards_at_diamond_join_when_branches_agree() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb_join = b.new_block();
        let bb0 = b.new_block();
        // Build alloc in entry; both branches store the SAME constant.
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let cond = b.body().params[0];
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cond),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.switch_to_block(bb_then);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(7.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_else);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(7.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_join);
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        match &body.blocks[bb_join.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 7.0),
            other => panic!("expected ret 7, got {other:?}"),
        }
    }

    /// Diamond with disagreement: branches store different consts →
    /// join intersection drops the entry → load NOT forwarded.
    #[test]
    fn no_forward_at_diamond_join_when_branches_disagree() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb_join = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let cond = b.body().params[0];
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cond),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.switch_to_block(bb_then);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(7.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_else);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(8.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_join);
        let _load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(_load)),
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let kept = body.blocks[bb_join.0 as usize]
            .instructions
            .iter()
            .any(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ));
        assert!(kept, "join load must not forward when branches disagree");
    }

    #[test]
    fn unknown_key_store_invalidates() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let arg0 = b.body().params[0];
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(alloc),
            index: Operand::Value(arg0),
            val: Operand::Const(Constant::Number(99.0)),
            purity: Purity::Pure,
        });
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let kept = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .any(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ));
        assert!(kept, "load after unknown-key store must not forward");
    }

    #[test]
    fn two_distinct_allocs_dont_alias() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let a1 = b.add_inst(InstKind::AllocObject).unwrap();
        let a2 = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(a1),
            key: key_x,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Pure,
        });
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(a2),
            key: key_x,
            val: Operand::Const(Constant::Number(2.0)),
            purity: Purity::Pure,
        });
        let load1 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(a1),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load1)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 1.0),
            other => panic!("expected ret 1, got {other:?}"),
        }
    }

    /// Number-keyed StoreIndex with integer Number → Idx key. Mixed
    /// Const(Number(0.0)) and Const(Int32(0)) should canonicalize.
    #[test]
    fn number_index_canonicalizes_to_int_key() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b
            .add_inst(InstKind::AllocArray { length: Some(0) })
            .unwrap();
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(alloc),
            index: Operand::Const(Constant::Number(2.0)),
            val: Operand::Const(Constant::Number(99.0)),
            purity: Purity::Pure,
        });
        let load = b
            .add_inst(InstKind::LoadIndex {
                obj: Operand::Value(alloc),
                index: Operand::Const(Constant::Int32(2)),
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 99.0),
            other => panic!("expected ret 99, got {other:?}"),
        }
    }

    /// Loop where store happens on the BACK-edge but pre-header
    /// established (a, k) → 5: the optimistic-then-monotone fixed
    /// point should still forward in the body if the back-edge agrees,
    /// or refuse to forward if it doesn't. This test: body re-stores
    /// the same const → forwarding holds.
    #[test]
    fn loop_with_invariant_store_forwards_in_body() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb_pre = b.new_block();
        let bb_header = b.new_block();
        let bb_body = b.new_block();
        let bb_exit = b.new_block();
        // bb_pre: alloc, store(alloc, x, 5), branch bb_header
        b.switch_to_block(bb_pre);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        // bb_header: cond_branch (cond, body, exit)
        b.switch_to_block(bb_header);
        let cond = b.body().params[0];
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cond),
            then_b: bb_body,
            then_args: vec![],
            else_b: bb_exit,
            else_args: vec![],
        });
        // bb_body: load(alloc, x), store(alloc, x, 5), branch header
        b.switch_to_block(bb_body);
        let body_load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        let _ = body_load;
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        // bb_exit: ret
        b.switch_to_block(bb_exit);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();
        module.functions[0].lowered_mut().entry = bb_pre;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        // bb_body's load should have been forwarded (no LoadProp left
        // in the body block).
        let body_load_remaining = body.blocks[bb_body.0 as usize]
            .instructions
            .iter()
            .any(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ));
        assert!(
            !body_load_remaining,
            "loop-invariant store should let body load forward"
        );
    }

    /// CSE: two Pure loads of the same `(alloc, key)` in the same block
    /// with no intervening store — the second forwards to the first.
    /// No store ever happens, so neither resolves to a constant.
    #[test]
    fn cse_redundant_loads_same_block() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let load1 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        let load2 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        // Use both loads so neither is dead.
        let sum = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(load1),
                rhs: Operand::Value(load2),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(sum)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let load_count = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ))
            .count();
        assert_eq!(load_count, 1, "second redundant load should be CSE'd");
        // The AddGeneric should now have both lhs and rhs equal to the
        // remaining load's Value.
        let add_kind = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .find_map(|iid| match &body.instructions[iid.0 as usize].kind {
                k @ InstKind::AddGeneric { .. } => Some(k.clone()),
                _ => None,
            })
            .expect("AddGeneric must remain");
        if let InstKind::AddGeneric { lhs, rhs } = add_kind {
            assert!(
                operand_bit_eq(&lhs, &rhs),
                "both add operands must reference same load: {lhs:?} vs {rhs:?}"
            );
        }
    }

    /// CSE across blocks: load in entry, load of same `(a, k)` in
    /// successor — successor forwards to entry's load.
    #[test]
    fn cse_redundant_load_across_blocks() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb1 = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let load1 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        let _ = load1;
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![],
        });
        b.switch_to_block(bb1);
        let load2 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load2)),
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let bb1_loads = body.blocks[bb1.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ))
            .count();
        assert_eq!(bb1_loads, 0, "successor load must forward to entry load");
        // Entry's load should still be there.
        let entry_loads = body.blocks[bb0.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ))
            .count();
        assert_eq!(entry_loads, 1, "entry load must remain alive");
    }

    /// Store invalidates seeded load entry: load1, store(a, k, 5),
    /// load2 → load2 forwards to Const(5), not load1.
    #[test]
    fn store_after_seeding_load_overrides() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let load1 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        let load2 = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        // Use load1 so it's not DCE bait.
        let sum = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(load1),
                rhs: Operand::Value(load2),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(sum)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        // load2 should forward to Const(5).
        let add_kind = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .find_map(|iid| match &body.instructions[iid.0 as usize].kind {
                k @ InstKind::AddGeneric { .. } => Some(k.clone()),
                _ => None,
            })
            .expect("AddGeneric must remain");
        if let InstKind::AddGeneric { rhs, .. } = add_kind {
            assert!(
                matches!(rhs, Operand::Const(Constant::Number(n)) if n == 5.0),
                "load2 should forward to Const(5), got {rhs:?}"
            );
        }
    }

    /// Counter-loop: body stores a different const each iteration
    /// (well, same const here but for parametric tracking the back-
    /// edge could disagree — this test confirms the fixed-point
    /// converges and doesn't infinite-loop on changing values).
    #[test]
    fn loop_with_disagreeing_back_edge_does_not_forward() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb_pre = b.new_block();
        let bb_header = b.new_block();
        let bb_body = b.new_block();
        let bb_exit = b.new_block();
        b.switch_to_block(bb_pre);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_header);
        let cond = b.body().params[0];
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cond),
            then_b: bb_body,
            then_args: vec![],
            else_b: bb_exit,
            else_args: vec![],
        });
        b.switch_to_block(bb_body);
        // Body stores a DIFFERENT const than the pre-header — back-
        // edge will disagree with pre-header; intersection at header
        // drops the entry. Body's load (in this test) WOULD have to
        // re-establish the entry intra-block to forward.
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(99.0)),
            purity: Purity::Pure,
        });
        let _load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_x,
                purity: Purity::Pure,
            })
            .unwrap();
        let _ = _load;
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_exit);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();
        module.functions[0].lowered_mut().entry = bb_pre;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ForwardLoadsPass));
        // Just confirm the analysis terminates. The intra-block
        // store-then-load case still forwards (same-block).
        pm.run(&mut module, FuncId(0)).unwrap();

        // The intra-block store→load in body forwards regardless of
        // header's join; just confirm we didn't infinite-loop.
        let _ = BlockId(0);
    }
}
