//! Single-block clonable-body inlining — sibling to `IdentityFnInlinePass`.
//!
//! Recognises functions whose body is a bounded, side-effect-tame
//! sequence we can splice at every Call site, and rewrites those Calls
//! by cloning the body into the caller. After cloning, downstream
//! function passes (`ForwardLoadsPass`, `ConstFoldPass`, `DcePass`,
//! …) collapse anything that resolves.
//!
//! This pass started life as plain "constructor-function inlining"
//! (a function returning a freshly-built alloc) but generalises to
//! any single-block body whose instructions stay within a permitted
//! language. Specifically, an instruction is admissible iff it is one
//! of:
//!
//! - `AllocObject` / `AllocArray` — produces a *tracked alloc*. Each
//!   call site gets its own fresh alloc Value, preserving JS
//!   identity-per-call semantics.
//! - `StoreProp` / `StoreIndex` whose `obj` is a tracked alloc, key is
//!   `Const(_)`, and `val` is "allowed" (Const, Arg, or a previously-
//!   produced body Value).
//! - `LoadProp` / `LoadIndex` whose `obj` is a tracked alloc. The
//!   tracked-alloc invariant guarantees this is a Pure load post-clone
//!   (the cloned alloc is unaliased in the caller), so `ForwardLoads`
//!   can resolve it whenever the key is also resolvable.
//! - A `Call` whose callee is `Const(Func(g))` with `g` already in the
//!   summaries map. Treated as a *recursive expansion site*: at clone
//!   time, the callee's summary is spliced in instead of cloning the
//!   Call literally. If the callee `returns_alloc`, the produced Value
//!   here also becomes a tracked alloc.
//! - Any other "expression" instruction whose effect is a subset of
//!   `PURE | MAY_THROW` and whose operands are all allowed (no heap
//!   mutation, no diverge, not opaque).
//!
//! The terminator must be `Return(Some(Value(v)))` where `v` is some
//! body Value seen above. The summary records `returns_alloc` based on
//! whether `v` is itself a tracked alloc — this propagates through
//! recursive Calls so chains like `decoder1 → decoder2 → mkArr` all
//! collapse.
//!
//! Phase 1 is a fixed point over the function set: functions whose
//! bodies match the above (using the current summaries map for the
//! Call-to-clonable rule) get added to the table; iterate until stable.
//! Bound is the function count.
//!
//! Phase 2 walks every function body and rewrites each Call to a known
//! summary by recursively splicing.
//!
//! Soundness notes:
//! - Each Call site receives freshly-cloned alloc Values; identity
//!   semantics preserved.
//! - The tracked-alloc invariant is what makes `LoadProp/LoadIndex`
//!   admissible: at the matcher we only accept loads whose `obj` is in
//!   the per-body tracked-alloc set, and at clone time those allocs are
//!   freshly allocated *in the caller*, so the loads remain Pure.
//! - Calls to non-clonable functions disqualify (would change observable
//!   side effects relative to the wrapping Call).

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    Constant, Effect, FunctionBody, InstId, InstKind, Module, Operand, Value, ValueDef,
    ValueDefKind, find_value_for_inst, walk_inst_operands_mut,
};

use crate::ModulePass;

pub struct ConstructorFnInlinePass;

#[derive(Clone, Debug)]
struct Step {
    kind: InstKind,
    /// `Some(body_value_idx)` if this step produces a Value.
    produced: Option<u32>,
    /// `Some(target_fid)` for `Call`-to-clonable steps; expanded
    /// recursively at clone time instead of cloned literally.
    recurse_target: Option<u32>,
}

#[derive(Clone, Debug)]
struct InlineSummary {
    arity: usize,
    steps: Vec<Step>,
    /// Body Value index returned by the function.
    return_body_value: u32,
    /// True iff the return Value is a tracked alloc — used by callers'
    /// matchers to admit stores/loads against this function's result,
    /// and to chain through nested clonable-Call returns.
    returns_alloc: bool,
}

impl ModulePass for ConstructorFnInlinePass {
    fn name(&self) -> &'static str {
        "constructor_fn_inline"
    }

    fn run(&mut self, module: &mut Module) -> Result<bool, Error> {
        // Phase 1: fixed-point summary build. Each iteration grows the
        // table by trying to match every not-yet-summarised function
        // against the current table; stable when no new entries.
        let mut summaries: HashMap<u32, InlineSummary> = HashMap::new();
        loop {
            let mut grew = false;
            for (i, func) in module.functions.iter().enumerate() {
                let fid = i as u32;
                if summaries.contains_key(&fid) {
                    continue;
                }
                if let Some(s) = match_inlinable(func, &summaries) {
                    summaries.insert(fid, s);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }

        if summaries.is_empty() {
            return Ok(false);
        }

        // Phase 2: rewrite Call sites in every function body.
        let mut any_changed = false;
        let n_funcs = module.functions.len();
        for fid_idx in 0..n_funcs {
            let body = match &mut module.functions[fid_idx].body {
                FunctionBody::Lowered(b) => b,
                FunctionBody::PassThrough(_) => continue,
            };

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

            rewrites.sort_by_key(|&(bi, pos, _, _)| (bi, std::cmp::Reverse(pos)));

            for (bi, pos, call_iid, target_fid) in rewrites {
                let summary = summaries.get(&target_fid).unwrap().clone();
                inline_at_call_site(body, bi, pos, InstId(call_iid), &summary, &summaries);
                any_changed = true;
            }
        }
        Ok(any_changed)
    }
}

/// Permissive shape predicate exposed for `DeadFnElimPass`: returns true
/// iff `func`'s body could plausibly be a clonable wrapper (single
/// block, no params, instructions in the admissible language). Does
/// NOT verify that inner Calls reference clonable functions — that's
/// fine for DeadFnElim's purposes: a function whose inner Calls
/// reference non-clonable callees won't have its own callers inlined
/// either (this pass would never produce a summary for it), so its
/// `Const(Func)` uses won't drop to zero, and DeadFnElim won't try.
pub fn is_inlinable_shape(func: &ir::ssa::Function) -> bool {
    match_shape_only(func)
}

/// Back-compat alias — kept for any external callers that still
/// reference the old name.
pub fn is_constructor_shape(func: &ir::ssa::Function) -> bool {
    is_inlinable_shape(func)
}

fn match_shape_only(func: &ir::ssa::Function) -> bool {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return false,
    };
    if body.blocks.len() != 1 {
        return false;
    }
    let entry = &body.blocks[body.entry.0 as usize];
    if !entry.params.is_empty() || entry.instructions.is_empty() {
        return false;
    }

    let mut allowed_values: HashSet<u32> = HashSet::new();
    let mut tracked_allocs: HashSet<u32> = HashSet::new();
    for v_idx in 0..body.values.len() {
        if let ValueDefKind::Arg(_) = body.values[v_idx].kind {
            allowed_values.insert(v_idx as u32);
        }
    }

    let allowed_effect = Effect::PURE | Effect::MAY_THROW;
    for &iid in &entry.instructions {
        let kind = &body.instructions[iid.0 as usize].kind;
        if !walk_admissible(kind, body, iid, &mut allowed_values, &mut tracked_allocs, allowed_effect, /*permissive_calls=*/ true) {
            return false;
        }
    }
    matches!(
        &entry.terminator,
        ir::ssa::Terminator::Return { val: Some(Operand::Value(v))} if allowed_values.contains(&v.0)
    )
}

/// Match the inlinable shape on `func` against the current summaries
/// map; returns the summary if it matches.
fn match_inlinable(
    func: &ir::ssa::Function,
    summaries: &HashMap<u32, InlineSummary>,
) -> Option<InlineSummary> {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    if body.blocks.len() != 1 {
        return None;
    }
    let entry = &body.blocks[body.entry.0 as usize];
    if !entry.params.is_empty() || entry.instructions.is_empty() {
        return None;
    }

    let arity = body.params.len();
    let mut allowed_values: HashSet<u32> = HashSet::new();
    let mut tracked_allocs: HashSet<u32> = HashSet::new();
    for v_idx in 0..body.values.len() {
        if let ValueDefKind::Arg(_) = body.values[v_idx].kind {
            allowed_values.insert(v_idx as u32);
        }
    }

    let mut steps: Vec<Step> = Vec::new();
    let allowed_effect = Effect::PURE | Effect::MAY_THROW;

    for &iid in &entry.instructions {
        let kind = &body.instructions[iid.0 as usize].kind;
        match kind {
            InstKind::AllocObject | InstKind::AllocArray { .. } => {
                let v = find_value_for_inst(body, iid);
                allowed_values.insert(v.0);
                tracked_allocs.insert(v.0);
                steps.push(Step {
                    kind: kind.clone(),
                    produced: Some(v.0),
                    recurse_target: None,
                });
            }
            InstKind::StoreProp { obj, val, .. } => {
                let obj_v = match obj {
                    Operand::Value(v) => v.0,
                    _ => return None,
                };
                if !tracked_allocs.contains(&obj_v) {
                    return None;
                }
                if !operand_allowed(val, &allowed_values) {
                    return None;
                }
                steps.push(Step {
                    kind: kind.clone(),
                    produced: None,
                    recurse_target: None,
                });
            }
            InstKind::StoreIndex { obj, index, val, .. } => {
                let obj_v = match obj {
                    Operand::Value(v) => v.0,
                    _ => return None,
                };
                if !tracked_allocs.contains(&obj_v) {
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
                    recurse_target: None,
                });
            }
            InstKind::LoadProp { obj, .. } => {
                let obj_v = match obj {
                    Operand::Value(v) => v.0,
                    _ => return None,
                };
                if !tracked_allocs.contains(&obj_v) {
                    return None;
                }
                let v = find_value_for_inst(body, iid);
                allowed_values.insert(v.0);
                steps.push(Step {
                    kind: kind.clone(),
                    produced: Some(v.0),
                    recurse_target: None,
                });
            }
            InstKind::LoadIndex { obj, index, .. } => {
                let obj_v = match obj {
                    Operand::Value(v) => v.0,
                    _ => return None,
                };
                if !tracked_allocs.contains(&obj_v) {
                    return None;
                }
                if !operand_allowed(index, &allowed_values) {
                    return None;
                }
                let v = find_value_for_inst(body, iid);
                allowed_values.insert(v.0);
                steps.push(Step {
                    kind: kind.clone(),
                    produced: Some(v.0),
                    recurse_target: None,
                });
            }
            InstKind::Call { callee, args } => {
                let target_fid = match callee {
                    Operand::Const(Constant::Func(fid)) => fid.0,
                    _ => return None,
                };
                let inner = summaries.get(&target_fid)?;
                for a in args {
                    if !operand_allowed(a, &allowed_values) {
                        return None;
                    }
                }
                let v = find_value_for_inst(body, iid);
                allowed_values.insert(v.0);
                if inner.returns_alloc {
                    tracked_allocs.insert(v.0);
                }
                steps.push(Step {
                    kind: kind.clone(),
                    produced: Some(v.0),
                    recurse_target: Some(target_fid),
                });
            }
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
                    recurse_target: None,
                });
            }
        }
    }

    let return_body_value = match &entry.terminator {
        ir::ssa::Terminator::Return {
            val: Some(Operand::Value(v)),
        } => v.0,
        _ => return None,
    };
    if !allowed_values.contains(&return_body_value) {
        return None;
    }
    let returns_alloc = tracked_allocs.contains(&return_body_value);

    Some(InlineSummary {
        arity,
        steps,
        return_body_value,
        returns_alloc,
    })
}

/// Single-instruction shape check shared between match_shape_only
/// (permissive — accepts any Call-to-Func without summary lookup) and
/// future variants. Updates the running allowed/tracked sets in place.
fn walk_admissible(
    kind: &InstKind,
    body: &ir::ssa::LoweredBody,
    iid: InstId,
    allowed_values: &mut HashSet<u32>,
    tracked_allocs: &mut HashSet<u32>,
    allowed_effect: Effect,
    permissive_calls: bool,
) -> bool {
    match kind {
        InstKind::AllocObject | InstKind::AllocArray { .. } => {
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            tracked_allocs.insert(v.0);
            true
        }
        InstKind::StoreProp { obj, val, .. } => {
            let Operand::Value(o) = obj else { return false };
            if !tracked_allocs.contains(&o.0) {
                return false;
            }
            operand_allowed(val, allowed_values)
        }
        InstKind::StoreIndex { obj, index, val, .. } => {
            let Operand::Value(o) = obj else { return false };
            if !tracked_allocs.contains(&o.0) {
                return false;
            }
            matches!(index, Operand::Const(_)) && operand_allowed(val, allowed_values)
        }
        InstKind::LoadProp { obj, .. } => {
            let Operand::Value(o) = obj else { return false };
            if !tracked_allocs.contains(&o.0) {
                return false;
            }
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            true
        }
        InstKind::LoadIndex { obj, index, .. } => {
            let Operand::Value(o) = obj else { return false };
            if !tracked_allocs.contains(&o.0) {
                return false;
            }
            if !operand_allowed(index, allowed_values) {
                return false;
            }
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            true
        }
        InstKind::Call { callee, args } => {
            if !permissive_calls {
                return false;
            }
            if !matches!(callee, Operand::Const(Constant::Func(_))) {
                return false;
            }
            for a in args {
                if !operand_allowed(a, allowed_values) {
                    return false;
                }
            }
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            // Permissive shape: assume callee returns alloc-like
            // result (it's only used by DeadFnElim to identify wrapper
            // shapes, where over-acceptance is fine — see
            // is_inlinable_shape doc).
            tracked_allocs.insert(v.0);
            true
        }
        other => {
            let eff = other.effect();
            if eff.intersects(!allowed_effect) {
                return false;
            }
            if other.is_void() {
                return false;
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
                return false;
            }
            let produced_v = find_value_for_inst(body, iid);
            allowed_values.insert(produced_v.0);
            true
        }
    }
}

fn operand_allowed(op: &Operand, allowed_values: &HashSet<u32>) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => allowed_values.contains(&v.0),
        _ => false,
    }
}

/// Splice `summary` into the caller at `pos`, retire the original Call,
/// and replace the Call's result Value uses with the cloned return.
fn inline_at_call_site(
    body: &mut ir::ssa::LoweredBody,
    bi: usize,
    pos: usize,
    call_iid: InstId,
    summary: &InlineSummary,
    summaries: &HashMap<u32, InlineSummary>,
) {
    let call_args: Vec<Operand> = match &body.instructions[call_iid.0 as usize].kind {
        InstKind::Call { args, .. } => args.clone(),
        _ => unreachable!("inline_at_call_site called on non-Call inst"),
    };
    let (new_iids, return_op) = build_clone(body, summary, &call_args, summaries);

    let blk = &mut body.blocks[bi];
    blk.instructions.splice(pos..pos, new_iids.iter().copied());
    let call_pos = pos + new_iids.len();
    debug_assert_eq!(blk.instructions[call_pos].0, call_iid.0);

    let call_result_v = find_value_for_inst(body, call_iid);
    body.replace_value_uses(call_result_v, return_op);
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

/// Materialise `summary`'s body into the caller's instruction/value
/// arrays, with operands remapped from inner-body Values to caller-side
/// Operands. Recurses through `recurse_target` steps by splicing the
/// inner summary instead of cloning the Call literally.
///
/// Returns: (the InstIds appended, in body-order; the caller-side
/// Operand the inner function "returns").
fn build_clone(
    body: &mut ir::ssa::LoweredBody,
    summary: &InlineSummary,
    caller_args: &[Operand],
    summaries: &HashMap<u32, InlineSummary>,
) -> (Vec<InstId>, Operand) {
    let mut remap: HashMap<u32, Operand> = HashMap::new();
    for i in 0..summary.arity {
        let sub = caller_args
            .get(i)
            .copied()
            .unwrap_or(Operand::Const(Constant::Undefined));
        remap.insert(i as u32, sub);
    }

    let mut new_iids: Vec<InstId> = Vec::new();
    for step in &summary.steps {
        if let Some(target_fid) = step.recurse_target {
            // Translate the Call's inner-body args through the current
            // remap into caller-side Operands.
            let inner_call_args: Vec<Operand> = match &step.kind {
                InstKind::Call { args, .. } => args.clone(),
                _ => unreachable!("recurse_target step must wrap a Call"),
            };
            let translated: Vec<Operand> = inner_call_args
                .iter()
                .map(|a| match a {
                    Operand::Value(v) => remap
                        .get(&v.0)
                        .copied()
                        .unwrap_or(Operand::Const(Constant::Undefined)),
                    other => *other,
                })
                .collect();
            let inner_summary = summaries
                .get(&target_fid)
                .expect("recurse_target must be in summaries")
                .clone();
            let (inner_iids, inner_return_op) =
                build_clone(body, &inner_summary, &translated, summaries);
            new_iids.extend(inner_iids);
            if let Some(orig) = step.produced {
                remap.insert(orig, inner_return_op);
            }
        } else {
            let new_iid = InstId(body.instructions.len() as u32);
            let new_kind = clone_with_remap(&step.kind, &remap);
            body.instructions
                .push(ir::ssa::Instruction { kind: new_kind });
            if let Some(orig) = step.produced {
                body.values.push(ValueDef::inst(new_iid));
                let new_v = Value(body.values.len() as u32 - 1);
                remap.insert(orig, Operand::Value(new_v));
            }
            new_iids.push(new_iid);
        }
    }

    let return_op = remap
        .get(&summary.return_body_value)
        .copied()
        .expect("matcher invariant: return value must be in remap");
    (new_iids, return_op)
}

fn clone_with_remap(src: &InstKind, remap: &HashMap<u32, Operand>) -> InstKind {
    let mut out = src.clone();
    walk_inst_operands_mut(&mut out, &mut |op| {
        if let Operand::Value(v) = op {
            match remap.get(&v.0) {
                Some(replacement) => *op = *replacement,
                None => {
                    debug_assert!(
                        false,
                        "inlinable body referenced unmapped Value %{}; \
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

    #[test]
    fn inlines_array_constructor_then_forward_loads_resolves_index() {
        let mut module = Module::new();
        let s_a;
        let s_b;
        let s_c;
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

    /// Body has a Call to a non-clonable function (a function that
    /// itself doesn't match the inlinable shape) → matcher rejects.
    #[test]
    fn does_not_match_when_body_calls_non_clonable() {
        let mut module = Module::new();
        let key_foo;
        // helper has two blocks → not clonable.
        {
            let mut b = FunctionBuilder::new(&mut module, "helper", 0);
            let bb0 = b.new_block();
            let bb1 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Branch { target: bb1, args: vec![] });
            b.switch_to_block(bb1);
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
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
            "non-matching mk → Call to mk must remain"
        );
    }

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

    /// **The decoder shape (A.1)**:
    /// `function mkArr() { return ['a','b','c']; }`
    /// `function dec(i) { return mkArr()[i]; }`
    /// `function f() { return dec(1); }`
    /// → after the pipeline, `f` returns "b" directly.
    /// dec's body has a Call to mkArr (a constructor) and a LoadIndex
    /// against its result — the matcher's recursive Call-to-clonable
    /// rule + tracked-alloc-through-call rule make dec also clonable.
    /// At f's call site, both functions get spliced and the load
    /// resolves.
    #[test]
    fn inlines_decoder_that_calls_constructor_then_indexes() {
        let mut module = Module::new();
        let s_a;
        let s_b;
        let s_c;
        // FuncId(0) mkArr() → ['a','b','c']
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
        // FuncId(1) dec(i) → mkArr()[i]
        {
            let mut b = FunctionBuilder::new(&mut module, "dec", 1);
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let inner_call = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
                })
                .unwrap();
            let load_v = b
                .add_inst(InstKind::LoadIndex {
                    obj: Operand::Value(inner_call),
                    index: Operand::Value(arg0),
                    purity: Purity::Pure,
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(load_v)),
            });
            b.finish();
        }
        // FuncId(2) f() → dec(1)
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(1))),
                    args: vec![Operand::Const(Constant::Int32(1))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        pm.add_pass(Box::new(crate::forward_loads::ForwardLoadsPass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[2].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::String(id))),
            } => assert_eq!(module.strings.resolve(id), "b"),
            other => panic!("expected ret String('b'), got {other:?}"),
        }
    }

    /// Decoder with arithmetic on the index: `dec(i) { return
    /// mkArr()[i + 1]; }; f() { return dec(0); }` → "b".
    #[test]
    fn decoder_with_index_arithmetic() {
        let mut module = Module::new();
        let s_a;
        let s_b;
        let s_c;
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
        {
            let mut b = FunctionBuilder::new(&mut module, "dec", 1);
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let inner_call = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
                })
                .unwrap();
            let key = b
                .add_inst(InstKind::AddGeneric {
                    lhs: Operand::Value(arg0),
                    rhs: Operand::Const(Constant::Int32(1)),
                })
                .unwrap();
            let load_v = b
                .add_inst(InstKind::LoadIndex {
                    obj: Operand::Value(inner_call),
                    index: Operand::Value(key),
                    purity: Purity::Pure,
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(load_v)),
            });
            b.finish();
        }
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(1))),
                    args: vec![Operand::Const(Constant::Int32(0))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        pm.add_pass(Box::new(crate::refine_arith::RefineArithPass));
        pm.add_pass(Box::new(crate::const_fold::ConstFoldPass));
        pm.add_pass(Box::new(crate::forward_loads::ForwardLoadsPass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[2].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::String(id))),
            } => assert_eq!(module.strings.resolve(id), "b"),
            other => panic!("expected ret String('b'), got {other:?}"),
        }
    }

    /// Layered decoders: mkArr → dec1 → dec2 → f. Internal fixed point
    /// must accept dec2 (calls dec1 → calls mkArr) by iterating the
    /// summary table.
    #[test]
    fn inlines_layered_decoders() {
        let mut module = Module::new();
        let s_a;
        let s_b;
        let s_c;
        // FuncId(0) mkArr
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
        // FuncId(1) dec1(i) = mkArr() — wrapper returning alloc.
        {
            let mut b = FunctionBuilder::new(&mut module, "dec1", 1);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let inner = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(inner)),
            });
            b.finish();
        }
        // FuncId(2) dec2(i) = dec1(i)[i]
        {
            let mut b = FunctionBuilder::new(&mut module, "dec2", 1);
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let inner = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(1))),
                    args: vec![Operand::Value(arg0)],
                })
                .unwrap();
            let load_v = b
                .add_inst(InstKind::LoadIndex {
                    obj: Operand::Value(inner),
                    index: Operand::Value(arg0),
                    purity: Purity::Pure,
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(load_v)),
            });
            b.finish();
        }
        // FuncId(3) f() = dec2(2)
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(2))),
                    args: vec![Operand::Const(Constant::Int32(2))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        pm.add_pass(Box::new(crate::forward_loads::ForwardLoadsPass));
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[3].lowered();
        let entry = f_body.entry;
        match &f_body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::String(id))),
            } => assert_eq!(module.strings.resolve(id), "c"),
            other => panic!("expected ret String('c'), got {other:?}"),
        }
    }
}
