//! Branch folding — collapse conditional terminators with constant
//! conditions into unconditional branches.
//!
//! ConstFoldPass produces `Const::Bool` values; without this pass they
//! sit as `CondBranch` conditions and the dead branch survives. Switch
//! terminators with a constant `val` similarly collapse to whichever
//! case matches (or `default`).
//!
//! Mutations are terminator-only: rewrite `CondBranch{Const::Bool(b)}`
//! to `Branch{taken_target}`, rewrite `Switch{Const(k)}` to the matching
//! case's branch (or default). After any change, predecessor lists are
//! rebuilt via `LoweredBody::rebuild_predecessors`. No CFG block removal
//! — unreachable blocks survive but lose their predecessors; the emit
//! pipeline already tolerates them via reverse-postorder traversal.

use ir::error::Error;
use ir::ssa::{Constant, FuncId, Module, Operand, Terminator};

use crate::FunctionPass;

pub struct BranchFoldPass;

impl FunctionPass for BranchFoldPass {
    fn name(&self) -> &'static str {
        "branch_fold"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let body = func.lowered_mut();

        let mut changed = false;
        for blk in body.blocks.iter_mut() {
            let new_term = match &blk.terminator {
                Terminator::CondBranch {
                    cond: Operand::Const(Constant::Bool(b)),
                    then_b,
                    then_args,
                    else_b,
                    else_args,
                } => {
                    if *b {
                        Some(Terminator::Branch {
                            target: *then_b,
                            args: then_args.clone(),
                        })
                    } else {
                        Some(Terminator::Branch {
                            target: *else_b,
                            args: else_args.clone(),
                        })
                    }
                }
                Terminator::Switch {
                    val: Operand::Const(k),
                    cases,
                    default,
                    default_args,
                    ..
                } => {
                    let matched = cases.iter().find(|c| const_eq(&c.key, k));
                    Some(match matched {
                        Some(case) => Terminator::Branch {
                            target: case.target,
                            args: case.args.clone(),
                        },
                        None => Terminator::Branch {
                            target: *default,
                            args: default_args.clone(),
                        },
                    })
                }
                _ => None,
            };
            if let Some(t) = new_term {
                blk.terminator = t;
                changed = true;
            }
        }

        if changed {
            body.rebuild_predecessors();
        }
        Ok(changed)
    }
}

/// Strict equality on `Constant` for switch-case matching. Mirrors
/// JS `===` semantics restricted to the constant primitive types
/// SwitchCase keys carry: Number/Int32/String/Bool/Null/Undefined.
/// NaN is unequal to itself.
fn const_eq(a: &Constant, b: &Constant) -> bool {
    use Constant::*;
    match (a, b) {
        (Number(x), Number(y)) => x == y && !x.is_nan(),
        (Int32(x), Int32(y)) => x == y,
        (Number(x), Int32(y)) | (Int32(y), Number(x)) => *x == (*y as f64) && !x.is_nan(),
        (String(x), String(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (Null, Null) | (Undefined, Undefined) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        BlockId, Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, SwitchCase,
        Terminator,
    };

    #[test]
    fn folds_cond_branch_true_to_then() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb_then);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Const(Constant::Number(1.0))),
        });
        b.switch_to_block(bb_else);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Const(Constant::Number(2.0))),
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
        // Entry is the last-created block (bb0). Re-point entry.
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0]
            .lowered_mut()
            .rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(BranchFoldPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        match &body.blocks[bb0.0 as usize].terminator {
            Terminator::Branch { target, .. } => assert_eq!(*target, bb_then),
            other => panic!("expected Branch to then, got {other:?}"),
        }
        assert!(
            body.blocks[bb_else.0 as usize].predecessors.is_empty(),
            "else block should be unreachable after fold"
        );
    }

    #[test]
    fn folds_cond_branch_false_to_else() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb_then);
        b.set_terminator(Terminator::Return { val: None });
        b.switch_to_block(bb_else);
        b.set_terminator(Terminator::Return { val: None });
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Const(Constant::Bool(false)),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0]
            .lowered_mut()
            .rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(BranchFoldPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        match &body.blocks[bb0.0 as usize].terminator {
            Terminator::Branch { target, .. } => assert_eq!(*target, bb_else),
            other => panic!("expected Branch to else, got {other:?}"),
        }
    }

    #[test]
    fn no_fold_when_cond_is_value() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb_then);
        b.set_terminator(Terminator::Return { val: None });
        b.switch_to_block(bb_else);
        b.set_terminator(Terminator::Return { val: None });
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::AllocObject)
            .unwrap();
        let _ = v;
        let cond_v = b
            .add_inst(InstKind::ToBool {
                val: Operand::Value(v),
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
        module.functions[0]
            .lowered_mut()
            .rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(BranchFoldPass));
        let iters = pm.run(&mut module, FuncId(0)).unwrap();
        assert_eq!(iters, 1, "no change → one iter and exit");
    }

    #[test]
    fn folds_switch_to_matching_case() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_a = b.new_block();
        let bb_b = b.new_block();
        let bb_def = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb_a);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Const(Constant::Number(10.0))),
        });
        b.switch_to_block(bb_b);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Const(Constant::Number(20.0))),
        });
        b.switch_to_block(bb_def);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Const(Constant::Number(99.0))),
        });
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Switch {
            val: Operand::Const(Constant::Number(2.0)),
            cases: vec![
                SwitchCase {
                    key: Constant::Number(1.0),
                    target: bb_a,
                    args: vec![],
                },
                SwitchCase {
                    key: Constant::Number(2.0),
                    target: bb_b,
                    args: vec![],
                },
            ],
            default: bb_def,
            default_args: vec![],
            join: BlockId(0),
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0]
            .lowered_mut()
            .rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(BranchFoldPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        match &body.blocks[bb0.0 as usize].terminator {
            Terminator::Branch { target, .. } => assert_eq!(*target, bb_b),
            other => panic!("expected Branch to bb_b, got {other:?}"),
        }
    }

    #[test]
    fn folds_switch_to_default_when_no_match() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_a = b.new_block();
        let bb_def = b.new_block();
        let bb0 = b.new_block();
        b.switch_to_block(bb_a);
        b.set_terminator(Terminator::Return { val: None });
        b.switch_to_block(bb_def);
        b.set_terminator(Terminator::Return { val: None });
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Switch {
            val: Operand::Const(Constant::Number(99.0)),
            cases: vec![SwitchCase {
                key: Constant::Number(1.0),
                target: bb_a,
                args: vec![],
            }],
            default: bb_def,
            default_args: vec![],
            join: BlockId(0),
        });
        b.finish();
        module.functions[0].lowered_mut().entry = bb0;
        module.functions[0]
            .lowered_mut()
            .rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(BranchFoldPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        match &body.blocks[bb0.0 as usize].terminator {
            Terminator::Branch { target, .. } => assert_eq!(*target, bb_def),
            other => panic!("expected Branch to default, got {other:?}"),
        }
    }
}
