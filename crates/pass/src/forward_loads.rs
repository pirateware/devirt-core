//! Heap store-load forwarding — resolve Pure `LoadProp`/`LoadIndex`
//! against preceding `StoreProp`/`StoreIndex` on the same unaliased
//! allocation. Same-block dataflow only.
//!
//! `RefinePurityPass` is the precision baseline: only loads marked
//! `Purity::Pure` are forwarded. Pure means the receiver is a
//! provably-unaliased local allocation (no Proxy, no escape), so
//! the heap-shape map per (alloc, key) is correct.
//!
//! Same-block only for v1: cross-block dataflow needs a join lattice
//! and per-block fixed point. Babel-style `{foo:1}.foo` and array-
//! literal-then-immediate-indexed-read both fit same-block.
//!
//! Side-effect barriers: any instruction that writes the heap on a
//! receiver we can't prove distinct (Calls, OpaqueEval,
//! WRITES_HEAP-bearing kinds outside the tracked allocs) flushes the
//! map. This is conservative — real aliasing is impossible because
//! Pure marking already proved unaliased — but the map is stored
//! per-block-walk anyway; flushing on barriers is cheap insurance.

use std::collections::HashMap;

use ir::error::Error;
use ir::ssa::{
    Constant, Effect, FuncId, InstKind, Module, Operand, Purity, ValueDef, ValueDefKind,
    find_value_for_inst, walk_inst_operands_mut,
};

use crate::FunctionPass;

pub struct ForwardLoadsPass;

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
enum Key {
    Str(ir::ssa::StrId),
    Idx(i32),
}

fn key_from_str(s: ir::ssa::StrId) -> Key {
    Key::Str(s)
}

/// Map a constant operand to a `Key`. Numbers with non-integer values
/// or out-of-i32-range don't form a key (treated as missing).
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

/// Is this Value defined by an `AllocObject` or `AllocArray`? Those
/// are the only allocations whose heap shape we track. (Other
/// allocations — closures, regexes — don't get StoreProp/StoreIndex
/// in the obfuscation patterns this pass targets.)
fn is_tracked_alloc(body: &ir::ssa::LoweredBody, v: ir::ssa::Value) -> bool {
    if let ValueDefKind::Inst(iid) = body.values[v.0 as usize].kind {
        matches!(
            body.instructions[iid.0 as usize].kind,
            InstKind::AllocObject | InstKind::AllocArray { .. }
        )
    } else {
        false
    }
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

        let mut changed = false;
        for bi in 0..body.blocks.len() {
            // Heap-shape map for this block walk. Cleared per block
            // (no cross-block dataflow yet).
            let mut shape: HashMap<(u32, Key), Operand> = HashMap::new();
            // Snapshot the inst-id list — we'll be removing entries
            // during the walk.
            let iids: Vec<u32> =
                body.blocks[bi].instructions.iter().map(|i| i.0).collect();
            let mut to_unhook: Vec<u32> = Vec::new();

            for iid in iids {
                let kind = &body.instructions[iid as usize].kind;
                match kind {
                    // Stores: update the map for known-key stores on a
                    // tracked alloc. Unknown-key stores invalidate
                    // every entry for that receiver.
                    InstKind::StoreProp { obj, key, val, .. } => {
                        if let Operand::Value(a) = obj
                            && is_tracked_alloc(body, *a)
                        {
                            shape.insert((a.0, key_from_str(*key)), *val);
                        } else {
                            // Store on something other than a tracked
                            // alloc: could touch heap state we can't
                            // model. Flush.
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
                                // Unknown key on tracked alloc: drop
                                // every entry for this receiver only.
                                shape.retain(|(av, _), _| *av != a.0);
                            }
                        } else {
                            shape.clear();
                        }
                    }
                    // Loads: try to forward.
                    InstKind::LoadProp {
                        obj,
                        key,
                        purity: Purity::Pure,
                    } => {
                        if let Operand::Value(a) = obj
                            && let Some(stored) = shape.get(&(a.0, key_from_str(*key)))
                        {
                            let stored = *stored;
                            let v = find_value_for_inst(body, ir::ssa::InstId(iid));
                            body.replace_value_uses(v, stored);
                            body.values[v.0 as usize] = ValueDef::removed();
                            to_unhook.push(iid);
                            changed = true;
                        }
                    }
                    InstKind::LoadIndex {
                        obj,
                        index,
                        purity: Purity::Pure,
                    } => {
                        if let Operand::Value(a) = obj
                            && let Some(k) = key_from_index_operand(index)
                            && let Some(stored) = shape.get(&(a.0, k))
                        {
                            let stored = *stored;
                            let v = find_value_for_inst(body, ir::ssa::InstId(iid));
                            body.replace_value_uses(v, stored);
                            body.values[v.0 as usize] = ValueDef::removed();
                            to_unhook.push(iid);
                            changed = true;
                        }
                    }
                    // Allocations don't disturb existing entries —
                    // a fresh alloc is independent of all prior allocs.
                    InstKind::AllocObject
                    | InstKind::AllocArray { .. }
                    | InstKind::AllocClosure { .. }
                    | InstKind::AllocRegExp { .. } => {}
                    // Anything else: if the inst has WRITES_HEAP in its
                    // effect (Calls, OpaqueEval, etc.), flush the map
                    // — it could write to anything we're tracking.
                    other => {
                        if other.effect().contains(Effect::WRITES_HEAP) {
                            shape.clear();
                        }
                    }
                }
            }

            if !to_unhook.is_empty() {
                // Apply removals: drop unhooked InstIds from this
                // block, blank their operands. Same DCE convention as
                // ConstFoldPass / DcePass.
                let unhook_set: std::collections::HashSet<u32> =
                    to_unhook.iter().copied().collect();
                body.blocks[bi].instructions.retain(|i| !unhook_set.contains(&i.0));
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
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
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

    /// `let arr = []; arr[0] = "hello"; return arr[0];` — array form.
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

    /// Non-Pure (Throws) load is NOT forwarded. RefinePurity gates
    /// forwarding eligibility.
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
        // LoadProp must still be in the block's inst list.
        let kept = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .any(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ));
        assert!(kept, "Throws-purity load must not be forwarded");
    }

    /// Unknown-key store before the load invalidates the map for that
    /// receiver — load is not forwarded.
    #[test]
    fn unknown_key_store_invalidates() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let arg0 = b.body().params[0]; // unknown key Value
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Pure,
        });
        // Unknown-key store: invalidates entries for `alloc`.
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
        assert!(kept, "load after unknown-key store on same alloc must not forward");
    }

    /// Two distinct allocations don't interfere.
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
            } => assert_eq!(*n, 1.0, "load on a1 must read a1's store, not a2's"),
            other => panic!("expected ret 1, got {other:?}"),
        }
    }
}
