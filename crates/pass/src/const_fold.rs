//! Constant folding pass — the first SSA optimization pass.
//!
//! Walks every instruction in a function. For each instruction whose
//! `try_const_fold` returns `Some(c)`, replaces all uses of the
//! instruction's result Value with `Operand::Const(c)`, retires the
//! Value (`ValueDefKind::Removed`), and unhooks the instruction from
//! its block's instruction list. The instruction itself stays in
//! `LoweredBody::instructions` (InstIds are stable indices); the
//! retired Value tells the emit-time name-allocation walker to skip
//! it, so no dead `let __vN = 1 + 2;` line surfaces in the output.
//!
//! What the pass does NOT do:
//! - Run a use-list / def-use index. `replace_value_uses` linear-scans;
//!   acceptable at corpus scale and falls within the YAGNI rule (build
//!   the index when a pass profiles slow).
//! - Touch CFG, terminators, phi nodes, or anything mutable beyond
//!   instruction operands.
//! - Constant-fold ops that may throw (AddGeneric, loose Eq, heap
//!   reads, calls). `try_const_fold` rejects those by construction.
//! - Identity simplification (`x + 0`, `x * 1`, `x === x`). Natural
//!   follow-up; do once const folding shakes out the API.

use ir::error::Error;
use ir::ssa::{
    Constant, FuncId, InstId, Module, Operand, ValueDef, find_value_for_inst,
    try_const_fold_in_module,
};

use crate::FunctionPass;

pub struct ConstFoldPass;

impl FunctionPass for ConstFoldPass {
    fn name(&self) -> &'static str {
        "const_fold"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        if module.functions[fid.0 as usize].is_pass_through() {
            return Ok(false);
        }

        // Two phases: read-only walk produces (block_index, inst_id, kind)
        // candidates, then we hand each kind to the module-aware folder
        // (which may need to intern strings for ConcatStr) and apply the
        // resulting Constants. Cloning the InstKind is cheap — operands
        // are 16-byte enums and the candidate set is the foldable subset
        // only.
        let mut candidates: Vec<(usize, InstId, ir::ssa::InstKind)> = Vec::new();
        {
            let body = module.functions[fid.0 as usize].lowered();
            for (bi, blk) in body.blocks.iter().enumerate() {
                for &iid in &blk.instructions {
                    let kind = &body.instructions[iid.0 as usize].kind;
                    candidates.push((bi, iid, kind.clone()));
                }
            }
        }

        let mut to_fold: Vec<(usize, InstId, Constant)> = Vec::new();
        for (bi, iid, kind) in candidates {
            if let Some(c) = try_const_fold_in_module(module, &kind) {
                to_fold.push((bi, iid, c));
            }
        }

        if to_fold.is_empty() {
            return Ok(false);
        }

        let body = module.functions[fid.0 as usize].lowered_mut();
        for (bi, iid, c) in &to_fold {
            let v = find_value_for_inst(body, *iid);
            body.replace_value_uses(v, Operand::Const(*c));
            body.values[v.0 as usize] = ValueDef::removed();
            let blk = &mut body.blocks[*bi];
            blk.instructions.retain(|i| i != iid);
        }

        Ok(true)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBody, FunctionBuilder, InstKind, Module, Operand,
        PassThroughBody, Terminator, ValueDefKind,
    };

    /// Build a tiny module containing a function whose body is one block:
    ///   v0 = AddInt32 1, 2
    ///   ret v0
    fn module_with_addint32() -> Module {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Const(Constant::Int32(1)),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .expect("AddInt32 produces a Value");
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(v)) });
        b.finish();
        module
    }

    #[test]
    fn folds_add_int32_constants() {
        let mut module = module_with_addint32();
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ConstFoldPass));
        let iters = pm.run(&mut module, FuncId(0)).expect("pass succeeds");
        // First iter folds (changed=true), second confirms fixed point
        // (changed=false) → 2 iters total.
        assert_eq!(iters, 2);

        let f = &module.functions[0];
        let body = f.lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(3))),
            } => {}
            other => panic!("expected ret Const(Int32(3)), got {other:?}"),
        }

        // Folded AddInt32 should be unhooked from block.instructions.
        let blk = &body.blocks[entry.0 as usize];
        for &iid in &blk.instructions {
            let kind = &body.instructions[iid.0 as usize].kind;
            assert!(!matches!(kind, InstKind::AddInt32 { .. }));
        }

        // Folded Value should be retired.
        assert!(matches!(body.values[0].kind, ValueDefKind::Removed));
    }

    #[test]
    fn no_fold_when_operand_not_constant() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b
            .add_inst(InstKind::AllocObject)
            .expect("AllocObject produces a Value");
        let result = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(alloc),
                rhs: Operand::Const(Constant::Number(1.0)),
            })
            .expect("AddGeneric produces a Value");
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(result)) });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ConstFoldPass));
        // ConstFoldPass returns changed=false on AddGeneric (out of
        // try_const_fold's scope); pipeline converges in 1 iter.
        let iters = pm.run(&mut module, FuncId(0)).expect("pass succeeds");
        assert_eq!(iters, 1);
    }

    #[test]
    fn folds_strict_eq_constants() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let cmp = b
            .add_inst(InstKind::StrictEq {
                lhs: Operand::Const(Constant::Number(1.0)),
                rhs: Operand::Const(Constant::Number(1.0)),
            })
            .expect("StrictEq produces a Value");
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(cmp)) });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ConstFoldPass));
        pm.run(&mut module, FuncId(0)).expect("pass succeeds");

        let f = &module.functions[0];
        let body = f.lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Bool(true))),
            } => {}
            other => panic!("expected ret Const(Bool(true)), got {other:?}"),
        }
    }

    #[test]
    fn pass_through_function_skipped() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();
        // Replace with PassThrough body — exercises the
        // is_pass_through() short-circuit.
        module.functions[0].body = FunctionBody::PassThrough(PassThroughBody {
            source: "function f(){}".to_string(),
            param_names: vec![],
            function_name: None,
            is_async: false,
            is_generator: false,
            is_arrow: false,
            body_is_expression: false,
        });

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ConstFoldPass));
        let iters = pm.run(&mut module, FuncId(0)).expect("pass succeeds");
        assert_eq!(iters, 1, "PassThrough function should not iterate");
    }

    #[test]
    fn folds_concat_str_constants() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let a = b.intern("hello ");
        let b_id = b.intern("world");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::ConcatStr {
                lhs: Operand::Const(Constant::String(a)),
                rhs: Operand::Const(Constant::String(b_id)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(v)) });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ConstFoldPass));
        pm.run(&mut module, FuncId(0)).expect("pass succeeds");

        let body = module.functions[0].lowered();
        let entry = body.entry;
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::String(id))),
            } => {
                assert_eq!(module.strings.resolve(id), "hello world");
            }
            other => panic!("expected ret Const(String('hello world')), got {other:?}"),
        }
    }

    #[test]
    fn folds_bitwise_int32_constants() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::BitAnd {
                lhs: Operand::Const(Constant::Int32(0xff)),
                rhs: Operand::Const(Constant::Int32(0x0f)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(v)) });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(ConstFoldPass));
        pm.run(&mut module, FuncId(0)).expect("pass succeeds");

        let body = module.functions[0].lowered();
        let entry = body.entry;
        assert!(matches!(
            body.blocks[entry.0 as usize].terminator,
            Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(0x0f))),
            }
        ));
    }
}
