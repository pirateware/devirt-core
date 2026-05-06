//! Purity refinement on heap accesses rooted at unaliased local
//! allocations.
//!
//! `LoadProp` / `LoadIndex` / `StoreProp` / `StoreIndex` default to
//! `Purity::Opaque` because in general the obj could be a Proxy or
//! getter-bearing object. But when the obj is an `AllocObject` or
//! `AllocArray` whose result Value never escapes this function (the
//! only uses are as the `obj` operand of those four heap-access
//! kinds, and it's not captured by a PassThrough body), there's no
//! Proxy, no getter chain, no aliasing — the access is provably
//! `Purity::Pure`. That unlocks DCE on dead reads and is the
//! infrastructure the future heap-forwarding pass needs.
//!
//! Conservative by design:
//! - Only AllocObject and AllocArray. AllocClosure (functions),
//!   AllocRegExp (regex objects) have their own semantics.
//! - "Escape" check is structural: any use outside the four heap-
//!   access kinds disqualifies. That excludes Call args, Return
//!   values, branch args (block params), comparisons, type checks,
//!   stores into ANOTHER object's heap, etc.
//! - The alloc Value's source_name doesn't matter — pinning is about
//!   identity for the renamer, not about heap aliasing semantics.

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    FuncId, InstKind, Module, Operand, Purity, walk_inst_operands, walk_term_operands,
};

use crate::FunctionPass;

pub struct RefinePurityPass;

impl FunctionPass for RefinePurityPass {
    fn name(&self) -> &'static str {
        "refine_purity"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let captured: HashSet<u32> =
            func.captured_by_passthrough.iter().copied().collect();
        let body = func.lowered_mut();

        // Phase 1: identify candidate alloc Values (AllocObject/AllocArray).
        // Map alloc Value index → result-defining InstId, for cross-checking.
        let mut alloc_vals: HashSet<u32> = HashSet::new();
        for (vi, def) in body.values.iter().enumerate() {
            if let ir::ssa::ValueDefKind::Inst(iid) = def.kind {
                if matches!(
                    body.instructions[iid.0 as usize].kind,
                    InstKind::AllocObject | InstKind::AllocArray { .. }
                ) && !captured.contains(&(vi as u32))
                {
                    alloc_vals.insert(vi as u32);
                }
            }
        }

        if alloc_vals.is_empty() {
            return Ok(false);
        }

        // Phase 2: walk every operand. For each alloc Value, classify
        // every appearance: "obj of heap access" (allowed) vs anything
        // else (escapes). Track per-alloc the InstIds of heap accesses
        // we'd refine if it stays unaliased.
        //
        // We can't refine the AllocArray's own elements operands —
        // those are the array's initial contents; they aren't the obj.
        // Walk every InstKind explicitly via match-on-kind (rather than
        // walk_inst_operands which doesn't distinguish positions).
        let mut escapes: HashSet<u32> = HashSet::new();
        let mut heap_use_iids_by_alloc: HashMap<u32, Vec<u32>> = HashMap::new();

        for (idx, inst) in body.instructions.iter().enumerate() {
            // Check obj-position uses by inst kind. Anything that's
            // OBJ-position counts as a "heap access" use; the same
            // Value showing up in a different operand position counts
            // as escape.
            let (obj_alloc, is_heap_obj) = match &inst.kind {
                InstKind::LoadProp { obj, .. }
                | InstKind::LoadIndex { obj, .. }
                | InstKind::StoreProp { obj, .. }
                | InstKind::StoreIndex { obj, .. } => match obj {
                    Operand::Value(v) if alloc_vals.contains(&v.0) => (Some(v.0), true),
                    _ => (None, false),
                },
                _ => (None, false),
            };

            // Heap-obj use: stash the InstId for refinement.
            if let (Some(av), true) = (obj_alloc, is_heap_obj) {
                heap_use_iids_by_alloc
                    .entry(av)
                    .or_default()
                    .push(idx as u32);
            }

            // Now scan ALL operands; any reference to an alloc Value
            // that isn't the obj-of-heap-access counts as escape.
            walk_inst_operands(&inst.kind, |op| {
                if let Operand::Value(v) = op
                    && alloc_vals.contains(&v.0)
                {
                    let is_obj_position = obj_alloc == Some(v.0);
                    if !is_obj_position {
                        escapes.insert(v.0);
                    }
                }
            });
        }

        // Terminator operands always escape (return val, branch args,
        // throw val, switch disc, etc.).
        for blk in &body.blocks {
            walk_term_operands(&blk.terminator, |op| {
                if let Operand::Value(v) = op
                    && alloc_vals.contains(&v.0)
                {
                    escapes.insert(v.0);
                }
            });
        }

        // Phase 3: refine surviving allocs' heap accesses to Pure.
        let mut changed = false;
        for av in &alloc_vals {
            if escapes.contains(av) {
                continue;
            }
            let Some(iids) = heap_use_iids_by_alloc.get(av) else {
                continue;
            };
            for &iid_idx in iids {
                let kind = &mut body.instructions[iid_idx as usize].kind;
                match kind {
                    InstKind::LoadProp { purity, .. }
                    | InstKind::LoadIndex { purity, .. }
                    | InstKind::StoreProp { purity, .. }
                    | InstKind::StoreIndex { purity, .. } => {
                        if !matches!(*purity, Purity::Pure) {
                            *purity = Purity::Pure;
                            changed = true;
                        }
                    }
                    _ => unreachable!("recorded inst was not a heap access"),
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
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
    };

    /// alloc is unaliased — only used as obj of one StoreProp + one LoadProp.
    /// Both should refine to Pure.
    #[test]
    fn refines_unaliased_alloc_loads_and_stores() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(load)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefinePurityPass));
        let iters = pm.run(&mut module, FuncId(0)).unwrap();
        assert!(iters >= 1);

        let body = module.functions[0].lowered();
        let mut load_purities = Vec::new();
        let mut store_purities = Vec::new();
        for inst in &body.instructions {
            match &inst.kind {
                InstKind::LoadProp { purity, .. } => load_purities.push(*purity),
                InstKind::StoreProp { purity, .. } => store_purities.push(*purity),
                _ => {}
            }
        }
        assert_eq!(load_purities, vec![Purity::Pure]);
        assert_eq!(store_purities, vec![Purity::Pure]);
    }

    /// alloc escapes via Return — its Loads must NOT be refined.
    #[test]
    fn no_refine_when_alloc_escapes_via_return() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let load = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        // Return the alloc itself — escape.
        let _ = load;
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(alloc)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefinePurityPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        for inst in &body.instructions {
            if let InstKind::LoadProp { purity, .. } = &inst.kind {
                assert!(
                    matches!(*purity, Purity::Opaque),
                    "load on escaped alloc must stay Opaque"
                );
            }
        }
    }

    /// alloc passed as a Call argument — escape, no refine.
    #[test]
    fn no_refine_when_alloc_escapes_via_call_arg() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let key_use = b.intern("use");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let _ = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(alloc),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        // Look up `use` from globalThis and call it with alloc as arg.
        let callee = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Const(Constant::GlobalThis),
                key: key_use,
                purity: Purity::Throws,
            })
            .unwrap();
        b.add_inst(InstKind::Call {
            callee: Operand::Value(callee),
            args: vec![Operand::Value(alloc)],
        });
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefinePurityPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        // The first LoadProp (on the alloc) should NOT be refined.
        let alloc_load_purity = body.instructions.iter().find_map(|i| match &i.kind {
            InstKind::LoadProp {
                obj: Operand::Value(_),
                purity,
                ..
            } => Some(*purity),
            _ => None,
        });
        assert!(
            matches!(alloc_load_purity, Some(Purity::Opaque)),
            "alloc-load purity should remain Opaque after escape"
        );
    }
}
