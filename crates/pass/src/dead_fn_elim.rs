//! Dead-function elimination — drops top-level function declarations
//! whose only references are their own GlobalThis-store hoist.
//!
//! After identity-function inlining, the wrapper bodies (`function _0x1
//! (x) { return x; }`, etc.) typically have no remaining Const(Func(_))
//! uses anywhere — every Call site that referenced them got rewritten
//! into the resolved expression. The only thing keeping them visible
//! is the synthesized `StoreProp(GlobalThis, "_0x1", Const(Func(_0x1)))`
//! that the SSA converter emits to register the function declaration.
//!
//! This pass:
//!   1. Scans all functions for Const(Func(g)) operands and AllocClosure
//!      uses, classifying each ref as "decl-store" (val of a StoreProp
//!      onto GlobalThis) or "real use" (anything else).
//!   2. Scans for LoadProp/LoadIndex on GlobalThis → set of names that
//!      could be reading function bindings. If any LoadIndex on
//!      GlobalThis has a non-const key, poisons the analysis.
//!   3. A function `g` is dead iff: not a closure target, has the
//!      trivial-return shape (one block, zero insts, returns an arg
//!      or a clonable const — same predicate IdentityFnInlinePass
//!      uses to identify wrappers), has zero real uses, and every
//!      decl-store name for `g` is absent from the name-loads set
//!      (and no poison). The trivial-shape gate is what protects
//!      user-defined entry points: a real top-level function with
//!      a non-trivial body is never dropped, even if the file has no
//!      internal callers.
//!   4. For each dead g: unhook its decl-stores from their containing
//!      blocks and remove `Function(g)` entries from
//!      `module.top_level_layout`.
//!
//! The Function struct stays at its slot in `module.functions[]`
//! (FuncId stability) but emit doesn't render it — the top-level
//! walker only touches functions named in `top_level_layout`.

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    Constant, FunctionBody, InstId, InstKind, Module, Operand, StrId, Terminator,
    TopLevelEntry, ValueDefKind, walk_inst_operands, walk_term_operands,
};

use crate::ModulePass;

/// Mirrors `IdentityFnInlinePass`'s body-shape predicate. A function
/// matches if its body is one block, zero instructions, terminator
/// `Return(Some(_))` referencing either an arg or a clonable constant.
/// The actual operand doesn't matter here — we only need to know the
/// function is in the obfuscation-wrapper class that identity-inlining
/// targets.
fn is_trivial_return_shape(func: &ir::ssa::Function) -> bool {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return false,
    };
    if body.blocks.len() != 1 {
        return false;
    }
    let entry = &body.blocks[body.entry.0 as usize];
    if !entry.instructions.is_empty() || !entry.params.is_empty() {
        return false;
    }
    let val_op = match &entry.terminator {
        Terminator::Return { val: Some(op) } => *op,
        _ => return false,
    };
    match val_op {
        Operand::Const(c) => matches!(
            c,
            Constant::Number(_)
                | Constant::Int32(_)
                | Constant::Bool(_)
                | Constant::String(_)
                | Constant::Null
                | Constant::Undefined
                | Constant::Func(_)
        ),
        Operand::Value(v) => matches!(body.values[v.0 as usize].kind, ValueDefKind::Arg(_)),
        _ => false,
    }
}

pub struct DeadFnElimPass;

impl ModulePass for DeadFnElimPass {
    fn name(&self) -> &'static str {
        "dead_fn_elim"
    }

    fn run(&mut self, module: &mut Module) -> Result<bool, Error> {
        // Phase 1: walk every function body, collecting:
        // - real_uses: Const(Func(g)) appearing anywhere except as the
        //   `val` operand of a StoreProp targeting GlobalThis.
        // - decl_stores: per fid, the (enclosing_fid, block_index, inst_id, name)
        //   tuples for each StoreProp(GlobalThis, name, Const(Func(g))).
        // - global_name_loads: every const-string name read from GlobalThis.
        // - poison: any non-const LoadIndex on GlobalThis disqualifies all.
        // - closure_targets: AllocClosure target FuncIds (always alive).
        let mut real_uses: HashMap<u32, u32> = HashMap::new();
        let mut decl_stores: HashMap<u32, Vec<(u32, usize, InstId, StrId)>> = HashMap::new();
        let mut global_name_loads: HashSet<StrId> = HashSet::new();
        let mut poison = false;
        let mut closure_targets: HashSet<u32> = HashSet::new();

        for (fid_idx, func) in module.functions.iter().enumerate() {
            let body = match &func.body {
                FunctionBody::Lowered(b) => b,
                FunctionBody::PassThrough(_) => continue,
            };
            for (bi, blk) in body.blocks.iter().enumerate() {
                for &iid in &blk.instructions {
                    let kind = &body.instructions[iid.0 as usize].kind;

                    // Classify decl-store shape vs real Const(Func) use.
                    let mut decl_store_target: Option<(u32, StrId)> = None;
                    if let InstKind::StoreProp {
                        obj: Operand::Const(Constant::GlobalThis),
                        key,
                        val: Operand::Const(Constant::Func(target)),
                        ..
                    } = kind
                    {
                        decl_store_target = Some((target.0, *key));
                    }

                    if let Some((target_fid, key)) = decl_store_target {
                        decl_stores
                            .entry(target_fid)
                            .or_default()
                            .push((fid_idx as u32, bi, iid, key));
                    }

                    // Collect real uses + AllocClosure targets + GlobalThis reads.
                    walk_inst_operands(kind, |op| match op {
                        Operand::Const(Constant::Func(g)) => {
                            if decl_store_target.map(|(t, _)| t) != Some(g.0) {
                                *real_uses.entry(g.0).or_insert(0) += 1;
                            }
                        }
                        _ => {}
                    });

                    match kind {
                        InstKind::AllocClosure { func, .. } => {
                            closure_targets.insert(func.0);
                        }
                        InstKind::LoadProp {
                            obj: Operand::Const(Constant::GlobalThis),
                            key,
                            ..
                        } => {
                            global_name_loads.insert(*key);
                        }
                        InstKind::LoadIndex {
                            obj: Operand::Const(Constant::GlobalThis),
                            index,
                            ..
                        } => match index {
                            Operand::Const(Constant::String(s)) => {
                                global_name_loads.insert(*s);
                            }
                            // Non-const-key load on globalThis: any name
                            // could be read at runtime. Poison.
                            _ => poison = true,
                        },
                        _ => {}
                    }
                }

                walk_term_operands(&blk.terminator, |op| {
                    if let Operand::Const(Constant::Func(g)) = op {
                        *real_uses.entry(g.0).or_insert(0) += 1;
                    }
                });
            }
        }

        if poison {
            return Ok(false);
        }

        // Phase 2: classify each function as dead or alive.
        //
        // We restrict deletion to functions with the trivial-return shape
        // — the same predicate `IdentityFnInlinePass` uses to identify
        // obfuscation wrappers. This avoids deleting top-level user
        // functions that just happen to have no internal callers (they
        // may be invoked from outside the file: a runtime, a REPL, an
        // entry-point hook). The trivial shape is strong evidence of an
        // obfuscation primitive that was probably already inlined out.
        let mut dead: Vec<u32> = Vec::new();
        for (fid_idx, func) in module.functions.iter().enumerate() {
            let g = fid_idx as u32;
            if closure_targets.contains(&g) {
                continue; // referenced from AllocClosure
            }
            if real_uses.get(&g).copied().unwrap_or(0) > 0 {
                continue;
            }
            if !is_trivial_return_shape(func) {
                continue;
            }
            // Decl-stores: every name must NOT be in global_name_loads.
            let stores = decl_stores.get(&g).cloned().unwrap_or_default();
            let any_name_loaded =
                stores.iter().any(|(_, _, _, name)| global_name_loads.contains(name));
            if any_name_loaded {
                continue;
            }
            dead.push(g);
        }

        if dead.is_empty() {
            return Ok(false);
        }
        let dead_set: HashSet<u32> = dead.iter().copied().collect();

        // Phase 3: drop decl-stores for dead functions.
        for g in &dead {
            let Some(stores) = decl_stores.get(g) else {
                continue;
            };
            for (enclosing_fid, bi, iid, _) in stores {
                let body = match &mut module.functions[*enclosing_fid as usize].body {
                    FunctionBody::Lowered(b) => b,
                    FunctionBody::PassThrough(_) => continue,
                };
                let blk = &mut body.blocks[*bi];
                blk.instructions.retain(|i| i.0 != iid.0);
                // StoreProp operands are all Consts (GlobalThis + name +
                // Const(Func(g))) — no Value refs to blank.
            }
        }

        // Phase 4: drop dead functions from the top-level layout.
        module
            .top_level_layout
            .retain(|entry| match entry {
                TopLevelEntry::Function(fid) => !dead_set.contains(&fid.0),
                _ => true,
            });

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
        TopLevelEntry,
    };

    /// `function unused() { return 1; }; function main() { return 2; }` —
    /// `main` is the entry, `unused` has no uses → dropped.
    #[test]
    fn drops_unreferenced_top_level_function() {
        let mut module = Module::new();
        // FuncId(0) is "main" — registers `unused` and `main` via decl-stores.
        let unused_name;
        let main_name;
        {
            let mut b = FunctionBuilder::new(&mut module, "main_module", 0);
            unused_name = b.intern("unused");
            main_name = b.intern("main");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            // Synthesize the two decl-stores.
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Const(Constant::GlobalThis),
                key: unused_name,
                val: Operand::Const(Constant::Func(FuncId(1))),
                purity: Purity::Throws,
            });
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Const(Constant::GlobalThis),
                key: main_name,
                val: Operand::Const(Constant::Func(FuncId(2))),
                purity: Purity::Throws,
            });
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        // FuncId(1) "unused" — body returns 1.
        {
            let mut b = FunctionBuilder::new(&mut module, "unused", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Number(1.0))),
            });
            b.finish();
        }
        // FuncId(2) "main" — body returns 2.
        {
            let mut b = FunctionBuilder::new(&mut module, "main", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Number(2.0))),
            });
            b.finish();
        }
        // Layout names all three.
        module.top_level_layout = vec![
            TopLevelEntry::Function(FuncId(0)),
            TopLevelEntry::Function(FuncId(1)),
            TopLevelEntry::Function(FuncId(2)),
        ];

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(DeadFnElimPass));
        pm.run_module(&mut module).unwrap();

        // unused should be gone from layout AND its decl-store dropped.
        let layout_fids: Vec<u32> = module
            .top_level_layout
            .iter()
            .filter_map(|e| match e {
                TopLevelEntry::Function(f) => Some(f.0),
                _ => None,
            })
            .collect();
        // Both 0 (entry) and 2 (main, has its name loaded? actually no
        // load happens here — but main's Const(Func) is referenced only
        // by its own decl-store, so by the policy it IS dead too).
        // To check: only `unused` (FuncId 1) is dead; main (FuncId 2)
        // is also dead because nothing loads its name. That's expected
        // for this test setup — both unused functions get removed.
        assert!(!layout_fids.contains(&1), "unused should be dropped");
        // FuncId(0) (entry) always stays.
        assert!(layout_fids.contains(&0));

        // The decl-store for `unused` should be gone from module fn 0.
        let main_module = module.functions[0].lowered();
        let entry = main_module.entry;
        let store_targets: Vec<u32> = main_module.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter_map(|iid| {
                if let InstKind::StoreProp {
                    val: Operand::Const(Constant::Func(g)),
                    ..
                } = main_module.instructions[iid.0 as usize].kind
                {
                    Some(g.0)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !store_targets.contains(&1),
            "decl-store for unused must be dropped"
        );
    }

    /// A function whose name IS loaded from globalThis must be kept.
    #[test]
    fn keeps_function_when_name_is_loaded() {
        let mut module = Module::new();
        let kept_name;
        {
            let mut b = FunctionBuilder::new(&mut module, "main_module", 0);
            kept_name = b.intern("kept");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Const(Constant::GlobalThis),
                key: kept_name,
                val: Operand::Const(Constant::Func(FuncId(1))),
                purity: Purity::Throws,
            });
            // Loads `kept` from globalThis — this is what keeps it alive.
            let _ = b
                .add_inst(InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    key: kept_name,
                    purity: Purity::Throws,
                })
                .unwrap();
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "kept", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        module.top_level_layout = vec![
            TopLevelEntry::Function(FuncId(0)),
            TopLevelEntry::Function(FuncId(1)),
        ];

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(DeadFnElimPass));
        pm.run_module(&mut module).unwrap();

        let layout_fids: Vec<u32> = module
            .top_level_layout
            .iter()
            .filter_map(|e| match e {
                TopLevelEntry::Function(f) => Some(f.0),
                _ => None,
            })
            .collect();
        assert!(
            layout_fids.contains(&1),
            "function whose name is read must be kept"
        );
    }

    /// Function used as `Call` callee (not just a decl-store) must be kept.
    #[test]
    fn keeps_called_function() {
        let mut module = Module::new();
        let helper_name;
        {
            let mut b = FunctionBuilder::new(&mut module, "main_module", 0);
            helper_name = b.intern("helper");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Const(Constant::GlobalThis),
                key: helper_name,
                val: Operand::Const(Constant::Func(FuncId(1))),
                purity: Purity::Throws,
            });
            let _ = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(1))),
                    args: vec![],
                })
                .unwrap();
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "helper", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        module.top_level_layout = vec![
            TopLevelEntry::Function(FuncId(0)),
            TopLevelEntry::Function(FuncId(1)),
        ];

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(DeadFnElimPass));
        pm.run_module(&mut module).unwrap();

        let layout_fids: Vec<u32> = module
            .top_level_layout
            .iter()
            .filter_map(|e| match e {
                TopLevelEntry::Function(f) => Some(f.0),
                _ => None,
            })
            .collect();
        assert!(
            layout_fids.contains(&1),
            "called function must be kept (real Const(Func) use)"
        );
    }
}
