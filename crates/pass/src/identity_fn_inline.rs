//! Identity-function inlining — the first inter-procedural pass.
//!
//! Many obfuscators wrap real calls in trivial selectors:
//!   `function _0x1(x) { return x; }`        // identity
//!   `function _0x2(x, y) { return x; }`     // first-arg selector
//!   `function _0x3() { return 42; }`        // constant
//!
//! Each such wrapper means every call site is `_0xN(actual_thing)` and
//! the real expression hides behind one indirection. This pass detects
//! the trivial-return shape per function, then walks every other
//! function rewriting `Call(Const(Func(fid)), [args])` into the
//! resolved result — `args[i]` for selectors, the constant for
//! constant functions — when `fid` matches.
//!
//! Out of scope (today):
//! - `CallMethod` (receiver matters) and `CallSpread` (arg arity is
//!   dynamic). Both produce `Call`-shape instructions only when the
//!   frontend can prove plain-call semantics; today they don't.
//! - `Construct` (`new id()` returns the implicit `this`, not the arg).
//! - Functions whose body is multi-block but trivially equivalent to a
//!   selector (would need a CFG-level normalizer first).
//! - `Const(This)` and `Const(GlobalThis)` returns — context-dependent
//!   at the call site, unsafe to clone.
//!
//! Soundness: replacing `Call(id, [x])` with `x` drops the Call inst's
//! `OPAQUE` effect. Sound because the identity body itself has no
//! side effects (its body is empty, terminator is `Return(arg)`); the
//! arg-evaluation effects already happened before the Call regardless
//! of whether we elide it. Receiver `this` is not bound for a plain
//! `Call` (no method-call shape) so identity functions referencing
//! `this` don't fit this pattern and aren't matched.

use std::collections::HashMap;

use ir::error::Error;
use ir::ssa::{
    Constant, FunctionBody, InstKind, Module, Operand, Terminator, ValueDef, ValueDefKind,
    find_value_for_inst, walk_inst_operands_mut,
};

use crate::ModulePass;

/// Trivial-return shape. `Selector(i)` returns `params[i]` (i-th
/// argument). `Const(c)` returns a clonable constant.
#[derive(Clone, Copy, Debug)]
enum Summary {
    Selector(usize),
    Const(Constant),
}

pub struct IdentityFnInlinePass;

impl ModulePass for IdentityFnInlinePass {
    fn name(&self) -> &'static str {
        "identity_fn_inline"
    }

    fn run(&mut self, module: &mut Module) -> Result<bool, Error> {
        // Phase 1: summarize every Lowered function whose body is one
        // block, zero instructions, and a `Return(Some(operand))` whose
        // operand is either `Value(arg_i)` or a clonable `Const`.
        let mut summary: HashMap<u32, Summary> = HashMap::new();
        for (i, func) in module.functions.iter().enumerate() {
            let body = match &func.body {
                FunctionBody::Lowered(b) => b,
                FunctionBody::PassThrough(_) => continue,
            };
            if body.blocks.len() != 1 {
                continue;
            }
            let entry = &body.blocks[body.entry.0 as usize];
            if !entry.instructions.is_empty() || !entry.params.is_empty() {
                continue;
            }
            let val_op = match &entry.terminator {
                Terminator::Return { val: Some(op) } => *op,
                _ => continue,
            };
            let s = match val_op {
                Operand::Const(c) if is_clonable_const(&c) => Summary::Const(c),
                Operand::Value(v) => match body.values[v.0 as usize].kind {
                    ValueDefKind::Arg(idx) => Summary::Selector(idx as usize),
                    _ => continue,
                },
                _ => continue,
            };
            summary.insert(i as u32, s);
        }

        if summary.is_empty() {
            return Ok(false);
        }

        // Phase 2: rewrite Call sites in every other function.
        let mut any_changed = false;
        let n_funcs = module.functions.len();
        for fid_idx in 0..n_funcs {
            let body = match &mut module.functions[fid_idx].body {
                FunctionBody::Lowered(b) => b,
                FunctionBody::PassThrough(_) => continue,
            };

            // Collect rewrites: (block_index, inst_id, replacement_operand).
            // We need to clone args out of the Call before mutating, so do
            // it in a separate read-only walk first.
            let mut rewrites: Vec<(usize, u32, Operand)> = Vec::new();
            for (bi, blk) in body.blocks.iter().enumerate() {
                for &iid in &blk.instructions {
                    let inst = &body.instructions[iid.0 as usize];
                    if let InstKind::Call { callee, args } = &inst.kind
                        && let Operand::Const(Constant::Func(fid)) = callee
                        && let Some(s) = summary.get(&fid.0)
                    {
                        let replacement = match s {
                            Summary::Selector(i) => args
                                .get(*i)
                                .copied()
                                .unwrap_or(Operand::Const(Constant::Undefined)),
                            Summary::Const(c) => Operand::Const(*c),
                        };
                        rewrites.push((bi, iid.0, replacement));
                    }
                }
            }

            if rewrites.is_empty() {
                continue;
            }

            for (bi, iid_u32, replacement) in &rewrites {
                let result_v = find_value_for_inst(body, ir::ssa::InstId(*iid_u32));
                body.replace_value_uses(result_v, *replacement);
                body.values[result_v.0 as usize] = ValueDef::removed();
                let blk = &mut body.blocks[*bi];
                blk.instructions.retain(|i| i.0 != *iid_u32);
                // Blank operands on the now-dead instruction so validation's
                // full-vec walk doesn't see references to the retired Value
                // (same pattern as DcePass).
                walk_inst_operands_mut(
                    &mut body.instructions[*iid_u32 as usize].kind,
                    &mut |op| {
                        *op = Operand::Const(Constant::Undefined);
                    },
                );
                any_changed = true;
            }
        }
        Ok(any_changed)
    }
}

fn is_clonable_const(c: &Constant) -> bool {
    use Constant::*;
    matches!(
        c,
        Number(_) | Int32(_) | Bool(_) | String(_) | Null | Undefined | Func(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Terminator,
    };

    /// `function id(x) { return x; }` then `function f(y) { return id(y); }`
    /// → after pass, f's Call is gone and ret references y directly.
    #[test]
    fn inlines_identity_call() {
        let mut module = Module::new();
        // id(x) { return x; }
        {
            let mut b = FunctionBuilder::new(&mut module, "id", 1);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let arg0 = b.body().params[0];
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(arg0)),
            });
            b.finish();
        }
        // f(y) { return id(y); }
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let arg0 = b.body().params[0];
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Value(arg0)],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(IdentityFnInlinePass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        // No Call left.
        assert!(
            !f_body.blocks[entry.0 as usize]
                .instructions
                .iter()
                .any(|iid| matches!(
                    f_body.instructions[iid.0 as usize].kind,
                    InstKind::Call { .. }
                )),
            "Call should have been retired"
        );
        // Return now references the arg directly (or whatever the
        // identity produces).
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Value(v)),
            } => {
                assert!(
                    matches!(f_body.values[v.0 as usize].kind, ValueDefKind::Arg(0)),
                    "return val should resolve to arg 0"
                );
            }
            other => panic!("expected ret of arg-Value, got {other:?}"),
        }
    }

    /// `function selA(a, b) { return a; }` then `f(x, y) { return selA(x, y); }`
    /// → f's Call is gone, ret references x.
    #[test]
    fn inlines_first_arg_selector() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "selA", 2);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let a = b.body().params[0];
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(a)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 2);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let x = b.body().params[0];
            let y = b.body().params[1];
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Value(x), Operand::Value(y)],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(IdentityFnInlinePass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Value(v)),
            } => {
                assert!(matches!(
                    f_body.values[v.0 as usize].kind,
                    ValueDefKind::Arg(0)
                ));
            }
            other => panic!("expected ret of arg(0), got {other:?}"),
        }
    }

    /// `function k() { return 42; }` then call site → folded to const.
    #[test]
    fn inlines_constant_function() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "k", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Number(42.0))),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(IdentityFnInlinePass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 42.0),
            other => panic!("expected ret 42, got {other:?}"),
        }
    }

    /// Body has an instruction (not just terminator) — not identity.
    #[test]
    fn no_inline_when_body_is_nontrivial() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let a = b.body().params[0];
            // Adds 1 — not identity.
            let v = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(a),
                    rhs: Operand::Const(Constant::Number(1.0)),
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(v)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "g", 1);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let x = b.body().params[0];
            let _ = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Value(x)],
                })
                .unwrap();
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(IdentityFnInlinePass));
        pm.run_module(&mut module).unwrap();

        let g_body = module.functions[1].lowered();
        let entry = g_body.entry;
        assert!(
            g_body.blocks[entry.0 as usize]
                .instructions
                .iter()
                .any(|iid| matches!(
                    g_body.instructions[iid.0 as usize].kind,
                    InstKind::Call { .. }
                )),
            "Call to non-identity function must be preserved"
        );
    }
}
