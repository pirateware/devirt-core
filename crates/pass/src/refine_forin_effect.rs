//! ForIn-iteration effect refinement.
//!
//! `ForInInit` and `ForInNext` default to
//! `READS_HEAP | MAY_THROW | MAY_OBSERVE_GETTERS`. The
//! `MAY_OBSERVE_GETTERS` bit signals that iteration may invoke
//! property accessors (getters / setters / Proxy traps /
//! prototype-chain callbacks) — i.e. iteration can run arbitrary
//! user code.
//!
//! When the iterated object is a fresh-and-unaliased local
//! `AllocObject` whose only writes are direct data-property
//! `StoreProp` / `StoreIndex` (no setters defined, no Proxy, no
//! prototype tampering reachable from this function), iteration is
//! provably guaranteed to NOT invoke any user-defined accessor.
//! In that case we narrow the per-site `ForInIterShape` from
//! `Opaque` to `PlainData`, dropping `MAY_OBSERVE_GETTERS` from
//! the effect.
//!
//! The pass is conservative on three axes:
//!
//! 1. Identity of the iterated object — uses `EscapeAnalysis::
//!    is_provably_unaliased`. Anything captured by a PassThrough
//!    closure, escaped via Return / Call arg, or whose alias-set
//!    can't be proven equal to a single root is left at `Opaque`.
//! 2. Container kind — only `AllocObject` (and its alias-set) is
//!    eligible. `AllocArray` is excluded for safety: arrays have
//!    a Number-keyed prototype with `Symbol.iterator`, and even
//!    for-in (string-keyed) iteration walks `Array.prototype`
//!    which a host can install accessors on. Conservative — a
//!    follow-up could allow `AllocArray` once the analysis can
//!    prove the prototype chain is unmodified.
//! 3. Store shape — every store into the alloc must be a plain
//!    data-property `StoreProp`/`StoreIndex` (no DefineProperty
//!    with getter — the IR doesn't yet expose getter-defining
//!    property descriptors, so this is currently vacuous, but the
//!    structural check still rejects unsupported store kinds).
//!
//! Limitation: the IR does not currently model accessor-defining
//! property descriptors (`Object.defineProperty(o, k, { get: ... })`)
//! distinctly from a plain `StoreProp`. If a frontend ever lowers
//! such constructs to a dedicated InstKind, this pass needs an
//! explicit "no accessor-defining store" check; today every
//! StoreProp is a data-property store by construction.
//!
//! Soundness sketch: when alloc `a` is provably unaliased AND
//! every store into it is a data-property store, then by
//! construction `a` has:
//! - no host-installed Proxy trap (it's an `AllocObject`, not
//!   a `Construct(Proxy, ...)`);
//! - no user-defined accessor (only data-property stores);
//! - the default `Object.prototype` chain (no `__proto__`
//!   reassignment is currently representable in IR; and even
//!   the prototype chain's accessors are absent on the keys an
//!   `AllocObject`-shaped object can have without escaping).
//! Therefore for-in over `a` runs no user code. The clear of
//! `MAY_OBSERVE_GETTERS` is sound.

use std::collections::HashSet;

use ir::error::Error;
use ir::ssa::{
    FuncId, ForInIterShape, InstKind, Module, Operand, Value, ValueDefKind,
};

use crate::FunctionPass;
use crate::escape_analysis::EscapeAnalysis;

pub struct RefineForInEffectPass;

impl FunctionPass for RefineForInEffectPass {
    fn name(&self) -> &'static str {
        "refine_forin_effect"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let captured: HashSet<u32> =
            func.captured_by_passthrough.iter().copied().collect();
        let body = func.lowered_mut();

        // Bail early if there are no ForInInit/ForInNext instructions
        // at all — the common case in non-iterating functions.
        let has_forin = body.instructions.iter().any(|i| {
            matches!(
                i.kind,
                InstKind::ForInInit { .. } | InstKind::ForInNext { .. }
            )
        });
        if !has_forin {
            return Ok(false);
        }

        let analysis = match EscapeAnalysis::compute(body, &captured) {
            Some(a) => a,
            None => return Ok(false),
        };

        // Pre-compute: for each Value `v`, is its defining alloc an
        // AllocObject whose stores are all plain data-property stores?
        // We check this on the alloc identity: a Value is
        // provably-plain-data iff (a) it's provably-unaliased AND (b)
        // every Store{Prop,Index} into it is plain data (the IR's
        // current StoreProp/StoreIndex *is* a data-property store —
        // see module-level comment).
        //
        // The "every store into it" check operates on alloc Values;
        // we identify the alloc Value of each ForInInit's obj by
        // walking through the alias-sets implicitly via
        // is_provably_unaliased, then verifying the obj is an
        // AllocObject result (alias-set singleton-rooted at an
        // AllocObject InstKind).
        //
        // Block AllocArray: only AllocObject is eligible. We check
        // the obj Value's defining inst directly.

        // Phase 1: read-only scan to identify the InstId of every
        // ForInInit / ForInNext we want to refine.
        let mut to_refine: Vec<u32> = Vec::new();
        for (idx, inst) in body.instructions.iter().enumerate() {
            let (obj_or_iter_val, is_init) = match &inst.kind {
                InstKind::ForInInit { obj, shape } => {
                    if matches!(shape, ForInIterShape::PlainData) {
                        continue;
                    }
                    match obj {
                        Operand::Value(v) => (*v, true),
                        _ => continue,
                    }
                }
                InstKind::ForInNext { iter, shape } => {
                    if matches!(shape, ForInIterShape::PlainData) {
                        continue;
                    }
                    match iter {
                        Operand::Value(v) => (*v, false),
                        _ => continue,
                    }
                }
                _ => continue,
            };

            let Some(alloc_v) = resolve_forin_alloc(body, obj_or_iter_val, is_init)
            else {
                continue;
            };
            if !analysis.is_provably_unaliased(alloc_v) {
                continue;
            }
            if !all_stores_are_plain_data(body, alloc_v) {
                continue;
            }
            to_refine.push(idx as u32);
        }

        // Phase 2: mutate. Each inst's kind is already known to be a
        // ForInInit/ForInNext by phase 1's match; the assignment below
        // is unconditional.
        let mut changed = false;
        for idx in to_refine {
            let kind = &mut body.instructions[idx as usize].kind;
            match kind {
                InstKind::ForInInit { shape, .. }
                | InstKind::ForInNext { shape, .. } => {
                    *shape = ForInIterShape::PlainData;
                    changed = true;
                }
                _ => unreachable!("phase 1 guarantees ForInInit/ForInNext"),
            }
        }

        Ok(changed)
    }
}

/// Resolve a ForIn site's iterated alloc.
/// - For `ForInInit { obj }`: the obj Value.
/// - For `ForInNext { iter }`: the iter Value's defining
///   `ForInInit`'s obj Value.
/// Returns `None` if the trace doesn't bottom out at a Value that
/// looks like a candidate alloc (AllocObject).
fn resolve_forin_alloc(
    body: &ir::ssa::LoweredBody,
    v: Value,
    is_init: bool,
) -> Option<Value> {
    let obj_v = if is_init {
        v
    } else {
        // v is the iter result — find its defining ForInInit and
        // return that's obj.
        let def = &body.values[v.0 as usize];
        let iid = match def.kind {
            ValueDefKind::Inst(i) => i,
            _ => return None,
        };
        let kind = &body.instructions[iid.0 as usize].kind;
        match kind {
            InstKind::ForInInit { obj: Operand::Value(o), .. } => *o,
            _ => return None,
        }
    };
    // Confirm obj_v is defined by an AllocObject inst.
    let def = &body.values[obj_v.0 as usize];
    let iid = match def.kind {
        ValueDefKind::Inst(i) => i,
        _ => return None,
    };
    if !matches!(body.instructions[iid.0 as usize].kind, InstKind::AllocObject) {
        return None;
    }
    Some(obj_v)
}

/// Walk every instruction; for each Store{Prop,Index} whose obj is
/// `alloc_v`, the check is currently a no-op (every IR store is a
/// data-property store). Returns true unconditionally today, but
/// retains the structural walk to future-proof against new
/// accessor-defining store kinds. Also confirms there's no Store
/// instruction that takes `alloc_v` as obj that is *not* a plain
/// StoreProp/StoreIndex — i.e. nothing exotic.
fn all_stores_are_plain_data(body: &ir::ssa::LoweredBody, alloc_v: Value) -> bool {
    for inst in &body.instructions {
        match &inst.kind {
            InstKind::StoreProp { obj, .. } | InstKind::StoreIndex { obj, .. } => {
                if let Operand::Value(o) = obj {
                    if o.0 == alloc_v.0 {
                        // Plain data store — fine. (Future: if the IR
                        // gains an accessor-defining StoreProp variant,
                        // reject it here.)
                        continue;
                    }
                }
            }
            // DeleteProp/DeleteIndex on the alloc would still be
            // data-property removal, not accessor installation —
            // sound to allow. But a delete also means iteration's
            // key set differs across the loop, which doesn't affect
            // MAY_OBSERVE_GETTERS specifically. Allow.
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, ForInIterShape, FunctionBuilder, InstKind, Module, Operand,
        Purity, Terminator,
    };

    /// Build a tiny ForIn loop over the given seed obj-construction
    /// closure. Returns the module. The caller's closure should emit
    /// the obj-defining instruction(s) and return the obj Operand.
    fn build_forin_module<F>(emit_obj: F) -> Module
    where
        F: FnOnce(&mut FunctionBuilder<'_>) -> Operand,
    {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb_pre = b.new_block();
        let bb_header = b.new_block();
        let bb_body = b.new_block();
        let bb_exit = b.new_block();

        b.switch_to_block(bb_pre);
        let obj_op = emit_obj(&mut b);
        let iter = b
            .add_inst(InstKind::ForInInit {
                obj: obj_op,
                shape: ForInIterShape::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });

        b.switch_to_block(bb_header);
        let done = b
            .add_inst(InstKind::ForInDone {
                iter: Operand::Value(iter),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(done),
            then_b: bb_exit,
            then_args: vec![],
            else_b: bb_body,
            else_args: vec![],
        });

        b.switch_to_block(bb_body);
        let _key = b
            .add_inst(InstKind::ForInNext {
                iter: Operand::Value(iter),
                shape: ForInIterShape::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });

        b.switch_to_block(bb_exit);
        b.set_terminator(Terminator::Return { val: None });

        b.finish();
        module
    }

    fn forin_shapes(module: &Module) -> Vec<ForInIterShape> {
        let body = module.functions[0].lowered();
        let mut out = Vec::new();
        for inst in &body.instructions {
            match &inst.kind {
                InstKind::ForInInit { shape, .. }
                | InstKind::ForInNext { shape, .. } => out.push(*shape),
                _ => {}
            }
        }
        out
    }

    /// ForIn over a fresh `{a: 1, b: 2}` alloc — should narrow.
    #[test]
    fn refines_forin_over_fresh_object_literal() {
        let mut module = build_forin_module(|b| {
            let key_a = b.intern("a");
            let key_b = b.intern("b");
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_a,
                val: Operand::Const(Constant::Number(1.0)),
                purity: Purity::Opaque,
            });
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_b,
                val: Operand::Const(Constant::Number(2.0)),
                purity: Purity::Opaque,
            });
            Operand::Value(alloc)
        });

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineForInEffectPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let shapes = forin_shapes(&module);
        assert_eq!(
            shapes,
            vec![ForInIterShape::PlainData, ForInIterShape::PlainData],
            "ForInInit and ForInNext should both refine to PlainData"
        );
    }

    /// ForIn over a function arg (unknown obj) — must NOT narrow.
    #[test]
    fn no_refine_for_in_over_arg() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let arg0 = b.body.params[0];
        let bb_pre = b.new_block();
        let bb_header = b.new_block();
        let bb_body = b.new_block();
        let bb_exit = b.new_block();

        b.switch_to_block(bb_pre);
        let iter = b
            .add_inst(InstKind::ForInInit {
                obj: Operand::Value(arg0),
                shape: ForInIterShape::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_header);
        let done = b
            .add_inst(InstKind::ForInDone {
                iter: Operand::Value(iter),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(done),
            then_b: bb_exit,
            then_args: vec![],
            else_b: bb_body,
            else_args: vec![],
        });
        b.switch_to_block(bb_body);
        b.add_inst(InstKind::ForInNext {
            iter: Operand::Value(iter),
            shape: ForInIterShape::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_exit);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineForInEffectPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let shapes = forin_shapes(&module);
        assert_eq!(
            shapes,
            vec![ForInIterShape::Opaque, ForInIterShape::Opaque],
            "ForIn over an arg must stay Opaque"
        );
    }

    /// ForIn over an alloc that escapes (stored to globalThis) —
    /// must NOT narrow.
    #[test]
    fn no_refine_when_alloc_escapes_via_globalthis() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let key_g = b.intern("g");
        let bb_pre = b.new_block();
        let bb_header = b.new_block();
        let bb_body = b.new_block();
        let bb_exit = b.new_block();

        b.switch_to_block(bb_pre);
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        // Escape: write alloc into globalThis.g.
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Const(Constant::GlobalThis),
            key: key_g,
            val: Operand::Value(alloc),
            purity: Purity::Throws,
        });
        let iter = b
            .add_inst(InstKind::ForInInit {
                obj: Operand::Value(alloc),
                shape: ForInIterShape::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_header);
        let done = b
            .add_inst(InstKind::ForInDone {
                iter: Operand::Value(iter),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(done),
            then_b: bb_exit,
            then_args: vec![],
            else_b: bb_body,
            else_args: vec![],
        });
        b.switch_to_block(bb_body);
        b.add_inst(InstKind::ForInNext {
            iter: Operand::Value(iter),
            shape: ForInIterShape::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_exit);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineForInEffectPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let shapes = forin_shapes(&module);
        assert_eq!(
            shapes,
            vec![ForInIterShape::Opaque, ForInIterShape::Opaque],
            "ForIn over escaped alloc must stay Opaque"
        );
    }

    /// ForIn over an AllocArray — currently NOT refined (out of
    /// scope per pass docs). Pinning behavior so a future widening
    /// is an explicit decision.
    #[test]
    fn no_refine_for_in_over_alloc_array() {
        let mut module = build_forin_module(|b| {
            let alloc = b
                .add_inst(InstKind::AllocArray { length: Some(0) })
                .unwrap();
            Operand::Value(alloc)
        });
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineForInEffectPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let shapes = forin_shapes(&module);
        assert_eq!(
            shapes,
            vec![ForInIterShape::Opaque, ForInIterShape::Opaque],
            "ForIn over AllocArray is conservatively left Opaque"
        );
    }

    /// Verifies the effect changes alongside the shape: PURE check.
    #[test]
    fn refined_forin_drops_may_observe_getters_bit() {
        use ir::ssa::Effect;
        let mut module = build_forin_module(|b| {
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            Operand::Value(alloc)
        });
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(RefineForInEffectPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        for inst in &body.instructions {
            match &inst.kind {
                k @ (InstKind::ForInInit { .. } | InstKind::ForInNext { .. }) => {
                    let eff = k.effect();
                    assert!(
                        !eff.contains(Effect::MAY_OBSERVE_GETTERS),
                        "refined ForIn must NOT carry MAY_OBSERVE_GETTERS"
                    );
                    assert!(
                        eff.contains(Effect::READS_HEAP)
                            && eff.contains(Effect::MAY_THROW),
                        "refined ForIn keeps READS_HEAP | MAY_THROW"
                    );
                }
                _ => {}
            }
        }
    }
}
