//! Escape / points-to analysis on `AllocObject` / `AllocArray` results.
//!
//! `RefinePurityPass` is the conservative baseline: it only refines a
//! load/store to `Purity::Pure` when the obj is a fresh local alloc whose
//! only uses are `obj`-position of a heap access. Anything else — including
//! storing the alloc into another local alloc, or branching it through a
//! block param — is treated as escape, and downstream Pure-load reasoning
//! gives up.
//!
//! This pass widens the set of provably-unaliased allocs by computing a
//! per-function points-to / containment graph:
//!
//! 1. Roots = candidate allocs (`AllocObject` / `AllocArray`, not
//!    captured by a PassThrough body).
//! 2. Each `Value` is assigned an *alias-set* — the set of root allocs it
//!    may transitively reference. Roots seed their own singleton; block
//!    params merge the alias-sets of every predecessor's edge arg.
//!    Iterate the block-param transfer to a fixed point.
//! 3. Walk every operand site. The site is either *containment*
//!    (val-position of a `StoreProp`/`StoreIndex` whose obj is a
//!    candidate alloc — this records `outer → inner` so an outer escape
//!    forces inner escape too) or *escape* (everything else: Returns,
//!    Throw, Switch disc, Call/Construct args, terminator branch args
//!    that flow into block params we already cover, unsupported inst
//!    operands, etc.).
//! 4. Seed escapes with everything in escape-position; then propagate
//!    along the containment graph (outer escapes ⇒ inner escapes).
//! 5. Refine `purity` on every heap access whose obj alias-set is
//!    non-empty *and* every alloc in that set is non-escaping.
//!
//! This pass is a refinement on top of `RefinePurityPass` — it never
//! down-refines (only `Opaque`/`Throws` → `Pure`), and the conservative
//! pass remains the soundness baseline. The two cooperate cleanly: the
//! conservative pass handles the common "alloc never leaves the obj
//! slot" case in one operand walk; this pass handles the transitive-
//! containment and block-param cases that need a fixed-point graph.
//!
//! Containment soundness sketch: when alloc `a` is `Store{Prop,Index}`d
//! into another candidate alloc `b`, `a` cannot be reached from outside
//! the function unless `b` is. We therefore mark `a` as escaping iff
//! `b` is. If the obj of the Store is a non-candidate or escapes, the
//! val operand is treated as a normal escape — equivalent to
//! `RefinePurityPass`'s baseline.

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    FuncId, InstKind, LoweredBody, Module, Operand, Purity, Terminator, Value, ValueDefKind,
    walk_inst_operands,
};

use crate::FunctionPass;

pub struct EscapeAnalysisPass;

impl FunctionPass for EscapeAnalysisPass {
    fn name(&self) -> &'static str {
        "escape_analysis"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let captured: HashSet<u32> =
            func.captured_by_passthrough.iter().copied().collect();
        let body = func.lowered_mut();

        let analysis = match EscapeAnalysis::compute(body, &captured) {
            Some(a) => a,
            None => return Ok(false),
        };

        // Refine surviving (non-escaping) allocs' heap accesses to Pure.
        let mut changed = false;
        for inst in body.instructions.iter_mut() {
            let obj = match &inst.kind {
                InstKind::LoadProp { obj, .. }
                | InstKind::LoadIndex { obj, .. }
                | InstKind::StoreProp { obj, .. }
                | InstKind::StoreIndex { obj, .. } => obj,
                _ => continue,
            };
            let Operand::Value(v) = obj else { continue };
            if !analysis.is_provably_unaliased(*v) {
                continue;
            }
            let purity = match &mut inst.kind {
                InstKind::LoadProp { purity, .. }
                | InstKind::LoadIndex { purity, .. }
                | InstKind::StoreProp { purity, .. }
                | InstKind::StoreIndex { purity, .. } => purity,
                _ => unreachable!(),
            };
            if !matches!(*purity, Purity::Pure) {
                *purity = Purity::Pure;
                changed = true;
            }
        }
        Ok(changed)
    }
}

/// Result of the per-function escape analysis. Exposes a single
/// query: `is_provably_unaliased(value)` — true iff `value` is
/// *known* to be a (tracked) candidate alloc and that alloc is
/// non-escaping.
pub struct EscapeAnalysis {
    /// Per-Value alias-set: every root alloc index this Value may
    /// transitively alias. Values with no alloc-flavored origin have
    /// an empty set.
    alias_sets: Vec<HashSet<u32>>,
    /// Allocs that escape (transitively).
    escaped: HashSet<u32>,
}

impl EscapeAnalysis {
    /// Returns `None` when the function has no candidate allocs at all
    /// (skip work entirely).
    pub fn compute(body: &LoweredBody, captured: &HashSet<u32>) -> Option<Self> {
        // Phase 1: seed candidate alloc roots.
        let mut alias_sets: Vec<HashSet<u32>> =
            (0..body.values.len()).map(|_| HashSet::new()).collect();
        let mut roots: HashSet<u32> = HashSet::new();
        for (vi, def) in body.values.iter().enumerate() {
            if let ValueDefKind::Inst(iid) = def.kind
                && matches!(
                    body.instructions[iid.0 as usize].kind,
                    InstKind::AllocObject | InstKind::AllocArray { .. }
                )
                && !captured.contains(&(vi as u32))
            {
                alias_sets[vi].insert(vi as u32);
                roots.insert(vi as u32);
            }
        }
        if roots.is_empty() {
            return None;
        }

        // Phase 2: propagate alias-sets through block-param edges to a
        // fixed point. For each predecessor edge that targets a block
        // with params, the i-th edge arg's alias-set unions into the
        // i-th param's alias-set.
        loop {
            let mut changed = false;
            for blk in &body.blocks {
                for (succ, args) in blk.terminator.successors() {
                    let dst = &body.blocks[succ.0 as usize];
                    if dst.params.is_empty() {
                        continue;
                    }
                    // Edges with mismatched arity are a validation error;
                    // skip rather than panic — validate_between catches it.
                    for (i, arg) in args.iter().enumerate() {
                        if i >= dst.params.len() {
                            break;
                        }
                        let src_set = match arg {
                            Operand::Value(v) => alias_sets[v.0 as usize].clone(),
                            _ => continue,
                        };
                        let dst_v = dst.params[i].0 as usize;
                        for r in src_set {
                            if alias_sets[dst_v].insert(r) {
                                changed = true;
                            }
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }

        // Phase 3: walk every operand. Classify each operand as either
        // a *containment* edge (val of a Store into a candidate-alloc
        // container) or an *escape* (everything else). Build the
        // containment graph and the initial escape set.
        //
        // Containment edges go outer → inner: when outer escapes, inner
        // does too. We record edges as a HashMap<outer_root, Vec<inner_root>>.
        let mut escaped: HashSet<u32> = HashSet::new();
        let mut contains: HashMap<u32, HashSet<u32>> = HashMap::new();

        for blk in &body.blocks {
            for &iid in &blk.instructions {
                let inst = &body.instructions[iid.0 as usize];
                match &inst.kind {
                    InstKind::StoreProp { obj, val, .. } => {
                        handle_store(
                            obj,
                            val,
                            &alias_sets,
                            &roots,
                            &mut escaped,
                            &mut contains,
                        );
                        // index/key non-Value: nothing.
                    }
                    InstKind::StoreIndex {
                        obj, index, val, ..
                    } => {
                        handle_store(
                            obj,
                            val,
                            &alias_sets,
                            &roots,
                            &mut escaped,
                            &mut contains,
                        );
                        // Index is escape-position — but it's almost
                        // never an alloc; conservatively treat it as
                        // escape.
                        if let Operand::Value(v) = index {
                            escape_value(*v, &alias_sets, &mut escaped);
                        }
                    }
                    InstKind::LoadProp { obj, .. } => {
                        // obj is heap-access position — never escape.
                        let _ = obj;
                    }
                    InstKind::LoadIndex { obj, index, .. } => {
                        let _ = obj;
                        if let Operand::Value(v) = index {
                            escape_value(*v, &alias_sets, &mut escaped);
                        }
                    }
                    // ForInInit's `obj` is heap-read position: the
                    // iterator handle the inst returns is opaque and
                    // doesn't expose any reference to `obj`, so the
                    // alloc doesn't escape through it. (The iter
                    // value is consumed by ForInDone / ForInNext,
                    // both of which only read iterator state.)
                    InstKind::ForInInit { obj, .. } => {
                        let _ = obj;
                    }
                    // ForInDone / ForInNext take only the iterator
                    // handle. The handle's alias-set is empty (it's
                    // not itself a tracked alloc), so even if these
                    // fell into the default branch it would be a
                    // no-op for alloc escape; explicit-skip for
                    // clarity.
                    InstKind::ForInDone { .. } | InstKind::ForInNext { .. } => {}
                    other => {
                        // Every operand of every other inst is escape-position.
                        walk_inst_operands(other, |op| {
                            if let Operand::Value(v) = op {
                                escape_value(*v, &alias_sets, &mut escaped);
                            }
                        });
                    }
                }
            }

            // Terminator: branch args are *not* escapes — we already
            // modeled them as block-param flow above. For
            // Return/Throw/Switch-disc/CondBranch-cond, the operand is
            // truly escape-position.
            walk_term_escape_operands(&blk.terminator, |v| {
                escape_value(v, &alias_sets, &mut escaped);
            });
        }

        // Phase 4: propagate escape through containment.
        let mut worklist: Vec<u32> = escaped.iter().copied().collect();
        while let Some(outer) = worklist.pop() {
            if let Some(inners) = contains.get(&outer) {
                for &inner in inners {
                    if escaped.insert(inner) {
                        worklist.push(inner);
                    }
                }
            }
        }

        Some(Self {
            alias_sets,
            escaped,
        })
    }

    /// True iff `value`'s alias-set is non-empty *and* every alloc in
    /// the set is non-escaping. Empty alias-set returns `false` —
    /// the value isn't known to be a tracked alloc, so we cannot speak
    /// to its aliasing.
    pub fn is_provably_unaliased(&self, value: Value) -> bool {
        let set = &self.alias_sets[value.0 as usize];
        if set.is_empty() {
            return false;
        }
        set.iter().all(|r| !self.escaped.contains(r))
    }
}

fn escape_value(v: Value, alias_sets: &[HashSet<u32>], escaped: &mut HashSet<u32>) {
    for r in &alias_sets[v.0 as usize] {
        escaped.insert(*r);
    }
}

fn handle_store(
    obj: &Operand,
    val: &Operand,
    alias_sets: &[HashSet<u32>],
    roots: &HashSet<u32>,
    escaped: &mut HashSet<u32>,
    contains: &mut HashMap<u32, HashSet<u32>>,
) {
    // A Store into globalThis (or any non-Value obj) must be treated
    // as: val operand escapes. Same for stores into a Value whose
    // alias-set is empty (unknown obj — could be anything, including
    // globalThis after a load).
    let outer_set = match obj {
        Operand::Value(v) => &alias_sets[v.0 as usize],
        _ => {
            if let Operand::Value(vv) = val {
                escape_value(*vv, alias_sets, escaped);
            }
            return;
        }
    };

    if outer_set.is_empty() {
        // Unknown container. val escapes.
        if let Operand::Value(vv) = val {
            escape_value(*vv, alias_sets, escaped);
        }
        return;
    }

    // Container is one or more candidate allocs. Record containment
    // edges from each outer to each alias of val.
    let val_v = match val {
        Operand::Value(v) => *v,
        _ => return,
    };
    let inner_set = &alias_sets[val_v.0 as usize];
    if inner_set.is_empty() {
        // val is not an alloc — nothing to containerize.
        return;
    }
    for &outer in outer_set {
        if !roots.contains(&outer) {
            continue;
        }
        let entry = contains.entry(outer).or_default();
        for &inner in inner_set {
            entry.insert(inner);
        }
    }
}

/// Walks the *escape*-position operands of a terminator: everything
/// except branch-arg operands (those are modeled via block-param
/// alias-set propagation).
fn walk_term_escape_operands(t: &Terminator, mut f: impl FnMut(Value)) {
    match t {
        Terminator::Branch { .. } => {}
        Terminator::CondBranch { cond, .. } => {
            if let Operand::Value(v) = cond {
                f(*v);
            }
        }
        Terminator::Switch { val, .. } => {
            if let Operand::Value(v) = val {
                f(*v);
            }
        }
        Terminator::Return { val: Some(op) } => {
            if let Operand::Value(v) = op {
                f(*v);
            }
        }
        Terminator::Return { val: None } => {}
        Terminator::Throw { val } => {
            if let Operand::Value(v) = val {
                f(*v);
            }
        }
        Terminator::Try { .. } => {
            // try_args flow into try_entry's block params (handled by
            // block-param propagation, not as escape). catch_entry /
            // finally_entry take no edge args.
        }
        Terminator::Unreachable => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
    };

    fn collect_purities(body: &ir::ssa::LoweredBody) -> (Vec<Purity>, Vec<Purity>) {
        let mut loads = Vec::new();
        let mut stores = Vec::new();
        for inst in &body.instructions {
            match &inst.kind {
                InstKind::LoadProp { purity, .. } | InstKind::LoadIndex { purity, .. } => {
                    loads.push(*purity)
                }
                InstKind::StoreProp { purity, .. } | InstKind::StoreIndex { purity, .. } => {
                    stores.push(*purity)
                }
                _ => {}
            }
        }
        (loads, stores)
    }

    /// Transitive containment: outer = AllocObject; inner = AllocObject;
    /// `outer.x = inner`; `inner.y = 1`. Neither escapes — both alloc's
    /// heap accesses must refine to Pure.
    #[test]
    fn nested_containment_neither_escapes() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let key_y = b.intern("y");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let outer = b.add_inst(InstKind::AllocObject).unwrap();
        let inner = b.add_inst(InstKind::AllocObject).unwrap();
        // outer.x = inner
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(outer),
            key: key_x,
            val: Operand::Value(inner),
            purity: Purity::Opaque,
        });
        // inner.y = 1
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(inner),
            key: key_y,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(EscapeAnalysisPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let (_loads, stores) = collect_purities(module.functions[0].lowered());
        assert_eq!(stores, vec![Purity::Pure, Purity::Pure]);
    }

    /// Outer escapes via Return; inner is contained in outer → inner
    /// also escapes; neither's heap access refines.
    #[test]
    fn nested_outer_returned_propagates_inner_escape() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let key_y = b.intern("y");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let outer = b.add_inst(InstKind::AllocObject).unwrap();
        let inner = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(outer),
            key: key_x,
            val: Operand::Value(inner),
            purity: Purity::Opaque,
        });
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(inner),
            key: key_y,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(outer)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(EscapeAnalysisPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let (_loads, stores) = collect_purities(module.functions[0].lowered());
        // Both stores must remain non-Pure (Opaque).
        assert!(stores.iter().all(|p| matches!(p, Purity::Opaque)));
    }

    /// alloc passed to a Call → escapes; no refinement.
    #[test]
    fn call_arg_escapes() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_use = b.intern("use");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let callee = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Const(Constant::GlobalThis),
                key: key_use,
                purity: Purity::Throws,
            })
            .unwrap();
        b.add_inst(InstKind::Call {
            callee: Operand::Value(callee),
            args: vec![Operand::Value(alloc)],
        });
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(EscapeAnalysisPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        // No StoreProp/LoadProp on the alloc to refine — but verify the
        // analysis classifies it as escaped via the public query.
        let func = module.functions[0].lowered();
        let captured = HashSet::new();
        let analysis = EscapeAnalysis::compute(func, &captured).unwrap();
        assert!(
            !analysis.is_provably_unaliased(alloc),
            "alloc passed to Call must be escaping"
        );
    }

    /// alloc returned → escapes.
    #[test]
    fn returned_alloc_escapes() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(alloc)),
        });
        b.finish();

        let func = module.functions[0].lowered();
        let captured = HashSet::new();
        let analysis = EscapeAnalysis::compute(func, &captured).unwrap();
        assert!(!analysis.is_provably_unaliased(alloc));
    }

    /// Storing into globalThis (`globalThis.x = alloc`) escapes.
    #[test]
    fn store_to_globalthis_escapes() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Const(Constant::GlobalThis),
            key: key_x,
            val: Operand::Value(alloc),
            purity: Purity::Throws,
        });
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let func = module.functions[0].lowered();
        let captured = HashSet::new();
        let analysis = EscapeAnalysis::compute(func, &captured).unwrap();
        assert!(!analysis.is_provably_unaliased(alloc));
    }

    /// alloc flows through a block param into a non-escaping consumer.
    /// The block-param Value should be provably unaliased (its alias-set
    /// is exactly the alloc).
    #[test]
    fn block_param_propagates_alias_set() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let param = b.add_block_param(bb1);
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![Operand::Value(alloc)],
        });
        b.switch_to_block(bb1);
        // Use the block param as obj of a StoreProp (escape-free use).
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(param),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(EscapeAnalysisPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        // The StoreProp on the block-param should be refined to Pure.
        let store_purity = body
            .instructions
            .iter()
            .find_map(|i| match &i.kind {
                InstKind::StoreProp { purity, .. } => Some(*purity),
                _ => None,
            })
            .unwrap();
        assert_eq!(store_purity, Purity::Pure);
    }

    /// alloc passed to a block param whose successor returns it → escapes.
    #[test]
    fn block_param_then_return_escapes() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let param = b.add_block_param(bb1);
        b.switch_to_block(bb0);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![Operand::Value(alloc)],
        });
        b.switch_to_block(bb1);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(param)),
        });
        b.finish();

        let func = module.functions[0].lowered();
        let captured = HashSet::new();
        let analysis = EscapeAnalysis::compute(func, &captured).unwrap();
        assert!(
            !analysis.is_provably_unaliased(alloc),
            "alloc returned via block-param edge must escape"
        );
        assert!(
            !analysis.is_provably_unaliased(param),
            "block-param aliasing the escaped alloc is also tainted"
        );
    }
}
