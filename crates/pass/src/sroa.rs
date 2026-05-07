//! Scalar Replacement of Aggregates (SROA / mem2reg) — issue #51.
//!
//! Rewrites unaliased `AllocObject` / `AllocArray` whose accesses all use
//! const keys into pure SSA value flow. Each (alloc, key) pair becomes a
//! direct producer/consumer chain: the value stored at key `k` becomes the
//! result of every subsequent load at key `k`, the alloc + stores + loads
//! retire, and downstream passes see a flat dataflow IR with no heap.
//!
//! ## v1 scope: single-block
//!
//! All stores and loads on the alloc must live in one basic block. The
//! alloc inst itself must also live in that block (which is the natural
//! shape — frontends emit `let o = {}; o.x = ...; o.y = ...;` adjacent
//! to its uses). A v2 ticket will lift this to cross-block via Braun-style
//! SSA reconstruction with phi insertion at joins.
//!
//! ## Soundness
//!
//! For an alloc `a` to be SROA-eligible:
//! 1. `EscapeAnalysis::is_provably_unaliased(a)` — uses the existing
//!    points-to analysis; this rules out Returns, Throws, Calls/Construct
//!    args, store-into-globalThis, and store-as-val into other allocs.
//! 2. Every use of `a` (instruction operand or terminator operand) is
//!    either:
//!    - the `obj` of `LoadProp` / `StoreProp` with an interned key
//!      (StrId is always const), OR
//!    - the `obj` of `LoadIndex` / `StoreIndex` with `index =
//!      Operand::Const(_)` (Int32 / Number-coercible-to-Int32 / String).
//!    Anything else (including ForIn, prototype ops, store-as-val, edge
//!    args) bails the alloc.
//! 3. All of `a`'s loads and stores live in a single block, alongside the
//!    alloc inst itself.
//!
//! ## Never-stored-key load semantics
//!
//! For a fresh `AllocObject`, an own-prop miss produces `undefined` (no
//! prototype chain inheritance for own props of a plain `{}`). For a
//! fresh `AllocArray`, indices that have never been written are sparse
//! holes which read as `undefined`. So loads of never-stored keys are
//! safely replaced with `Const(Undefined)`.
//!
//! ## Pipeline placement
//!
//! Registered after `EscapeAnalysisPass` (which provides the
//! `is_provably_unaliased` query) and before `ForwardLoadsPass`. After
//! SROA the affected alloc/loads/stores are gone, so ForwardLoads has
//! nothing to do for those keys — but other escaping or cross-block
//! allocs remain its responsibility, and ConstFold/CopyProp/GVN/DCE
//! benefit from running on the simplified IR.

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    Constant, FuncId, InstId, InstKind, Module, Operand, Value, ValueDef, ValueDefKind,
    walk_inst_operands, walk_inst_operands_mut, walk_term_operands,
};

use crate::FunctionPass;
use crate::escape_analysis::EscapeAnalysis;

pub struct SroaPass;

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
enum Key {
    Str(ir::ssa::StrId),
    Idx(i32),
}

/// Normalize an `index` operand to a `Key` if it is a const that maps
/// cleanly to an integer (or string) key. Mirrors `forward_loads`'s
/// normalization so a mixed `StoreProp 'x'` + `StoreIndex 0` on the same
/// alloc each become their own scalar slot.
fn key_from_index_operand(op: &Operand) -> Option<Key> {
    match op {
        Operand::Const(Constant::Int32(i)) => Some(Key::Idx(*i)),
        Operand::Const(Constant::Number(n)) => {
            if n.is_finite() && n.trunc() == *n && *n >= i32::MIN as f64 && *n <= i32::MAX as f64 {
                Some(Key::Idx(*n as i32))
            } else {
                None
            }
        }
        Operand::Const(Constant::String(s)) => Some(Key::Str(*s)),
        _ => None,
    }
}

impl FunctionPass for SroaPass {
    fn name(&self) -> &'static str {
        "sroa"
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

        // Step 1: enumerate candidate alloc Values (AllocObject / AllocArray)
        // that the escape analysis says are unaliased.
        let mut candidates: Vec<u32> = Vec::new();
        for (vi, def) in body.values.iter().enumerate() {
            if def.is_removed() || captured.contains(&(vi as u32)) {
                continue;
            }
            // Skip values with a source name — those are user-visible bindings
            // we shouldn't silently dissolve. (Also matches DCE convention.)
            if def.source_name.is_some() {
                continue;
            }
            let ValueDefKind::Inst(iid) = def.kind else {
                continue;
            };
            if !matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::AllocObject | InstKind::AllocArray { .. }
            ) {
                continue;
            }
            if !analysis.is_provably_unaliased(Value(vi as u32)) {
                continue;
            }
            candidates.push(vi as u32);
        }
        if candidates.is_empty() {
            return Ok(false);
        }

        // Step 2: per-candidate, classify all uses. Every use must be the obj
        // of a const-key load/store. We also need every load+store + the
        // alloc itself to be in a single block.
        //
        // Build inst → block index map for fast lookups.
        let mut inst_block: HashMap<u32, u32> = HashMap::new();
        for (bi, blk) in body.blocks.iter().enumerate() {
            for &iid in &blk.instructions {
                inst_block.insert(iid.0, bi as u32);
            }
        }

        let mut changed = false;
        // Processing one alloc at a time keeps the bookkeeping simple: we
        // never need to merge per-key state across allocs, and removing one
        // alloc never affects another's eligibility (each was independently
        // proven unaliased and used only as obj of const-key accesses).
        for &av in &candidates {
            let alloc_val = Value(av);
            let alloc_iid = match body.values[av as usize].kind {
                ValueDefKind::Inst(iid) => iid,
                _ => continue,
            };
            let Some(&alloc_bi) = inst_block.get(&alloc_iid.0) else {
                continue;
            };

            // Collect every load and store on this alloc, and verify it's
            // the alloc's only kind of use.
            //
            // `loads`: (iid, key, result_value). Result value is what gets
            //   replaced via replace_value_uses.
            // `stores`: (iid, key) in inst-vec order; we re-read val at
            //   rewrite time.
            let mut loads: Vec<(InstId, Key, Value)> = Vec::new();
            let mut stores: Vec<(InstId, Key)> = Vec::new();
            let mut bail = false;

            // Walk every instruction; classify uses of `alloc_val`.
            'inst_scan: for (idx, inst) in body.instructions.iter().enumerate() {
                let iid = InstId(idx as u32);
                match &inst.kind {
                    InstKind::LoadProp {
                        obj: Operand::Value(o),
                        key,
                        ..
                    } if *o == alloc_val => {
                        // obj is alloc; key (StrId) is intrinsically const.
                        let v = ir::ssa::find_value_for_inst(body, iid);
                        loads.push((iid, Key::Str(*key), v));
                    }
                    InstKind::LoadIndex {
                        obj: Operand::Value(o),
                        index,
                        ..
                    } if *o == alloc_val => {
                        let Some(k) = key_from_index_operand(index) else {
                            bail = true;
                            break 'inst_scan;
                        };
                        let v = ir::ssa::find_value_for_inst(body, iid);
                        loads.push((iid, k, v));
                    }
                    InstKind::StoreProp {
                        obj: Operand::Value(o),
                        key,
                        val,
                        ..
                    } if *o == alloc_val => {
                        // Self-store (`a.x = a`) is a fixpoint of containment;
                        // EscapeAnalysis tolerates it but SROA v1 doesn't
                        // unwind self-references. Bail.
                        if matches!(val, Operand::Value(v) if *v == alloc_val) {
                            bail = true;
                            break 'inst_scan;
                        }
                        stores.push((iid, Key::Str(*key)));
                    }
                    InstKind::StoreIndex {
                        obj: Operand::Value(o),
                        index,
                        val,
                        ..
                    } if *o == alloc_val => {
                        if matches!(val, Operand::Value(v) if *v == alloc_val) {
                            bail = true;
                            break 'inst_scan;
                        }
                        let Some(k) = key_from_index_operand(index) else {
                            bail = true;
                            break 'inst_scan;
                        };
                        stores.push((iid, k));
                    }
                    _ => {
                        // Any other operand-position use of the alloc is
                        // disqualifying. Catches the conservative tail
                        // (e.g., the alloc as the val of a store into
                        // another unaliased alloc — EscapeAnalysis tolerates
                        // via containment but SROA v1 doesn't unwind).
                        let mut sees_alloc = false;
                        walk_inst_operands(&inst.kind, |op| {
                            if matches!(op, Operand::Value(v) if *v == alloc_val) {
                                sees_alloc = true;
                            }
                        });
                        if sees_alloc {
                            bail = true;
                            break 'inst_scan;
                        }
                    }
                }
            }
            if bail {
                continue;
            }
            // Terminator operand check: an alloc reaching a terminator
            // (Return, Throw, Switch disc) escapes per EscapeAnalysis. The
            // remaining case is a branch arg flowing into a successor's
            // block-param — modeled in EscapeAnalysis via alias-set
            // propagation, not as escape. Bail conservatively if any
            // terminator references the alloc; v1 single-block can't span
            // a block-param anyway.
            let mut term_uses_alloc = false;
            for blk in &body.blocks {
                walk_term_operands(&blk.terminator, |op| {
                    if matches!(op, Operand::Value(v) if *v == alloc_val) {
                        term_uses_alloc = true;
                    }
                });
                if term_uses_alloc {
                    break;
                }
            }
            if term_uses_alloc {
                continue;
            }

            // Single-block scope: every load/store inst must live in the
            // same block as the alloc. Bail (defer to v2) otherwise.
            let same_block = loads
                .iter()
                .map(|(iid, _, _)| iid.0)
                .chain(stores.iter().map(|(iid, _)| iid.0))
                .all(|iid| inst_block.get(&iid).copied() == Some(alloc_bi));
            if !same_block {
                continue;
            }

            // Step 3: walk the block in order; maintain per-key current value.
            // On store: update map. On load: replace value uses with current
            // value, retire the load. After the walk, retire alloc + stores.
            let block_iids: Vec<u32> = body.blocks[alloc_bi as usize]
                .instructions
                .iter()
                .map(|i| i.0)
                .collect();
            let store_iids: HashSet<u32> = stores.iter().map(|(i, _)| i.0).collect();
            let load_set: HashMap<u32, Key> =
                loads.iter().map(|(i, k, _)| (i.0, *k)).collect();

            let mut current: HashMap<Key, Operand> = HashMap::new();
            let mut to_unhook: Vec<u32> = Vec::new();
            let mut alloc_changed = !stores.is_empty();

            for iid in &block_iids {
                if store_iids.contains(iid) {
                    let kind = &body.instructions[*iid as usize].kind;
                    let (k, val) = match kind {
                        InstKind::StoreProp { key, val, .. } => (Key::Str(*key), *val),
                        InstKind::StoreIndex { index, val, .. } => {
                            let Some(k) = key_from_index_operand(index) else {
                                continue;
                            };
                            (k, *val)
                        }
                        _ => continue,
                    };
                    current.insert(k, val);
                    to_unhook.push(*iid);
                    continue;
                }
                if let Some(k) = load_set.get(iid).copied() {
                    let result_v = ir::ssa::find_value_for_inst(body, InstId(*iid));
                    // Resolve to current[k] or Undefined if never stored.
                    // For a fresh AllocObject, own-prop miss is undefined
                    // (no prototype chain for own-prop reads on a plain
                    // object literal). For a fresh AllocArray, untouched
                    // indices read as undefined (sparse holes).
                    let stored = current
                        .get(&k)
                        .copied()
                        .unwrap_or(Operand::Const(Constant::Undefined));
                    if !matches!(stored, Operand::Value(sv) if sv == result_v) {
                        body.replace_value_uses(result_v, stored);
                        body.values[result_v.0 as usize] = ValueDef::removed();
                        to_unhook.push(*iid);
                        alloc_changed = true;
                    }
                }
            }

            // Retire alloc + stores + loads. Stores + loads are already in
            // `to_unhook`; add the alloc itself.
            to_unhook.push(alloc_iid.0);
            body.values[av as usize] = ValueDef::removed();

            let unhook_set: HashSet<u32> = to_unhook.iter().copied().collect();
            body.blocks[alloc_bi as usize]
                .instructions
                .retain(|i| !unhook_set.contains(&i.0));
            for iid in &to_unhook {
                walk_inst_operands_mut(
                    &mut body.instructions[*iid as usize].kind,
                    &mut |op| {
                        *op = Operand::Const(Constant::Undefined);
                    },
                );
                inst_block.remove(iid);
            }

            if alloc_changed {
                changed = true;
            }
        }

        Ok(changed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, LoweredBody, Module, Operand, Purity,
        Terminator, ValueDefKind,
    };

    fn count_kinds(body: &LoweredBody) -> (usize, usize, usize) {
        let mut allocs = 0;
        let mut stores = 0;
        let mut loads = 0;
        for blk in &body.blocks {
            for &iid in &blk.instructions {
                match &body.instructions[iid.0 as usize].kind {
                    InstKind::AllocObject | InstKind::AllocArray { .. } => allocs += 1,
                    InstKind::StoreProp { .. } | InstKind::StoreIndex { .. } => stores += 1,
                    InstKind::LoadProp { .. } | InstKind::LoadIndex { .. } => loads += 1,
                    _ => {}
                }
            }
        }
        (allocs, stores, loads)
    }

    /// `let o = {}; o.a = 1; o.b = 2; return o.a + o.b;` — alloc + 2 stores
    /// + 2 loads all retire post-SROA.
    #[test]
    fn simple_object_two_keys_sroa() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let key_b = b.intern("b");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_b,
            val: Operand::Const(Constant::Number(2.0)),
            purity: Purity::Opaque,
        });
        let la = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        let lb = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_b,
                purity: Purity::Opaque,
            })
            .unwrap();
        let sum = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(la),
                rhs: Operand::Value(lb),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(sum)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, stores, loads) = count_kinds(body);
        assert_eq!(
            (allocs, stores, loads),
            (0, 0, 0),
            "alloc + stores + loads must all retire post-SROA"
        );
        let add = body
            .instructions
            .iter()
            .find_map(|i| match &i.kind {
                InstKind::AddGeneric { lhs, rhs } => Some((*lhs, *rhs)),
                _ => None,
            })
            .unwrap();
        assert!(matches!(
            add.0,
            Operand::Const(Constant::Number(n)) if n == 1.0
        ));
        assert!(matches!(
            add.1,
            Operand::Const(Constant::Number(n)) if n == 2.0
        ));
    }

    /// Array with index-keyed stores + loads.
    #[test]
    fn simple_array_index_keys_sroa() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let arr = b
            .add_inst(InstKind::AllocArray { length: Some(2) })
            .unwrap();
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(arr),
            index: Operand::Const(Constant::Int32(0)),
            val: Operand::Const(Constant::Number(10.0)),
            purity: Purity::Opaque,
        });
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(arr),
            index: Operand::Const(Constant::Int32(1)),
            val: Operand::Const(Constant::Number(20.0)),
            purity: Purity::Opaque,
        });
        let l0 = b
            .add_inst(InstKind::LoadIndex {
                obj: Operand::Value(arr),
                index: Operand::Const(Constant::Int32(0)),
                purity: Purity::Opaque,
            })
            .unwrap();
        let _l1 = b
            .add_inst(InstKind::LoadIndex {
                obj: Operand::Value(arr),
                index: Operand::Const(Constant::Int32(1)),
                purity: Purity::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(l0)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, stores, loads) = count_kinds(body);
        assert_eq!((allocs, stores, loads), (0, 0, 0));
    }

    /// Mixed StoreProp ('foo') + StoreIndex (0) on same alloc; both keys
    /// SROA into independent scalars.
    #[test]
    fn mixed_prop_and_index_keys_sroa_independently() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_foo = b.intern("foo");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_foo,
            val: Operand::Const(Constant::Number(7.0)),
            purity: Purity::Opaque,
        });
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(o),
            index: Operand::Const(Constant::Int32(0)),
            val: Operand::Const(Constant::Number(8.0)),
            purity: Purity::Opaque,
        });
        let lf = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_foo,
                purity: Purity::Opaque,
            })
            .unwrap();
        let li = b
            .add_inst(InstKind::LoadIndex {
                obj: Operand::Value(o),
                index: Operand::Const(Constant::Int32(0)),
                purity: Purity::Opaque,
            })
            .unwrap();
        let sum = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(lf),
                rhs: Operand::Value(li),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(sum)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, stores, loads) = count_kinds(body);
        assert_eq!((allocs, stores, loads), (0, 0, 0));
    }

    /// Returned alloc → escapes; SROA must NOT touch it.
    #[test]
    fn escaping_alloc_bails() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(o)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, stores, _) = count_kinds(body);
        assert_eq!(allocs, 1, "escaping alloc must remain");
        assert_eq!(stores, 1, "store on escaping alloc must remain");
    }

    /// Non-const-key store on alloc A bails A; but alloc B in the same
    /// function (with all-const keys) still SROAs.
    #[test]
    fn non_const_key_bails_only_affected_alloc() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let arg0 = b.body.params[0];
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        // alloc A: dynamic-key store via param-derived index.
        let a = b.add_inst(InstKind::AllocArray { length: Some(1) }).unwrap();
        b.add_inst(InstKind::StoreIndex {
            obj: Operand::Value(a),
            index: Operand::Value(arg0),
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        // alloc B: clean.
        let bb_alloc = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(bb_alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(42.0)),
            purity: Purity::Opaque,
        });
        let lb = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(bb_alloc),
                key: key_x,
                purity: Purity::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(lb)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        // alloc A + its 1 store survive; alloc B + its store + load all retire.
        let (allocs, stores, loads) = count_kinds(body);
        assert_eq!(allocs, 1, "alloc A must survive (dynamic key)");
        assert_eq!(stores, 1, "alloc A's store must survive");
        assert_eq!(loads, 0, "alloc B's load must retire");
        let term = &body.blocks[body.entry.0 as usize].terminator;
        match term {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 42.0),
            other => panic!("expected Return(Number(42)), got {other:?}"),
        }
    }

    /// Load of un-stored key on a fresh AllocObject resolves to Undefined.
    #[test]
    fn never_stored_key_resolves_undefined() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_missing = b.intern("missing");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        let l = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_missing,
                purity: Purity::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(l)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, _, loads) = count_kinds(body);
        assert_eq!(allocs, 0);
        assert_eq!(loads, 0);
        let term = &body.blocks[body.entry.0 as usize].terminator;
        match term {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Undefined)),
            } => {}
            other => panic!("expected Return(Undefined), got {other:?}"),
        }
    }

    /// Cross-block: stores in entry, load in successor. v1 must bail.
    #[test]
    fn cross_block_alloc_bails_v1() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let bb0 = b.new_block();
        let bb1 = b.new_block();
        b.switch_to_block(bb0);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(5.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![],
        });
        b.switch_to_block(bb1);
        let l = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(l)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, stores, loads) = count_kinds(body);
        assert_eq!(
            (allocs, stores, loads),
            (1, 1, 1),
            "cross-block alloc must be deferred to v2"
        );
        assert!(matches!(
            body.values[o.0 as usize].kind,
            ValueDefKind::Inst(_)
        ));
    }
}
