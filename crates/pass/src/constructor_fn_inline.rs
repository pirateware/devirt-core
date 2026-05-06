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

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    Constant, Effect, FunctionBody, InstId, InstKind, Module, Operand, Value, ValueDef,
    ValueDefKind, find_value_for_inst, walk_inst_operands_mut,
};

use crate::ModulePass;

pub struct ConstructorFnInlinePass;

/// One construction step, in body order. Either an expression
/// instruction (produces a Value) or a store on the alloc (void).
#[derive(Clone, Debug)]
struct Step {
    kind: InstKind,
    /// `Some(body_value_idx)` if this step produces a Value (i.e., is
    /// an expression instruction). `None` for stores.
    produced: Option<u32>,
}

/// Function summary for a constructor candidate. Steps are stored in
/// body order; the alloc step is one of them (identified by
/// `alloc_body_value` matching its `produced`), not pulled out
/// separately. AST-level "evaluate expressions, then construct
/// object literal" leaves expressions in body order BEFORE the alloc
/// — handled naturally by walking steps in order.
#[derive(Clone, Debug)]
struct ConstructorSummary {
    /// Body's Value index for the alloc result.
    alloc_body_value: u32,
    /// Function arity — bounds the call_args lookup at cloning time.
    arity: usize,
    /// All non-terminator instructions in body order: alloc,
    /// expressions, and stores interleaved.
    steps: Vec<Step>,
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
///
/// Accepted body shape (single block, no block params):
///   - Instruction 0: `AllocObject` or `AllocArray`.
///   - Instructions 1..N: each is either
///       a. A **store** (`StoreProp` / `StoreIndex`) whose `obj` is
///          the alloc Value, key is `Const(_)`, val is an "allowed
///          operand" (see below). StoreIndex's `index` must also be
///          `Const(_)` so the post-clone shape is trackable by
///          ForwardLoadsPass.
///       b. An **expression** instruction whose effect is a subset
///          of `PURE | MAY_THROW` (no heap reads/writes, no diverge,
///          not opaque) and whose every operand is "allowed."
///   - Terminator: `Return(Some(Value(alloc)))`.
///
/// "Allowed operand" = `Const(_)`, `Arg(i)` Value, or a Value
/// produced by an earlier expression step in this same body. That
/// last case is what makes the matcher accept patterns like
/// `function mk(x) { return { val: x + 1 }; }`: the `AddGeneric`
/// step produces a Value referenced by the subsequent StoreProp.
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

    let arity = body.params.len();

    // Allowed-operand set grows as we walk: seeded with all Args, gains
    // each expression's produced Value (and the alloc when we see it).
    let mut allowed_values: HashSet<u32> = HashSet::new();
    for v_idx in 0..body.values.len() {
        if let ValueDefKind::Arg(_) = body.values[v_idx].kind {
            allowed_values.insert(v_idx as u32);
        }
    }

    let mut steps: Vec<Step> = Vec::new();
    let allowed_effect = Effect::PURE | Effect::MAY_THROW;
    let mut alloc_value: Option<Value> = None;

    for &iid in &entry.instructions {
        let kind = &body.instructions[iid.0 as usize].kind;
        match kind {
            InstKind::AllocObject | InstKind::AllocArray { .. } => {
                if alloc_value.is_some() {
                    // Multiple allocs in one body — not modeled.
                    return None;
                }
                let v = find_value_for_inst(body, iid);
                allowed_values.insert(v.0);
                alloc_value = Some(v);
                steps.push(Step {
                    kind: kind.clone(),
                    produced: Some(v.0),
                });
            }
            InstKind::StoreProp { obj, val, .. } => {
                let av = match alloc_value {
                    Some(v) => v,
                    None => return None, // store before alloc
                };
                if !operand_is(obj, av) {
                    return None;
                }
                if !operand_allowed(val, &allowed_values) {
                    return None;
                }
                steps.push(Step {
                    kind: kind.clone(),
                    produced: None,
                });
            }
            InstKind::StoreIndex {
                obj, index, val, ..
            } => {
                let av = match alloc_value {
                    Some(v) => v,
                    None => return None,
                };
                if !operand_is(obj, av) {
                    return None;
                }
                if !matches!(index, Operand::Const(_)) {
                    return None;
                }
                if !operand_allowed(val, &allowed_values) {
                    return None;
                }
                steps.push(Step {
                    kind: kind.clone(),
                    produced: None,
                });
            }
            // Expression: pure or may-throw only (no heap, no diverge,
            // not opaque). Stores/loads are excluded because they'd
            // need their own purity reasoning. Calls excluded (OPAQUE).
            other => {
                let eff = other.effect();
                if eff.intersects(!allowed_effect) {
                    return None;
                }
                if other.is_void() {
                    return None;
                }
                let mut ok = true;
                ir::ssa::walk_inst_operands(other, |op| {
                    if let Operand::Value(v) = op
                        && !allowed_values.contains(&v.0)
                    {
                        ok = false;
                    }
                });
                if !ok {
                    return None;
                }
                let produced_v = find_value_for_inst(body, iid);
                allowed_values.insert(produced_v.0);
                steps.push(Step {
                    kind: kind.clone(),
                    produced: Some(produced_v.0),
                });
            }
        }
    }

    let alloc_value = alloc_value?;

    // Terminator: Return Some(Value(alloc)).
    match &entry.terminator {
        ir::ssa::Terminator::Return {
            val: Some(Operand::Value(v)),
        } if *v == alloc_value => {}
        _ => return None,
    }

    Some(ConstructorSummary {
        alloc_body_value: alloc_value.0,
        arity,
        steps,
    })
}

fn operand_is(op: &Operand, target: Value) -> bool {
    matches!(op, Operand::Value(v) if *v == target)
}

fn operand_allowed(op: &Operand, allowed_values: &HashSet<u32>) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => allowed_values.contains(&v.0),
        _ => false, // Capture etc.: not supported by this matcher.
    }
}

/// Splice the constructor's construction sequence into the caller's
/// block at `pos`, with operands rewritten via a body-Value →
/// caller-Operand remap that handles all four substitution kinds
/// uniformly:
///   - The constructor's alloc Value → the freshly-cloned alloc Value.
///   - Each `Arg(i)` Value → `call_args[i]` (or `Const::Undefined` for
///     missing args).
///   - Each previously-cloned expression Value → its fresh caller-side
///     Value.
///   - Const operands pass through unchanged.
///
/// Steps are cloned in body order; expression steps push fresh
/// `ValueDef::inst` entries before subsequent steps reference them.
fn inline_constructor_at(
    body: &mut ir::ssa::LoweredBody,
    bi: usize,
    pos: usize,
    call_iid: InstId,
    summary: &ConstructorSummary,
) {
    let call_args: Vec<Operand> = match &body.instructions[call_iid.0 as usize].kind {
        InstKind::Call { args, .. } => args.clone(),
        _ => unreachable!("inline_constructor_at called on non-Call inst"),
    };

    // Build the body-Value → caller-Operand remap. Args map to
    // call_args[i]; expression and alloc Values get added as we walk
    // steps. The alloc step's new Value is what the Call's result
    // gets retargeted to (after step cloning).
    let mut remap: HashMap<u32, Operand> = HashMap::new();
    for i in 0..summary.arity {
        let substitute = call_args
            .get(i)
            .copied()
            .unwrap_or(Operand::Const(Constant::Undefined));
        remap.insert(i as u32, substitute);
    }

    let mut new_inst_ids: Vec<InstId> = Vec::with_capacity(summary.steps.len());

    // Walk steps in body order; alloc, expressions, and stores all go
    // through the same uniform clone path. Expression-shaped steps
    // (including the alloc) push fresh ValueDefs and feed the remap;
    // stores are void.
    for step in &summary.steps {
        let new_iid = InstId(body.instructions.len() as u32);
        let new_kind = clone_with_remap(&step.kind, &remap);
        body.instructions.push(ir::ssa::Instruction { kind: new_kind });
        if let Some(orig_body_val) = step.produced {
            body.values.push(ValueDef::inst(new_iid));
            let new_v = Value(body.values.len() as u32 - 1);
            remap.insert(orig_body_val, Operand::Value(new_v));
        }
        new_inst_ids.push(new_iid);
    }

    // Look up the cloned alloc's Value via the remap we just built.
    let new_alloc_value = match remap.get(&summary.alloc_body_value) {
        Some(Operand::Value(v)) => *v,
        _ => unreachable!(
            "matcher invariant: alloc step must have populated remap with a Value"
        ),
    };

    // 3. Splice cloned InstIds into the block immediately before the
    // Call. The Call's position shifts by len(new_inst_ids).
    let blk = &mut body.blocks[bi];
    blk.instructions.splice(pos..pos, new_inst_ids.iter().copied());
    let call_pos = pos + new_inst_ids.len();
    debug_assert_eq!(blk.instructions[call_pos].0, call_iid.0);

    // 4. Retire the Call: replace its result Value uses with the
    // cloned alloc Value, mark Removed, unhook, blank operands.
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

/// Clone an InstKind, walking every operand and substituting
/// `Operand::Value(v)` per the body→caller remap. Unmapped Values
/// would mean the matcher missed a case — debug-assert. Const and
/// Capture operands pass through unchanged.
fn clone_with_remap(src: &InstKind, remap: &HashMap<u32, Operand>) -> InstKind {
    let mut out = src.clone();
    walk_inst_operands_mut(&mut out, &mut |op| {
        if let Operand::Value(v) = op {
            match remap.get(&v.0) {
                Some(replacement) => *op = *replacement,
                None => {
                    debug_assert!(
                        false,
                        "constructor body referenced unmapped Value %{}; \
                         matcher invariant broken",
                        v.0
                    );
                }
            }
        }
    });
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

    /// Body has an opaque-effect instruction (a Call) → matcher rejects.
    /// The relaxed matcher accepts pure / may-throw expressions, but
    /// OPAQUE-effect insts (Calls, OpaqueEval, heap loads/stores not on
    /// the alloc) still disqualify because faithfully cloning them
    /// would change observable side effects relative to the wrapping
    /// Call.
    #[test]
    fn does_not_match_when_body_has_extra_instruction() {
        let mut module = Module::new();
        let key_foo;
        // Helper fn — needs to exist so we can take a Func reference
        // to it as a Call callee.
        {
            let mut b = FunctionBuilder::new(&mut module, "helper", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        // mk fn — body has alloc + Call(helper) + store + ret. The
        // Call should disqualify it.
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 0);
            key_foo = b.intern("foo");
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            let _ = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
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
        // Caller invokes the (non-matching) mk.
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let _ = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(1))),
                    args: vec![],
                })
                .unwrap();
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[2].lowered();
        let entry = f_body.entry;
        assert!(
            f_body.blocks[entry.0 as usize]
                .instructions
                .iter()
                .any(|iid| matches!(
                    f_body.instructions[iid.0 as usize].kind,
                    InstKind::Call { callee: Operand::Const(Constant::Func(FuncId(1))), .. }
                )),
            "non-matching constructor → Call to mk must remain"
        );
    }

    /// `function mk(x) { return { val: x + 1 }; }` — body has an
    /// AddGeneric whose result feeds the StoreProp. The relaxed
    /// matcher should accept this, the cloning should propagate the
    /// arithmetic into the caller, and (after the full pipeline) the
    /// load resolves to the call-arg + 1.
    #[test]
    fn inlines_constructor_with_arithmetic_step() {
        let mut module = Module::new();
        let key_val;
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 1);
            key_val = b.intern("val");
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            let plus = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(arg0),
                    rhs: Operand::Const(Constant::Number(1.0)),
                })
                .unwrap();
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_val,
                val: Operand::Value(plus),
                purity: Purity::Pure,
            });
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(alloc)),
            });
            b.finish();
        }
        // f() — calls mk(41); pulls .val.
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Const(Constant::Number(41.0))],
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
        pm.add_pass(Box::new(crate::refine_arith::RefineArithPass));
        pm.add_pass(Box::new(crate::const_fold::ConstFoldPass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 42.0, "should fold (41 + 1) → 42"),
            other => panic!("expected ret 42, got {other:?}"),
        }
    }

    /// Chained expressions: a.x = (a + 1) * 2.
    #[test]
    fn inlines_constructor_with_chained_expression_steps() {
        let mut module = Module::new();
        let key_v;
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 1);
            key_v = b.intern("v");
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            let plus = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(arg0),
                    rhs: Operand::Const(Constant::Number(1.0)),
                })
                .unwrap();
            let times = b
                .add_inst(InstKind::MulGeneric {
                    lhs: Operand::Value(plus),
                    rhs: Operand::Const(Constant::Number(2.0)),
                })
                .unwrap();
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_v,
                val: Operand::Value(times),
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
                    args: vec![Operand::Const(Constant::Number(10.0))],
                })
                .unwrap();
            let load_v = b
                .add_inst(InstKind::LoadProp {
                    obj: Operand::Value(call_v),
                    key: key_v,
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
        pm.add_pass(Box::new(crate::refine_arith::RefineArithPass));
        pm.add_pass(Box::new(crate::const_fold::ConstFoldPass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 22.0, "should fold (10 + 1) * 2 → 22"),
            other => panic!("expected ret 22, got {other:?}"),
        }
    }
}
