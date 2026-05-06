//! Typed-arithmetic refinement pass.
//!
//! Lowering emits the `*Generic` arithmetic kinds unconditionally because
//! source-level `+`/`-`/`*`/etc. can throw on `valueOf` / `toString` and
//! `+` doubles as string concat. This pass walks every instruction and,
//! when the static type lattice (`ssa::value_type`) proves both operands
//! are Number, Int32, or (for `+`) String, rewrites the InstKind to the
//! typed variant. That unblocks `ConstFoldPass`, which only folds typed
//! arithmetic.
//!
//! No CFG changes, no value retirement — only `InstKind` mutation in
//! place. Runs before `ConstFoldPass` in the pipeline; the manager's
//! fixed-point loop handles the refine→fold→refine cascade.

use ir::error::Error;
use ir::ssa::{FuncId, InstKind, Module, Operand, ValueType, operand_type};

use crate::FunctionPass;

enum ArithOp {
    Sub,
    Mul,
}

fn is_numeric(t: ValueType) -> bool {
    matches!(t, ValueType::Number | ValueType::Int32)
}

fn refine_add(lt: ValueType, rt: ValueType, lhs: Operand, rhs: Operand) -> Option<InstKind> {
    if lt == ValueType::Int32 && rt == ValueType::Int32 {
        return Some(InstKind::AddInt32 { lhs, rhs });
    }
    if is_numeric(lt) && is_numeric(rt) {
        // At least one side is Number (or both Int32 handled above) —
        // result is Number; safe because neither side can be String,
        // so this is arithmetic +, not concat.
        return Some(InstKind::AddNumber { lhs, rhs });
    }
    if lt == ValueType::String && rt == ValueType::String {
        return Some(InstKind::ConcatStr { lhs, rhs });
    }
    None
}

fn refine_arith_pair(
    lt: ValueType,
    rt: ValueType,
    lhs: Operand,
    rhs: Operand,
    op: ArithOp,
) -> Option<InstKind> {
    if lt == ValueType::Int32 && rt == ValueType::Int32 {
        return Some(match op {
            ArithOp::Sub => InstKind::SubInt32 { lhs, rhs },
            ArithOp::Mul => InstKind::MulInt32 { lhs, rhs },
        });
    }
    if is_numeric(lt) && is_numeric(rt) {
        return Some(match op {
            ArithOp::Sub => InstKind::SubNumber { lhs, rhs },
            ArithOp::Mul => InstKind::MulNumber { lhs, rhs },
        });
    }
    None
}

pub struct RefineArithPass;

impl FunctionPass for RefineArithPass {
    fn name(&self) -> &'static str {
        "refine_arith"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let body = func.lowered_mut();

        // Snapshot rewrites from a read-only walk, then apply. Avoids
        // borrowing instructions mutably while inspecting other values.
        let mut rewrites: Vec<(usize, InstKind)> = Vec::new();
        for (idx, inst) in body.instructions.iter().enumerate() {
            let lt = |op: &Operand| operand_type(body, op);
            let new_kind = match &inst.kind {
                InstKind::AddGeneric { lhs, rhs } => {
                    refine_add(lt(lhs), lt(rhs), *lhs, *rhs)
                }
                InstKind::SubGeneric { lhs, rhs } => {
                    refine_arith_pair(lt(lhs), lt(rhs), *lhs, *rhs, ArithOp::Sub)
                }
                InstKind::MulGeneric { lhs, rhs } => {
                    refine_arith_pair(lt(lhs), lt(rhs), *lhs, *rhs, ArithOp::Mul)
                }
                _ => None,
            };
            if let Some(k) = new_kind {
                rewrites.push((idx, k));
            }
        }

        if rewrites.is_empty() {
            return Ok(false);
        }
        for (idx, k) in rewrites {
            body.instructions[idx].kind = k;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Terminator,
    };

    fn build_one_block_module(
        op: impl FnOnce(&mut FunctionBuilder) -> Operand,
    ) -> Module {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let result = op(&mut b);
        b.set_terminator(Terminator::Return { val: Some(result) });
        b.finish();
        module
    }

    fn entry_inst_kinds(module: &Module) -> Vec<InstKind> {
        let body = module.functions[0].lowered();
        let entry = body.entry;
        body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .map(|iid| body.instructions[iid.0 as usize].kind.clone())
            .collect()
    }

    #[test]
    fn refines_number_number_add_to_add_number() {
        let mut module = build_one_block_module(|b| {
            let v = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Const(Constant::Number(1.5)),
                    rhs: Operand::Const(Constant::Number(2.5)),
                })
                .unwrap();
            Operand::Value(v)
        });
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineArithPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let kinds = entry_inst_kinds(&module);
        assert!(matches!(kinds[0], InstKind::AddNumber { .. }), "{kinds:?}");
    }

    #[test]
    fn refines_int32_int32_add_to_add_int32() {
        let mut module = build_one_block_module(|b| {
            // Force Int32 typing via BitOr (always Int32).
            let l = b
                .add_inst(InstKind::BitOr {
                    lhs: Operand::Const(Constant::Int32(0)),
                    rhs: Operand::Const(Constant::Int32(5)),
                })
                .unwrap();
            let v = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(l),
                    rhs: Operand::Const(Constant::Int32(7)),
                })
                .unwrap();
            Operand::Value(v)
        });
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineArithPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let kinds = entry_inst_kinds(&module);
        assert!(
            matches!(kinds[1], InstKind::AddInt32 { .. }),
            "expected AddInt32, got {kinds:?}"
        );
    }

    #[test]
    fn refines_string_string_add_to_concat_str() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let a = b.intern("hello");
        let bb = b.intern("world");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::String(a)),
                rhs: Operand::Const(Constant::String(bb)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(v)) });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineArithPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let kinds = entry_inst_kinds(&module);
        assert!(
            matches!(kinds[0], InstKind::ConcatStr { .. }),
            "expected ConcatStr, got {kinds:?}"
        );
    }

    #[test]
    fn refines_int32_plus_number_to_add_number() {
        let mut module = build_one_block_module(|b| {
            let l = b
                .add_inst(InstKind::BitOr {
                    lhs: Operand::Const(Constant::Int32(0)),
                    rhs: Operand::Const(Constant::Int32(7)),
                })
                .unwrap();
            let v = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(l),
                    rhs: Operand::Const(Constant::Number(5.0)),
                })
                .unwrap();
            Operand::Value(v)
        });
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineArithPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let kinds = entry_inst_kinds(&module);
        assert!(
            matches!(kinds[1], InstKind::AddNumber { .. }),
            "Int32+Number should refine to AddNumber, got {kinds:?}"
        );
    }

    #[test]
    fn no_refine_when_one_side_unknown() {
        let mut module = build_one_block_module(|b| {
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            let v = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(alloc),
                    rhs: Operand::Const(Constant::Number(1.0)),
                })
                .unwrap();
            Operand::Value(v)
        });
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineArithPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let kinds = entry_inst_kinds(&module);
        assert!(
            matches!(kinds[1], InstKind::AddGeneric { .. }),
            "should stay generic when one side is opaque"
        );
    }

    #[test]
    fn no_refine_for_string_plus_number() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let s = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::String(s)),
                rhs: Operand::Const(Constant::Number(1.0)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: Some(Operand::Value(v)) });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineArithPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let kinds = entry_inst_kinds(&module);
        assert!(
            matches!(kinds[0], InstKind::AddGeneric { .. }),
            "string+number must stay generic — coercion semantics"
        );
    }
}
