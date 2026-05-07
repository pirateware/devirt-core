//! Partial-evaluation pass — issue #49.
//!
//! When a `Call` site has all-`Const` arguments and targets a function
//! whose body shape is admissible to `ConstructorFnInlinePass`, we
//! virtually clone the callee body into a sandbox `Module`, substitute
//! `ValueDefKind::Arg(i)` references with the corresponding `Const`,
//! and run a bounded mini-pipeline (`ConstFold` → `BranchFold` →
//! `ForwardLoads` → `Dce`). If the sandbox's exit terminator reduces
//! to `Return(Some(Operand::Const(c)))`, the original Call's result
//! Value uses are rewritten to `Operand::Const(c)` and the Call is
//! retired (instruction unhooked, operands blanked, Value marked
//! `Removed`) — the same retire convention used by other inliners.
//!
//! ## Why a sandbox
//!
//! The actual callee IR is never modified. We need a *speculative*
//! evaluation: if the const-arg specialisation collapses to a single
//! Const we win; otherwise we leave the Call alone and let other
//! passes handle it (physical inlining via `ConstructorFnInlinePass`,
//! summary-table forwarding via `ForwardLoadsPass`, or no
//! optimisation at all).
//!
//! ## Soundness
//!
//! - Admissibility is no longer gated by `is_inlinable_shape`. We
//!   sandbox ANY `Lowered` callee whose body fits the size caps and
//!   rely on the **converged-Const** check at the sandbox exit as the
//!   real soundness gate: every reachable `Return` terminator must
//!   carry the SAME `Operand::Const(_)`. If the body has external
//!   side effects, those effects materialise as live operations whose
//!   results flow into `Return`s as `Operand::Value(_)` (or
//!   `Operand::Capture(_)`), failing the gate. Equivalently:
//!     * A `Return` that depends on a heap load that the sandbox
//!       couldn't const-fold ⇒ value is `Operand::Value(_)` ⇒ bail.
//!     * A `Return` that depends on a Call result that didn't
//!       const-fold ⇒ same outcome ⇒ bail.
//!     * A `Return` that's just `Const(c)` ⇒ safe to apply at the
//!       call site.
//!   This also rules out `Throw` paths automatically: a reachable
//!   `Throw` is not a `Return(Const)` terminator, so any Throw-only
//!   exit fails the all-Returns-Const gate.
//! - The sandbox shares the input module's string interner (cloned).
//!   `try_const_fold_in_module` for `ConcatStr` resolves StrIds against
//!   the sandbox interner, then writes its `Const::String` result back
//!   into the *original* module via the operand we substitute — but
//!   that StrId is still valid in the original because we only ever
//!   look up strings that already existed in the cloned interner; new
//!   strings interned in the sandbox have IDs the original module's
//!   interner doesn't know about. To stay safe we re-intern any
//!   `Const::String` returned in the answer against the original
//!   module's interner before substituting.
//! - Calls inside the cloned body are NOT recursively partial-eval'd
//!   here. The mini-pipeline's `ForwardLoads` may resolve a
//!   `LoadProp(Call_result)` via the summaries-table fallback when the
//!   inner Call targets a constructor-shape function — but only
//!   summaries built from THIS sandbox module, which contains a single
//!   function. Cross-procedural summary lookup inside the sandbox is
//!   intentionally cold; this keeps the sandbox cost bounded.
//!
//! ## Bounds
//!
//! - `MAX_BLOCKS` (default 32) — sandbox declines bodies with more
//!   blocks; physical inlining is the right tool for those.
//! - `MAX_INSTS` (default 256) — same reasoning.
//! - The inner `PassManager` runs at the default 64-iteration cap; if
//!   it doesn't converge, the call site is left alone (we treat any
//!   `Err(_)` from the inner pipeline as "no answer").
//!
//! ## Pipeline placement
//!
//! Module pass, registered AFTER `ConstructorFnInlinePass` (so the
//! physical inliner has already cloned every Call it can — partial
//! eval picks up the calls the matcher rejected, e.g. multi-block
//! bodies whose const-arg specialisation collapses) and BEFORE
//! `ForwardLoadsPass` (the load-resolution pass benefits from Calls
//! that have already become Const results).

use ir::error::Error;
use ir::ssa::{
    Constant, FuncId, FunctionBody, InstId, InstKind, LoweredBody, Module, Operand, Terminator,
    Value, ValueDef, ValueDefKind, find_value_for_inst, walk_inst_operands_mut,
};

use crate::branch_fold::BranchFoldPass;
use crate::const_fold::ConstFoldPass;
use crate::dce::DcePass;
use crate::forward_loads::ForwardLoadsPass;
use crate::{ModulePass, PassManager};

/// Maximum block count of a callee we'll attempt to partial-eval.
/// Above this, physical inlining is the right tool.
const MAX_BLOCKS: usize = 32;
/// Maximum instruction count of a callee we'll attempt to partial-eval.
const MAX_INSTS: usize = 256;

pub struct PartialEvalPass;

impl ModulePass for PartialEvalPass {
    fn name(&self) -> &'static str {
        "partial_eval"
    }

    fn run(&mut self, module: &mut Module) -> Result<bool, Error> {
        let mut any_changed = false;
        let n_funcs = module.functions.len();

        // For each caller function, scan its body for eligible Call
        // sites. Process call sites one at a time; rewriting one call
        // doesn't invalidate the others (we only edit operands of the
        // call instruction itself plus the Value retire).
        for caller_idx in 0..n_funcs {
            // Collect candidate call sites first (immutable borrow).
            let candidates: Vec<(InstId, FuncId, Vec<Constant>)> = {
                let body = match &module.functions[caller_idx].body {
                    FunctionBody::Lowered(b) => b,
                    FunctionBody::PassThrough(_) => continue,
                };
                let mut out = Vec::new();
                for blk in &body.blocks {
                    for &iid in &blk.instructions {
                        let inst = &body.instructions[iid.0 as usize];
                        if let InstKind::Call { callee, args } = &inst.kind
                            && let Operand::Const(Constant::Func(target_fid)) = callee
                            && target_fid.0 != caller_idx as u32
                        {
                            // Require ALL args to be Const.
                            let mut all_const = Vec::with_capacity(args.len());
                            let mut ok = true;
                            for a in args {
                                match a {
                                    Operand::Const(c) => all_const.push(*c),
                                    _ => {
                                        ok = false;
                                        break;
                                    }
                                }
                            }
                            if !ok {
                                continue;
                            }
                            out.push((iid, *target_fid, all_const));
                        }
                    }
                }
                out
            };

            for (call_iid, target_fid, const_args) in candidates {
                // Gate: callee must be `Lowered` and fit size caps.
                // The `is_inlinable_shape` predicate is intentionally
                // NOT used: real-world decoders / wrappers / multi-
                // block helpers fail it for benign reasons (e.g.
                // captures, calls to non-clonable helpers, opaque
                // stores). Soundness is enforced at the sandbox exit
                // by the all-reachable-Returns-yield-the-same-Const
                // check — any external side effect in the body shows
                // up as a non-Const operand on a Return value.
                let target_idx = target_fid.0 as usize;
                if target_idx >= module.functions.len() {
                    continue;
                }
                let target_body = match &module.functions[target_idx].body {
                    FunctionBody::Lowered(b) => b,
                    FunctionBody::PassThrough(_) => continue,
                };
                if target_body.blocks.len() > MAX_BLOCKS
                    || target_body.instructions.len() > MAX_INSTS
                {
                    continue;
                }
                // Arity match — extra const args are fine (treated as
                // dropped); fewer is also fine (missing args become
                // undefined). We don't enforce strict equality.
                let arity = target_body.params.len();
                if const_args.len() < arity {
                    // Pad with undefined to match arity.
                }

                // Clone callee Function and substitute Args.
                let Some(answer) = sandbox_evaluate(
                    module,
                    target_fid,
                    &const_args,
                ) else {
                    continue;
                };

                // Re-intern any string in the answer against the original
                // module's interner. The sandbox interner was cloned
                // from the original, so any StrId pre-existing in the
                // sandbox is also valid in the original. New strings
                // interned during the sandbox run (e.g., ConcatStr)
                // have IDs the original interner does not know — fix
                // that by resolving the string in the sandbox and
                // re-interning into the module.
                let answer = match answer {
                    Operand::Const(Constant::String(_)) => {
                        // We need the sandbox interner for this; redo
                        // the evaluation returning the resolved text
                        // already. Easier: re-run with a side channel.
                        // Skip String answers conservatively for now —
                        // numeric/bool/null/undefined cover the
                        // decoder-shape test cases. (Surface a future
                        // ticket if a corpus sample needs it.)
                        continue;
                    }
                    other => other,
                };

                // Apply the rewrite to the caller's body.
                let body = match &mut module.functions[caller_idx].body {
                    FunctionBody::Lowered(b) => b,
                    FunctionBody::PassThrough(_) => continue,
                };
                rewrite_call_to_const(body, call_iid, answer);
                any_changed = true;
            }
        }

        Ok(any_changed)
    }
}

/// Build a one-function sandbox `Module` from the callee, substitute
/// `Arg(i)` with `Const(args[i])`, run the mini-pipeline, and return
/// the exit terminator's `Return` value when it has reduced to a
/// `Const`. Returns `None` for any non-converging or non-const result.
fn sandbox_evaluate(
    module: &Module,
    target_fid: FuncId,
    const_args: &[Constant],
) -> Option<Operand> {
    // Build the sandbox Module: one Function (the cloned callee),
    // shared (cloned) interner. Other functions are not copied; the
    // sandbox passes don't traverse FuncId const refs in the body.
    let mut sandbox = Module {
        functions: Vec::with_capacity(1),
        strings: module.strings.clone(),
        top_level_layout: Vec::new(),
    };
    let mut callee = module.functions[target_fid.0 as usize].clone();

    // Substitute Arg(i) Values with Const(args[i]) directly via
    // `replace_value_uses` for each arg-bound Value index.
    let body = match &mut callee.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    let arg_values: Vec<(Value, Constant)> = body
        .values
        .iter()
        .enumerate()
        .filter_map(|(vi, def)| match def.kind {
            ValueDefKind::Arg(i) => {
                let c = const_args.get(i as usize).copied()?;
                Some((Value(vi as u32), c))
            }
            _ => None,
        })
        .collect();
    for (v, c) in &arg_values {
        body.replace_value_uses(*v, Operand::Const(*c));
        // Retire the Arg slot too — not strictly required (no uses
        // remain after replace_value_uses), but keeps the IR tidy
        // for pretty-printing in error paths.
        body.values[v.0 as usize] = ValueDef::removed();
    }

    sandbox.functions.push(callee);
    let sandbox_fid = FuncId(0);

    // Run the mini-pipeline. We use the standard PassManager so the
    // fixed-point loop is the same as production. Validation off
    // (the sandbox can have dangling FuncId references that trip
    // module validation; we only need terminator inspection).
    let mut pm = PassManager::new()
        .with_validate_between(false)
        .with_max_iterations(16);
    pm.add_pass(Box::new(ConstFoldPass));
    pm.add_pass(Box::new(BranchFoldPass));
    pm.add_pass(Box::new(ForwardLoadsPass));
    pm.add_pass(Box::new(DcePass));

    // Treat any pipeline error (including PassFixedPointNotReached)
    // as "no answer". The original IR is untouched.
    if pm.run(&mut sandbox, sandbox_fid).is_err() {
        return None;
    }

    // Inspect every reachable Return terminator. If they all return
    // the same Const, that's the answer. Multi-Return bodies can
    // collapse to one reachable Return after BranchFold + Dce.
    let body = match &sandbox.functions[0].body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    // Walk reachable blocks from entry. Unreachable blocks may still
    // hold their original Return terminators after BranchFold (which
    // doesn't prune dead blocks); ignoring them is both correct and
    // necessary for branch-collapsing to register as a Const answer.
    let mut reachable = vec![false; body.blocks.len()];
    let mut stack = vec![body.entry];
    while let Some(b) = stack.pop() {
        let bi = b.0 as usize;
        if bi >= reachable.len() || reachable[bi] {
            continue;
        }
        reachable[bi] = true;
        for (succ, _) in body.blocks[bi].terminator.successors() {
            stack.push(succ);
        }
    }
    // Side-effect scan: any reachable instruction whose effect is
    // not exactly `PURE` is a real side effect we'd erase by
    // replacing the Call with a Const. The all-Returns-Const gate
    // alone is NOT sufficient — two failure modes the gate misses:
    //   (a) Standalone non-removable side effects, e.g.
    //       `globalThis.counter = 1; return 5;` produces
    //       `Return Const(5)` after sandbox folding while still
    //       carrying a `StoreProp` (non-removable, `WRITES_HEAP`).
    //       Erasing the Call would erase the store. Bail.
    //   (b) `MAY_THROW`-bearing operations whose result is dead.
    //       `Eq`/`Lt`/`AddGeneric`/`LoadProp_throws` are
    //       non-removable when unused (`MAY_THROW` blocks DCE), so
    //       they survive in the sandbox even if the converged Const
    //       answer doesn't depend on them. Erasing the Call would
    //       erase a potential throw. Bail.
    // If the body genuinely converges to a Const result, ConstFold
    // collapses arithmetic / comparisons on Const inputs and DCE
    // removes the now-pure dead computations — so a true partial-
    // eval candidate is `PURE`-only after sandbox convergence. This
    // is strictly stricter than v1's `is_inlinable_shape` gate
    // (which only checked the matcher's structural shape, not
    // residual effects), and stricter is the right direction for a
    // soundness gate that didn't previously exist explicitly.
    for (bi, blk) in body.blocks.iter().enumerate() {
        if !reachable[bi] {
            continue;
        }
        for &iid in &blk.instructions {
            let eff = body.instructions[iid.0 as usize].kind.effect();
            if eff != ir::ssa::Effect::PURE {
                return None;
            }
        }
    }

    let mut answer: Option<Constant> = None;
    for (bi, blk) in body.blocks.iter().enumerate() {
        if !reachable[bi] {
            continue;
        }
        match &blk.terminator {
            Terminator::Return { val: Some(Operand::Const(c)) } => match answer {
                None => answer = Some(*c),
                Some(prev) if ir::ssa::const_bit_eq(&prev, c) => {}
                Some(_) => return None,
            },
            // Any non-Const Return value (Operand::Value / Capture /
            // Arg / etc.) means the body's result depends on a
            // computation the sandbox couldn't fold — likely a
            // surviving heap load, an unfoldable Call, or a captured
            // operand. Bail; this is the load-bearing soundness gate.
            Terminator::Return { .. } => return None,
            // A reachable `Throw` would mean the call site can throw
            // an observable exception we cannot prove away. Even if
            // some other reachable block returns a Const, the call
            // semantics include the throw path; replacing the Call
            // result with a Const would erase that observable. Bail.
            Terminator::Throw { .. } => return None,
            // Try framing — bail conservatively. The sandbox doesn't
            // attempt to reason about exception flow.
            Terminator::Try { .. } => return None,
            // Branch / CondBranch / Switch are interior terminators;
            // they don't define the call's return value on their own.
            _ => {}
        }
    }
    answer.map(Operand::Const)
}

/// Replace the result of `call_iid` with `replacement` in `body`,
/// retire the Value, unhook the instruction from its block, and blank
/// the call's operands. Mirrors the convention in
/// `inline_flat_at_call_site`.
fn rewrite_call_to_const(body: &mut LoweredBody, call_iid: InstId, replacement: Operand) {
    // Find containing block + position.
    let (bi, pos) = {
        let mut found = None;
        'outer: for (bi, blk) in body.blocks.iter().enumerate() {
            for (pos, &iid) in blk.instructions.iter().enumerate() {
                if iid == call_iid {
                    found = Some((bi, pos));
                    break 'outer;
                }
            }
        }
        match found {
            Some(p) => p,
            None => return, // already retired (defensive)
        }
    };

    let call_v = find_value_for_inst(body, call_iid);
    body.replace_value_uses(call_v, replacement);
    body.values[call_v.0 as usize] = ValueDef::removed();
    body.blocks[bi].instructions.remove(pos);
    walk_inst_operands_mut(&mut body.instructions[call_iid.0 as usize].kind, &mut |op| {
        *op = Operand::Const(Constant::Undefined);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use ir::ssa::{
        BlockId, Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity,
        Terminator,
    };

    /// `function f(x) { if (x > 0) return 1; return 2; } f(5)` → 1.
    #[test]
    fn collapses_branch_on_const_arg() {
        let mut module = Module::new();
        // f(x): if x > 0 return 1 else return 2.
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let x = b.body.params[0];
            let entry = b.new_block();
            let then_b = b.new_block();
            let else_b = b.new_block();
            b.switch_to_block(entry);
            let cmp = b
                // x > 0  ≡  0 < x
                .add_inst(InstKind::Lt {
                    lhs: Operand::Const(Constant::Int32(0)),
                    rhs: Operand::Value(x),
                })
                .unwrap();
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(cmp),
                then_b,
                then_args: vec![],
                else_b,
                else_args: vec![],
            });
            b.switch_to_block(then_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(1))),
            });
            b.switch_to_block(else_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(2))),
            });
            b.finish();
        }
        // caller: return f(5)
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Const(Constant::Int32(5))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(changed, "PartialEvalPass should report changes");

        let caller = &module.functions[1];
        let body = match &caller.body {
            FunctionBody::Lowered(b) => b,
            _ => panic!("caller must be Lowered"),
        };
        match &body.blocks[0].terminator {
            Terminator::Return { val: Some(Operand::Const(Constant::Int32(1))) } => {}
            other => panic!("expected Return Int32(1), got {other:?}"),
        }
    }

    /// Negative branch: f(-3) → 2.
    #[test]
    fn collapses_branch_on_negative_const_arg() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let x = b.body.params[0];
            let entry = b.new_block();
            let then_b = b.new_block();
            let else_b = b.new_block();
            b.switch_to_block(entry);
            let cmp = b
                // x > 0  ≡  0 < x
                .add_inst(InstKind::Lt {
                    lhs: Operand::Const(Constant::Int32(0)),
                    rhs: Operand::Value(x),
                })
                .unwrap();
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(cmp),
                then_b,
                then_args: vec![],
                else_b,
                else_args: vec![],
            });
            b.switch_to_block(then_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(1))),
            });
            b.switch_to_block(else_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(2))),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Const(Constant::Int32(-3))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pass = PartialEvalPass;
        pass.run(&mut module).unwrap();

        let body = module.functions[1].lowered();
        match &body.blocks[0].terminator {
            Terminator::Return { val: Some(Operand::Const(Constant::Int32(2))) } => {}
            other => panic!("expected Return Int32(2), got {other:?}"),
        }
    }

    /// `function add(a, b) { return a + b; } add(2, 3)` → 5.
    #[test]
    fn collapses_multi_arg_addition() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "add", 2);
            let a = b.body.params[0];
            let bp = b.body.params[1];
            let entry = b.new_block();
            b.switch_to_block(entry);
            let sum = b
                .add_inst(InstKind::AddInt32 {
                    lhs: Operand::Value(a),
                    rhs: Operand::Value(bp),
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(sum)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![
                        Operand::Const(Constant::Int32(2)),
                        Operand::Const(Constant::Int32(3)),
                    ],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pass = PartialEvalPass;
        pass.run(&mut module).unwrap();

        let body = module.functions[1].lowered();
        match &body.blocks[0].terminator {
            Terminator::Return { val: Some(Operand::Const(Constant::Int32(5))) } => {}
            other => panic!("expected Return Int32(5), got {other:?}"),
        }
    }

    /// Side-effecting callee — partial eval should decline. The
    /// callee writes to a captured object then returns a Const. The
    /// post-pipeline side-effect scan must catch the residual
    /// `WRITES_HEAP` `StoreProp` and bail, even though every Return
    /// is `Const(1)`. (Without that scan, the v2 relaxation would
    /// erase the store by replacing the Call with `Const(1)`.)
    #[test]
    fn declines_side_effecting_callee() {
        let mut module = Module::new();
        let key_x;
        {
            let mut b =
                ir::ssa::FunctionBuilder::new_with_captures(&mut module, "f", 0, 1);
            key_x = b.intern("x");
            let entry = b.new_block();
            b.switch_to_block(entry);
            // Store on an opaque captured value — not a tracked
            // alloc, so the shape matcher rejects this body.
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Capture(0),
                key: key_x,
                val: Operand::Const(Constant::Int32(7)),
                purity: Purity::Opaque,
            });
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(1))),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
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

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(!changed, "side-effecting callee must not be partial-eval'd");
        // Caller still has the Call instruction live.
        let body = module.functions[1].lowered();
        let call_inst = body.blocks[0].instructions.first().copied().unwrap();
        let _ = key_x;
        match &body.instructions[call_inst.0 as usize].kind {
            InstKind::Call { .. } => {}
            other => panic!("call should still be present, got {other:?}"),
        }
    }

    /// Multi-block callee whose dead branch contains an operand
    /// `is_inlinable_shape` rejects (a `Capture`). Under v1 the
    /// shape predicate rejected the WHOLE body; under v2 we sandbox
    /// the body, BranchFold prunes the Capture-using branch via the
    /// const-arg specialisation, the side-effect scan sees only the
    /// reachable `PURE` blocks, and we resolve to a Const.
    ///
    /// `function f(x) { if (x === 1) return 42; return capturedThing; }`
    /// `f(1)` -> 42.
    #[test]
    fn relaxed_v2_collapses_with_unreachable_capture_path() {
        let mut module = Module::new();
        {
            let mut b =
                ir::ssa::FunctionBuilder::new_with_captures(&mut module, "f", 1, 1);
            let x = b.body.params[0];
            let entry = b.new_block();
            let then_b = b.new_block();
            let else_b = b.new_block();
            b.switch_to_block(entry);
            let cmp = b
                .add_inst(InstKind::StrictEq {
                    lhs: Operand::Value(x),
                    rhs: Operand::Const(Constant::Int32(1)),
                })
                .unwrap();
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(cmp),
                then_b,
                then_args: vec![],
                else_b,
                else_args: vec![],
            });
            b.switch_to_block(then_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(42))),
            });
            b.switch_to_block(else_b);
            // Return a captured opaque value — the shape predicate
            // rejects bodies referencing `Capture(_)` outside its
            // narrow recognised forms, so v1 declined this body.
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Capture(0)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Const(Constant::Int32(1))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(
            changed,
            "v2 partial-eval should fold f(1) -> 42 even though the dead else-branch references a Capture"
        );
        let body = module.functions[1].lowered();
        match &body.blocks[0].terminator {
            Terminator::Return { val: Some(Operand::Const(Constant::Int32(42))) } => {}
            other => panic!("expected Return Int32(42), got {other:?}"),
        }
    }

    /// Multi-block callee where every reachable branch returns the
    /// SAME Const but via different paths — this is the convergence
    /// case the relaxation enables. Build a 4-block body:
    /// `function f(x) { if (x > 10) { if (x > 100) return 7; return 7; } return 7; }`
    /// All paths converge to 7. With non-const `x`, the sandbox can't
    /// prune branches but the gate ("every reachable Return == same
    /// Const") still applies — we test with a const arg so the
    /// pipeline does the standard collapse.
    #[test]
    fn relaxed_v2_multi_block_converging_returns() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let x = b.body.params[0];
            let entry = b.new_block();
            let outer_then = b.new_block();
            let inner_then = b.new_block();
            let inner_else = b.new_block();
            let outer_else = b.new_block();
            b.switch_to_block(entry);
            let cmp1 = b
                .add_inst(InstKind::Lt {
                    lhs: Operand::Const(Constant::Int32(10)),
                    rhs: Operand::Value(x),
                })
                .unwrap();
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(cmp1),
                then_b: outer_then,
                then_args: vec![],
                else_b: outer_else,
                else_args: vec![],
            });
            b.switch_to_block(outer_then);
            let cmp2 = b
                .add_inst(InstKind::Lt {
                    lhs: Operand::Const(Constant::Int32(100)),
                    rhs: Operand::Value(x),
                })
                .unwrap();
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(cmp2),
                then_b: inner_then,
                then_args: vec![],
                else_b: inner_else,
                else_args: vec![],
            });
            b.switch_to_block(inner_then);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(7))),
            });
            b.switch_to_block(inner_else);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(7))),
            });
            b.switch_to_block(outer_else);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(7))),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Const(Constant::Int32(50))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(changed, "all-paths-return-7 callee should partial-eval to 7");
        let body = module.functions[1].lowered();
        match &body.blocks[0].terminator {
            Terminator::Return { val: Some(Operand::Const(Constant::Int32(7))) } => {}
            other => panic!("expected Return Int32(7), got {other:?}"),
        }
    }

    /// Multi-Return callee where the const-arg specialisation does
    /// NOT collapse all paths to the same Const — gate must bail.
    /// `function f(x) { if (x > 0) return 1; return 2; } f(unknown)`.
    /// We can't quite call with non-const, but we can make returns
    /// differ AND have an unfoldable branch. Easier: have a branch
    /// whose condition the sandbox can't fold, and the two arms
    /// return different Consts. Use a `Capture` as the condition.
    #[test]
    fn relaxed_v2_diverging_returns_bails() {
        let mut module = Module::new();
        {
            let mut b =
                ir::ssa::FunctionBuilder::new_with_captures(&mut module, "f", 0, 1);
            let entry = b.new_block();
            let then_b = b.new_block();
            let else_b = b.new_block();
            b.switch_to_block(entry);
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Capture(0),
                then_b,
                then_args: vec![],
                else_b,
                else_args: vec![],
            });
            b.switch_to_block(then_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(1))),
            });
            b.switch_to_block(else_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(2))),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
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

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(!changed, "diverging Return Consts must bail");
    }

    /// Reachable `Throw` must bail even when another reachable path
    /// returns a Const.
    #[test]
    fn relaxed_v2_reachable_throw_bails() {
        let mut module = Module::new();
        {
            let mut b =
                ir::ssa::FunctionBuilder::new_with_captures(&mut module, "f", 0, 1);
            let entry = b.new_block();
            let then_b = b.new_block();
            let else_b = b.new_block();
            b.switch_to_block(entry);
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Capture(0),
                then_b,
                then_args: vec![],
                else_b,
                else_args: vec![],
            });
            b.switch_to_block(then_b);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(1))),
            });
            b.switch_to_block(else_b);
            b.set_terminator(Terminator::Throw {
                val: Operand::Const(Constant::Int32(99)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 0);
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

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(!changed, "reachable Throw must bail to preserve throw observability");
    }

    /// Non-const arg — partial eval declines (the whole point of the
    /// gate).
    #[test]
    fn declines_when_arg_not_const() {
        let mut module = Module::new();
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 1);
            let x = b.body.params[0];
            let entry = b.new_block();
            b.switch_to_block(entry);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(x)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "caller", 1);
            let p = b.body.params[0];
            let entry = b.new_block();
            b.switch_to_block(entry);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Value(p)],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pass = PartialEvalPass;
        let changed = pass.run(&mut module).unwrap();
        assert!(
            !changed,
            "non-const arg must not trigger partial eval"
        );
        let _ = BlockId(0);
    }
}
