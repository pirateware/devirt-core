//! Dead code elimination — remove instructions whose result Value has
//! no uses and whose effect is removable.
//!
//! Walks all instruction and terminator operands once to build a use
//! count per Value. Then iterates a worklist of zero-use Values whose
//! defining instruction has effect `removable_when_unused`, marking
//! each Value as `Removed` and decrementing uses for its operands.
//! Cascading removals (a → b → c, all dead) finish in one pass call.
//!
//! Pinned Values: anything in `Function.captured_by_passthrough` is
//! lexically referenced from a PassThrough body, so the IR-visible
//! use count understates real use. Those Values are never removed
//! regardless of effect or use count.
//!
//! Block-param Values are not handled here. Removing a trivially-
//! unused block-param requires synchronizing every predecessor's edge
//! args; it's a separate copy-prop / phi-removal concern.

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    Constant, FuncId, Module, Operand, Value, ValueDef, ValueDefKind, walk_inst_operands,
    walk_inst_operands_mut, walk_term_operands,
};

use crate::FunctionPass;

pub struct DcePass;

impl FunctionPass for DcePass {
    fn name(&self) -> &'static str {
        "dce"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let pinned: HashSet<u32> =
            func.captured_by_passthrough.iter().copied().collect();
        let body = func.lowered_mut();

        // Initial use counts.
        let mut uses: HashMap<u32, usize> = HashMap::new();
        for inst in &body.instructions {
            walk_inst_operands(&inst.kind, |op| {
                if let Operand::Value(v) = op {
                    *uses.entry(v.0).or_insert(0) += 1;
                }
            });
        }
        for blk in &body.blocks {
            walk_term_operands(&blk.terminator, |op| {
                if let Operand::Value(v) = op {
                    *uses.entry(v.0).or_insert(0) += 1;
                }
            });
        }

        // Map InstId → containing block index, so we can unhook in O(1).
        let mut inst_block: HashMap<u32, usize> = HashMap::new();
        for (bi, blk) in body.blocks.iter().enumerate() {
            for &iid in &blk.instructions {
                inst_block.insert(iid.0, bi);
            }
        }

        // Seed worklist: every Value defined by an Inst with use-count 0
        // and a removable effect, not pinned, not already Removed.
        let mut worklist: Vec<Value> = Vec::new();
        for (vi, def) in body.values.iter().enumerate() {
            if let ValueDefKind::Inst(iid) = &def.kind
                && !pinned.contains(&(vi as u32))
                && uses.get(&(vi as u32)).copied().unwrap_or(0) == 0
                && body.instructions[iid.0 as usize]
                    .kind
                    .effect()
                    .removable_when_unused()
            {
                worklist.push(Value(vi as u32));
            }
        }

        if worklist.is_empty() {
            return Ok(false);
        }

        let mut removed_any = false;
        while let Some(v) = worklist.pop() {
            // Re-check guards — earlier removal may have already retired
            // this slot via cascade, or pinning may have been added since.
            if body.values[v.0 as usize].is_removed() || pinned.contains(&v.0) {
                continue;
            }
            let iid = match &body.values[v.0 as usize].kind {
                ValueDefKind::Inst(iid) => *iid,
                _ => continue,
            };

            // Decrement uses for each operand of this instruction; cascade
            // any newly-zero, removable, non-pinned Values onto worklist.
            let kind_ptr: *const _ = &body.instructions[iid.0 as usize].kind;
            // SAFETY: `walk_inst_operands` reads the kind only; we don't
            // mutate `body.instructions` until after the closure ends.
            // Splitting borrows would otherwise force a clone of the kind.
            let kind = unsafe { &*kind_ptr };
            walk_inst_operands(kind, |op| {
                if let Operand::Value(used) = op {
                    if let Some(c) = uses.get_mut(&used.0) {
                        if *c > 0 {
                            *c -= 1;
                        }
                        if *c == 0
                            && !pinned.contains(&used.0)
                            && let ValueDefKind::Inst(used_iid) =
                                &body.values[used.0 as usize].kind
                            && !body.values[used.0 as usize].is_removed()
                            && body.instructions[used_iid.0 as usize]
                                .kind
                                .effect()
                                .removable_when_unused()
                        {
                            worklist.push(*used);
                        }
                    }
                }
            });

            // Retire Value, unhook instruction from its block, and blank
            // the dead instruction's operands. The instruction stays in
            // `body.instructions` (InstIds are stable indices), but its
            // operands must not reference Values we may retire later in
            // this same pass — validation walks every entry in the flat
            // vec, including unhooked ones.
            body.values[v.0 as usize] = ValueDef::removed();
            if let Some(&bi) = inst_block.get(&iid.0) {
                body.blocks[bi].instructions.retain(|i| i.0 != iid.0);
            }
            walk_inst_operands_mut(&mut body.instructions[iid.0 as usize].kind, &mut |op| {
                *op = Operand::Const(Constant::Undefined);
            });
            removed_any = true;
        }

        Ok(removed_any)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Terminator,
        ValueDefKind,
    };

    #[test]
    fn removes_unused_pure_inst() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        // Pure unused alloc.
        let _ = b.add_inst(InstKind::AllocObject).unwrap();
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(DcePass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        assert!(
            body.blocks[entry.0 as usize].instructions.is_empty(),
            "unused AllocObject should be DCE'd"
        );
        assert!(matches!(
            body.values[0].kind,
            ValueDefKind::Removed
        ));
    }

    #[test]
    fn keeps_throwing_inst_even_when_unused() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        // AddGeneric may throw — must not be removed.
        let _ = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Number(1.0)),
                rhs: Operand::Const(Constant::Number(2.0)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(DcePass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        assert_eq!(
            body.blocks[entry.0 as usize].instructions.len(),
            1,
            "may-throw inst must be kept"
        );
    }

    #[test]
    fn cascades_chain_of_pure_uses() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let a = b.add_inst(InstKind::AllocObject).unwrap();
        let _ = b
            .add_inst(InstKind::Typeof {
                val: Operand::Value(a),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(DcePass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        assert!(
            body.blocks[entry.0 as usize].instructions.is_empty(),
            "Typeof and AllocObject should both DCE in one pass"
        );
    }
}
