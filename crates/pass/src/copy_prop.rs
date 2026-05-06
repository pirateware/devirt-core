//! Copy propagation via trivial block-param elimination.
//!
//! Block params that receive the same `Operand` from every predecessor
//! (modulo self-references) are noise — every use can be replaced with
//! that single Operand. This is the classic SSA "trivial phi" rule.
//!
//! BranchFoldPass turns CondBranch into Branch, which can collapse a
//! join block's two-pred edges to one — making its params trivial. This
//! pass cleans those up; PassManager's fixed-point loop handles cascade
//! (one removal can make another param trivial).
//!
//! Block params are synthetic intermediates with no `source_name`, so
//! they can never be in `Function.captured_by_passthrough` (that set
//! requires a source name to round-trip). Safe to retire without any
//! Function-level metadata bookkeeping.

use ir::error::Error;
use ir::ssa::{FuncId, Module, Value, ValueDefKind};

use crate::FunctionPass;

pub struct CopyPropPass;

impl FunctionPass for CopyPropPass {
    fn name(&self) -> &'static str {
        "copy_prop"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let body = func.lowered_mut();

        // Walk every block-param Value once. The IR helper handles all
        // the surgery (replace uses, drop param, strip predecessor args,
        // retire Value, shift trailing param indices). After a removal
        // OTHER params in the same block may have become trivial — we
        // re-scan from the start in that case until no more progress.
        let mut changed_overall = false;
        loop {
            let mut changed_this_round = false;
            // Snapshot the Value-id range; new Values aren't created by
            // this pass, so a fresh range each iteration is fine.
            let n = body.values.len();
            for vi in 0..n {
                if !matches!(body.values[vi].kind, ValueDefKind::BlockParam { .. }) {
                    continue;
                }
                if body.try_remove_trivial_block_param(Value(vi as u32)).is_some() {
                    changed_this_round = true;
                }
            }
            if !changed_this_round {
                break;
            }
            changed_overall = true;
        }
        Ok(changed_overall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Terminator, ValueDef,
    };

    /// Build a function with a join block that takes one param. Both
    /// predecessors pass the same constant — the param should be retired.
    #[test]
    fn folds_join_with_identical_const_args() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb_join = b.new_block();
        let bb0 = b.new_block();
        // bb_join takes one param; both branches feed Const(7).
        let join_param = {
            let body = b.body_mut();
            body.values.push(ValueDef::block_param(bb_join, 0));
            let v = Value(body.values.len() as u32 - 1);
            body.blocks[bb_join.0 as usize].params.push(v);
            v
        };
        b.switch_to_block(bb_join);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(join_param)),
        });
        b.switch_to_block(bb_then);
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![Operand::Const(Constant::Number(7.0))],
        });
        b.switch_to_block(bb_else);
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![Operand::Const(Constant::Number(7.0))],
        });
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Const(Constant::Bool(true)),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(CopyPropPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        assert!(
            body.blocks[bb_join.0 as usize].params.is_empty(),
            "join param should be retired"
        );
        match &body.blocks[bb_join.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 7.0),
            other => panic!("expected ret 7.0, got {other:?}"),
        }
    }

    /// Two preds feed *different* constants — param is real, not trivial,
    /// must be kept.
    #[test]
    fn keeps_param_when_preds_disagree() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb_join = b.new_block();
        let bb0 = b.new_block();
        let join_param = {
            let body = b.body_mut();
            body.values.push(ValueDef::block_param(bb_join, 0));
            let v = Value(body.values.len() as u32 - 1);
            body.blocks[bb_join.0 as usize].params.push(v);
            v
        };
        b.switch_to_block(bb_join);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(join_param)),
        });
        b.switch_to_block(bb_then);
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![Operand::Const(Constant::Number(7.0))],
        });
        b.switch_to_block(bb_else);
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![Operand::Const(Constant::Number(8.0))],
        });
        b.switch_to_block(bb0);
        // Use a non-const condition so BranchFold wouldn't simplify even
        // if it ran here. This pass should still leave the param alone.
        let cv = b.add_inst(InstKind::AllocObject).unwrap();
        let cond_v = b
            .add_inst(InstKind::ToBool {
                val: Operand::Value(cv),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cond_v),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(CopyPropPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        assert_eq!(
            body.blocks[bb_join.0 as usize].params.len(),
            1,
            "param must be kept when preds disagree"
        );
    }

    /// Self-referencing back-edge ("phi(x, self)") with one real
    /// incoming → trivial = the real incoming.
    #[test]
    fn folds_self_referential_loop_carry() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_loop = b.new_block();
        let bb0 = b.new_block();
        let loop_param = {
            let body = b.body_mut();
            body.values.push(ValueDef::block_param(bb_loop, 0));
            let v = Value(body.values.len() as u32 - 1);
            body.blocks[bb_loop.0 as usize].params.push(v);
            v
        };
        // bb_loop: branch back to itself with arg = self (loop carry).
        b.switch_to_block(bb_loop);
        b.set_terminator(Terminator::Branch {
            target: bb_loop,
            args: vec![Operand::Value(loop_param)],
        });
        // Entry feeds Const(42) once.
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Branch {
            target: bb_loop,
            args: vec![Operand::Const(Constant::Number(42.0))],
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0].lowered_mut().rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(CopyPropPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        assert!(
            body.blocks[bb_loop.0 as usize].params.is_empty(),
            "self-loop param with one real incoming should fold"
        );
    }
}
