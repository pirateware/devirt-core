//! Constructor-function inlining — sibling to `IdentityFnInlinePass`.
//!
//! Recognises functions whose body is purely a fresh-allocation
//! construction pattern and clones the construction at every Call
//! site. After cloning, the heap store-load forwarding pass
//! (`ForwardLoadsPass`) sees the alloc + stores in the caller's
//! function and folds dependent loads.
//!
//! Constructor shape (the matcher):
//!   - Lowered body, single block, no block params.
//!   - Instructions are exactly: ONE AllocObject or AllocArray, then
//!     zero or more StoreProp/StoreIndex whose `obj` is the alloc,
//!     whose key is `Const(_)` (string or integer), and whose `val`
//!     is `Operand::Const(_)` or `Operand::Value(arg_i)`. No other
//!     instructions are allowed.
//!   - Terminator is `Return { val: Some(Operand::Value(alloc)) }`.
//!
//! Cloning at a Call site:
//!   - Fresh Value IDs in the caller for the cloned alloc and each
//!     cloned store (stores produce no Value, but get fresh InstIds
//!     in the caller's `body.instructions`).
//!   - Substitute every `Operand::Value(arg_i)` in cloned operands
//!     with `args[i]` from the Call (bounds-check; out-of-range →
//!     `Const::Undefined`, matching JS missing-arg semantics).
//!   - Insert the cloned instructions into the caller's block at the
//!     position where the Call lives, just before the Call.
//!   - Retire the Call: replace its result Value uses with the
//!     cloned alloc's fresh Value, mark the Call's Value Removed,
//!     unhook the Call from the block, blank operands.
//!
//! Soundness:
//!   - Each Call site receives its own fresh alloc (distinct Value).
//!     JS semantics: each call to a constructor returns a fresh
//!     object — preserved.
//!   - The constructor body has no side effects beyond the alloc and
//!     store sequence (matcher guards this). No reordering risk.
//!
//! Out of scope (matcher rejects):
//!   - Multi-block bodies, block params, conditional construction.
//!   - Stores whose val is a heap read or arithmetic result (would
//!     need cloning the upstream sub-DAG; deferred).
//!   - Construct (`new`) — different semantics; skip.

use std::collections::HashMap;

use ir::error::Error;
use ir::ssa::{
    Constant, FunctionBody, InstId, InstKind, Module, Operand, Value, ValueDef, ValueDefKind,
    find_value_for_inst, walk_inst_operands_mut,
};
// `walk_inst_operands_mut` is used by the Call-retire blanking; the
// targeted store rewrites in `rewrite_operands` use direct field access.

use crate::ModulePass;

pub struct ConstructorFnInlinePass;

/// Function summary for a constructor candidate. Stores the cloned
/// instruction kinds that form the construction sequence (alloc
/// first, stores after) along with the alloc's Value and the result
/// Value (for the return).
#[derive(Clone, Debug)]
struct ConstructorSummary {
    alloc_kind: InstKind,
    /// Each store's `kind` — must be StoreProp or StoreIndex.
    store_kinds: Vec<InstKind>,
}

impl ModulePass for ConstructorFnInlinePass {
    fn name(&self) -> &'static str {
        "constructor_fn_inline"
    }

    fn run(&mut self, module: &mut Module) -> Result<bool, Error> {
        // Phase 1: build summaries for every Lowered function whose
        // body matches the constructor shape.
        let mut summaries: HashMap<u32, ConstructorSummary> = HashMap::new();
        for (i, func) in module.functions.iter().enumerate() {
            if let Some(s) = match_constructor(func) {
                summaries.insert(i as u32, s);
            }
        }

        if summaries.is_empty() {
            return Ok(false);
        }

        // Phase 2: rewrite Call sites. Walk every function body
        // (including the constructors themselves; recursive calls of
        // constructors-via-constructors aren't a thing under the
        // matcher, but the loop is uniform).
        let mut any_changed = false;
        let n_funcs = module.functions.len();
        for fid_idx in 0..n_funcs {
            let body = match &mut module.functions[fid_idx].body {
                FunctionBody::Lowered(b) => b,
                FunctionBody::PassThrough(_) => continue,
            };

            // Snapshot rewrite candidates before mutating: each entry
            // is (block_index, position_in_block.instructions, call_iid,
            // target_fid).
            let mut rewrites: Vec<(usize, usize, u32, u32)> = Vec::new();
            for (bi, blk) in body.blocks.iter().enumerate() {
                for (pos, &iid) in blk.instructions.iter().enumerate() {
                    let inst = &body.instructions[iid.0 as usize];
                    if let InstKind::Call { callee, .. } = &inst.kind
                        && let Operand::Const(Constant::Func(fid)) = callee
                        && summaries.contains_key(&fid.0)
                    {
                        rewrites.push((bi, pos, iid.0, fid.0));
                    }
                }
            }

            if rewrites.is_empty() {
                continue;
            }

            // Apply rewrites in reverse position order so earlier
            // rewrites don't shift later positions in `block.instructions`.
            rewrites.sort_by_key(|&(bi, pos, _, _)| (bi, std::cmp::Reverse(pos)));

            for (bi, pos, call_iid, target_fid) in rewrites {
                let summary = summaries.get(&target_fid).unwrap().clone();
                inline_constructor_at(body, bi, pos, InstId(call_iid), &summary);
                any_changed = true;
            }
        }
        Ok(any_changed)
    }
}

/// True iff `func` matches the constructor shape used by this pass.
/// Exposed so `DeadFnElimPass` can use the same predicate to recognise
/// constructor-shape functions as eligible for deletion when their
/// real-use count drops to zero (post-inlining).
pub fn is_constructor_shape(func: &ir::ssa::Function) -> bool {
    match_constructor(func).is_some()
}

/// Match the constructor shape on `func`. Returns the summary if it
/// matches, `None` otherwise.
fn match_constructor(func: &ir::ssa::Function) -> Option<ConstructorSummary> {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    if body.blocks.len() != 1 {
        return None;
    }
    let entry = &body.blocks[body.entry.0 as usize];
    if !entry.params.is_empty() {
        return None;
    }
    if entry.instructions.is_empty() {
        return None;
    }

    // First inst must be AllocObject or AllocArray, producing the
    // alloc Value.
    let first_iid = entry.instructions[0];
    let alloc_kind = body.instructions[first_iid.0 as usize].kind.clone();
    if !matches!(
        alloc_kind,
        InstKind::AllocObject | InstKind::AllocArray { .. }
    ) {
        return None;
    }
    let alloc_value = find_value_for_inst(body, first_iid);

    // Subsequent insts must be StoreProp/StoreIndex on the alloc, with
    // const keys and const-or-arg val operands.
    let mut store_kinds: Vec<InstKind> = Vec::new();
    for &iid in &entry.instructions[1..] {
        let kind = &body.instructions[iid.0 as usize].kind;
        match kind {
            InstKind::StoreProp { obj, val, .. } => {
                if !operand_is(obj, alloc_value) {
                    return None;
                }
                if !val_is_const_or_arg(val, body) {
                    return None;
                }
                store_kinds.push(kind.clone());
            }
            InstKind::StoreIndex {
                obj, index, val, ..
            } => {
                if !operand_is(obj, alloc_value) {
                    return None;
                }
                if !matches!(index, Operand::Const(_)) {
                    return None;
                }
                if !val_is_const_or_arg(val, body) {
                    return None;
                }
                store_kinds.push(kind.clone());
            }
            _ => return None,
        }
    }

    // Terminator must Return Some(Value(alloc)).
    match &entry.terminator {
        ir::ssa::Terminator::Return {
            val: Some(Operand::Value(v)),
        } if *v == alloc_value => {}
        _ => return None,
    }

    Some(ConstructorSummary {
        alloc_kind,
        store_kinds,
    })
}

fn operand_is(op: &Operand, target: Value) -> bool {
    matches!(op, Operand::Value(v) if *v == target)
}

fn val_is_const_or_arg(val: &Operand, body: &ir::ssa::LoweredBody) -> bool {
    match val {
        Operand::Const(_) => true,
        Operand::Value(v) => matches!(
            body.values[v.0 as usize].kind,
            ValueDefKind::Arg(_)
        ),
        _ => false,
    }
}

/// Splice the constructor's construction sequence into the caller's
/// block at `pos`, with operands rewritten so:
///   - Stores' `obj` reference the freshly-cloned alloc Value (not
///     the original constructor's alloc Value).
///   - `Operand::Value(arg_i)` operands become the call's args[i] (or
///     Const::Undefined if out of range).
///
/// Then retire the Call inst.
fn inline_constructor_at(
    body: &mut ir::ssa::LoweredBody,
    bi: usize,
    pos: usize,
    call_iid: InstId,
    summary: &ConstructorSummary,
) {
    // Extract Call's args (clone — we'll mutate body.instructions).
    let call_args: Vec<Operand> = match &body.instructions[call_iid.0 as usize].kind {
        InstKind::Call { args, .. } => args.clone(),
        _ => unreachable!("inline_constructor_at called on non-Call inst"),
    };

    // 1. Allocate a fresh Value for the cloned alloc.
    let new_alloc_iid = InstId(body.instructions.len() as u32);
    body.instructions.push(ir::ssa::Instruction {
        kind: rewrite_operands(&summary.alloc_kind, None, &call_args),
    });
    body.values.push(ValueDef::inst(new_alloc_iid));
    let new_alloc_value = Value(body.values.len() as u32 - 1);

    // 2. Allocate fresh InstIds for each cloned store. Stores are void
    // (no Value), so we don't push to body.values for them.
    let mut new_inst_ids: Vec<InstId> = Vec::with_capacity(1 + summary.store_kinds.len());
    new_inst_ids.push(new_alloc_iid);
    for store_kind in &summary.store_kinds {
        let new_store_iid = InstId(body.instructions.len() as u32);
        body.instructions.push(ir::ssa::Instruction {
            kind: rewrite_operands(store_kind, Some(new_alloc_value), &call_args),
        });
        new_inst_ids.push(new_store_iid);
    }

    // 3. Splice cloned InstIds into the block immediately before the
    // Call. Preserve insertion order. (Stores into a Vec at index N
    // shift everything from N onwards down by len(new_inst_ids).)
    let blk = &mut body.blocks[bi];
    blk.instructions.splice(pos..pos, new_inst_ids.iter().copied());

    // After the splice, the Call's position has shifted by
    // `new_inst_ids.len()`. Recompute and retire it.
    let call_pos = pos + new_inst_ids.len();
    debug_assert_eq!(blk.instructions[call_pos].0, call_iid.0);

    // Retire the Call: replace its result Value uses with the cloned
    // alloc's Value, mark Removed, unhook, blank operands.
    let call_result_v = find_value_for_inst(body, call_iid);
    body.replace_value_uses(call_result_v, Operand::Value(new_alloc_value));
    body.values[call_result_v.0 as usize] = ValueDef::removed();
    let blk = &mut body.blocks[bi];
    blk.instructions.remove(call_pos);
    walk_inst_operands_mut(
        &mut body.instructions[call_iid.0 as usize].kind,
        &mut |op| {
            *op = Operand::Const(Constant::Undefined);
        },
    );
}

/// Rewrite a constructor-body instruction kind for splicing into the
/// caller. Two targeted edits driven by the matcher's invariants:
///
///   1. `new_alloc` (Some only when rewriting a Store): retarget
///      the store's `obj` to the freshly-cloned alloc Value. The
///      matcher guaranteed the original `obj` equals the constructor's
///      alloc, so this is a direct overwrite.
///   2. Store `val` operand: if it's `Operand::Value(v)`, then per the
///      matcher's invariant `v` must be an `Arg(i)` — substitute with
///      `call_args[i]` (or `Const::Undefined` for out-of-range, which
///      mirrors JS missing-arg semantics). The constructor's args
///      occupy Value indices 0..arity-1 (FunctionBuilder pushes Arg
///      ValueDefs before any Inst ValueDefs), so `v.0` is the arg
///      index.
///
/// AllocObject / AllocArray have no operands needing rewriting.
/// StoreIndex's `index` is always `Const` per the matcher (rejected
/// otherwise) — no rewrite needed.
fn rewrite_operands(
    src: &InstKind,
    new_alloc: Option<Value>,
    call_args: &[Operand],
) -> InstKind {
    let mut out = src.clone();
    if let Some(a) = new_alloc {
        match &mut out {
            InstKind::StoreProp { obj, .. } | InstKind::StoreIndex { obj, .. } => {
                *obj = Operand::Value(a);
            }
            _ => {}
        }
    }
    match &mut out {
        InstKind::StoreProp { val, .. } | InstKind::StoreIndex { val, .. } => {
            if let Operand::Value(v) = val {
                let idx = v.0 as usize;
                *val = call_args
                    .get(idx)
                    .copied()
                    .unwrap_or(Operand::Const(Constant::Undefined));
            }
        }
        _ => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
    };

    /// `function mk() { return {foo: 1}; }` recognized as constructor;
    /// at the call site, the alloc + store get cloned in.
    #[test]
    fn inlines_simple_object_constructor() {
        let mut module = Module::new();
        let key_foo;
        // mk() — constructor.
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 0);
            key_foo = b.intern("foo");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_foo,
                val: Operand::Const(Constant::Number(1.0)),
                purity: Purity::Pure,
            });
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(alloc)),
            });
            b.finish();
        }
        // f() — calls mk() and returns it.
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
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
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
            "Call should be retired"
        );
        // An AllocObject + a StoreProp{key=foo} should be present.
        let kinds: Vec<&InstKind> = f_body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .map(|iid| &f_body.instructions[iid.0 as usize].kind)
            .collect();
        assert!(matches!(kinds[0], InstKind::AllocObject), "{kinds:?}");
        assert!(matches!(
            kinds[1],
            InstKind::StoreProp { key, val: Operand::Const(Constant::Number(n)), .. }
                if *key == key_foo && *n == 1.0
        ));
    }

    /// `function mkArr() { return ['a','b','c']; }` — array constructor
    /// with three indexed stores. After inlining + ForwardLoads (from
    /// the full pipeline), `mkArr()[1]` should resolve to "b".
    #[test]
    fn inlines_array_constructor_then_forward_loads_resolves_index() {
        let mut module = Module::new();
        let s_a;
        let s_b;
        let s_c;
        // mkArr() — constructor.
        {
            let mut b = FunctionBuilder::new(&mut module, "mkArr", 0);
            s_a = b.intern("a");
            s_b = b.intern("b");
            s_c = b.intern("c");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b
                .add_inst(InstKind::AllocArray { length: Some(3) })
                .unwrap();
            for (i, sid) in [s_a, s_b, s_c].iter().enumerate() {
                b.add_inst(InstKind::StoreIndex {
                    obj: Operand::Value(alloc),
                    index: Operand::Const(Constant::Int32(i as i32)),
                    val: Operand::Const(Constant::String(*sid)),
                    purity: Purity::Pure,
                });
            }
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(alloc)),
            });
            b.finish();
        }
        // f() — calls mkArr() and reads [1].
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
            let load_v = b
                .add_inst(InstKind::LoadIndex {
                    obj: Operand::Value(call_v),
                    index: Operand::Const(Constant::Int32(1)),
                    purity: Purity::Pure,
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(load_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        // ForwardLoads runs as a function pass.
        pm.add_pass(Box::new(crate::forward_loads::ForwardLoadsPass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::String(id))),
            } => assert_eq!(module.strings.resolve(id), "b"),
            other => panic!("expected ret String('b'), got {other:?}"),
        }
    }

    /// Constructor with arg-derived store value. `function mk(x) {
    /// return {val: x}; }` then `mk(42)` → cloned alloc + StoreProp
    /// where val = Const(42).
    #[test]
    fn inlines_constructor_with_arg_substitution() {
        let mut module = Module::new();
        let key_val;
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 1);
            key_val = b.intern("val");
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_val,
                val: Operand::Value(arg0),
                purity: Purity::Pure,
            });
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(alloc)),
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
                    args: vec![Operand::Const(Constant::Number(42.0))],
                })
                .unwrap();
            let load_v = b
                .add_inst(InstKind::LoadProp {
                    obj: Operand::Value(call_v),
                    key: key_val,
                    purity: Purity::Pure,
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(load_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        pm.add_pass(Box::new(crate::forward_loads::ForwardLoadsPass));
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

    /// Body has a non-store instruction → not a constructor.
    #[test]
    fn does_not_match_when_body_has_extra_instruction() {
        let mut module = Module::new();
        let key_foo;
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 0);
            key_foo = b.intern("foo");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            // Extra side-effecty inst — disqualifies.
            let _ = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Const(Constant::Number(1.0)),
                    rhs: Operand::Const(Constant::Number(2.0)),
                })
                .unwrap();
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_foo,
                val: Operand::Const(Constant::Number(1.0)),
                purity: Purity::Pure,
            });
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(alloc)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let _ = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
                })
                .unwrap();
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        assert!(
            f_body.blocks[entry.0 as usize]
                .instructions
                .iter()
                .any(|iid| matches!(
                    f_body.instructions[iid.0 as usize].kind,
                    InstKind::Call { .. }
                )),
            "non-matching constructor → Call must remain"
        );
    }
}
