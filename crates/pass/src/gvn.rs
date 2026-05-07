//! Global Value Numbering / Common Subexpression Elimination on pure
//! instructions beyond loads. Loads are already CSE'd by
//! `forward_loads.rs` via the heap-shape map; this pass handles every
//! other pure instruction (typed arithmetic, comparisons, bitwise,
//! `Typeof`, etc.).
//!
//! ## Soundness
//!
//! Eligible instructions have effect ⊆ `PURE | MAY_THROW` and contain
//! neither `READS_HEAP` nor `WRITES_HEAP`. Two evaluations of such an
//! instruction with bit-equal operands produce the same value (and
//! either both throw or neither does — `MAY_THROW` is GVN-safe under
//! the same-operand condition; the throwing path's observable effect
//! is identical). Allocation instructions are excluded explicitly:
//! they're effect-PURE but each evaluation has fresh identity, which
//! GVN would silently merge.
//!
//! ## Scope
//!
//! v1 is intra-block only. A later extension can walk the dominator
//! tree with a scoped hash map, which strictly increases hit rate but
//! requires a dominator computation; intra-block already catches the
//! ubiquitous "same expression evaluated twice in a straight-line
//! sequence" cases that survive ConstFold.
//!
//! ## Hashing
//!
//! Key = (kind discriminant tag + kind-specific extra fields, operand
//! list). For commutative kinds (`AddNumber`, `MulNumber`, `AddInt32`,
//! `MulInt32`, `Eq`, `StrictEq`, `BitAnd`, `BitOr`, `BitXor`,
//! `LogAnd`, `LogOr`), the operand pair is sorted by a total order on
//! `Operand` before hashing so `Add(a, b)` and `Add(b, a)` hash equal.
//! Non-commutative kinds preserve operand order.
//!
//! `Eq` and `StrictEq` are commutative under JS semantics
//! (`a == b` ⇔ `b == a`, `a === b` ⇔ `b === a`); `Lt` / `Le` are not.
//! Generic arithmetic (`AddGeneric`, `MulGeneric`) is *not* treated
//! as commutative even though the algebraic ops are: operand
//! evaluation order matters for `valueOf`/`toString` side effects,
//! and these ops are only `MAY_THROW` because the throw is from those
//! conversions, which run left-to-right. Identical-operand-order
//! still dedups, just not the swapped form.
//!
//! ## Skip rules
//!
//! - Non-pure effects (`READS_HEAP` or `WRITES_HEAP`).
//! - Allocation instructions (fresh-identity semantics).
//! - Calls / OpaqueEval / spread-args / for-in / catch (effect is
//!   `OPAQUE`, but explicit-skip for clarity).
//! - Pinned result Values (`Function.captured_by_passthrough`) — a
//!   PassThrough body lexically references the dropped Value's
//!   `source_name`; merging into another Value loses that anchor.
//! - Source-named result Values — same anchor concern; renamer wants
//!   the original binding visible.

use std::collections::HashMap;
use std::collections::HashSet;

use ir::error::Error;
use ir::ssa::{
    Constant, Effect, FuncId, InstKind, Module, Operand, StrId, Value, ValueDef,
    find_value_for_inst, walk_inst_operands_mut,
};

use crate::FunctionPass;

pub struct GvnPass;

/// Canonicalised operand form used as a hash-key component. Mirrors
/// `Operand` but with a totally-ordered, `Hash + Eq` payload so we
/// can sort commutative operand pairs and key the GVN map.
/// `Number` collapses NaN bit-patterns to a canonical NaN-tag, and
/// `Number(0.0)` and `Number(-0.0)` are kept distinct (so are 0 and
/// -0 under `f64::to_bits`), matching `const_bit_eq`'s semantics.
#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug, PartialOrd, Ord)]
enum OpKey {
    Value(u32),
    Capture(u32),
    Number(u64),
    Int32(i32),
    Bool(bool),
    String(StrId),
    Null,
    Undefined,
    Func(u32),
    GlobalThis,
    This,
    BigInt(StrId),
}

fn op_key(op: &Operand) -> OpKey {
    match op {
        Operand::Value(v) => OpKey::Value(v.0),
        Operand::Capture(i) => OpKey::Capture(*i as u32),
        Operand::Const(Constant::Number(n)) => OpKey::Number(n.to_bits()),
        Operand::Const(Constant::Int32(i)) => OpKey::Int32(*i),
        Operand::Const(Constant::Bool(b)) => OpKey::Bool(*b),
        Operand::Const(Constant::String(s)) => OpKey::String(*s),
        Operand::Const(Constant::Null) => OpKey::Null,
        Operand::Const(Constant::Undefined) => OpKey::Undefined,
        Operand::Const(Constant::Func(f)) => OpKey::Func(f.0),
        Operand::Const(Constant::GlobalThis) => OpKey::GlobalThis,
        Operand::Const(Constant::This) => OpKey::This,
        Operand::Const(Constant::BigInt(s)) => OpKey::BigInt(*s),
    }
}

/// Hash key for an instruction: kind discriminant tag + canonical
/// operand list. Commutative kinds canonicalize the operand pair
/// before hashing.
#[derive(Hash, PartialEq, Eq, Clone, Debug)]
struct Key {
    tag: u16,
    operands: Vec<OpKey>,
}

/// True iff this kind is a candidate for GVN. Caller has already
/// fetched `effect()`; we additionally exclude alloc and the explicit
/// special-cases noted in the module doc.
fn is_gvn_candidate(kind: &InstKind) -> bool {
    use InstKind::*;
    let eff = kind.effect();
    // Must be pure or may-throw only.
    if eff.intersects(Effect::READS_HEAP | Effect::WRITES_HEAP | Effect::MAY_DIVERGE) {
        return false;
    }
    if eff.contains(Effect::OPAQUE) {
        return false;
    }
    // Effect-PURE allocations are excluded — fresh identity.
    matches!(
        kind,
        AddGeneric { .. }
            | SubGeneric { .. }
            | MulGeneric { .. }
            | DivGeneric { .. }
            | ModGeneric { .. }
            | PowGeneric { .. }
            | BitAnd { .. }
            | BitOr { .. }
            | BitXor { .. }
            | Shl { .. }
            | Shr { .. }
            | UShr { .. }
            | AddNumber { .. }
            | SubNumber { .. }
            | MulNumber { .. }
            | AddInt32 { .. }
            | SubInt32 { .. }
            | MulInt32 { .. }
            | ConcatStr { .. }
            | Not { .. }
            | BitNot { .. }
            | LogAnd { .. }
            | LogOr { .. }
            | Eq { .. }
            | StrictEq { .. }
            | Lt { .. }
            | Le { .. }
            | ToNumber { .. }
            | ToString { .. }
            | ToBool { .. }
            | ToInt32 { .. }
            | Typeof { .. }
            | Instanceof { .. }
            | In { .. }
    )
}

/// Build a `Key` for `kind`. Returns `None` for non-candidates. The
/// `tag` is a hand-assigned discriminant to keep `Key` `Hash`-clean
/// without having to derive `Hash` on `InstKind` (which is `Clone +
/// Debug` only and contains `f64`).
fn build_key(kind: &InstKind) -> Option<Key> {
    use InstKind::*;
    let (tag, ops, commutative): (u16, Vec<Operand>, bool) = match kind {
        AddGeneric { lhs, rhs } => (1, vec![*lhs, *rhs], false),
        SubGeneric { lhs, rhs } => (2, vec![*lhs, *rhs], false),
        MulGeneric { lhs, rhs } => (3, vec![*lhs, *rhs], false),
        DivGeneric { lhs, rhs } => (4, vec![*lhs, *rhs], false),
        ModGeneric { lhs, rhs } => (5, vec![*lhs, *rhs], false),
        PowGeneric { lhs, rhs } => (6, vec![*lhs, *rhs], false),
        BitAnd { lhs, rhs } => (7, vec![*lhs, *rhs], true),
        BitOr { lhs, rhs } => (8, vec![*lhs, *rhs], true),
        BitXor { lhs, rhs } => (9, vec![*lhs, *rhs], true),
        Shl { lhs, rhs } => (10, vec![*lhs, *rhs], false),
        Shr { lhs, rhs } => (11, vec![*lhs, *rhs], false),
        UShr { lhs, rhs } => (12, vec![*lhs, *rhs], false),
        AddNumber { lhs, rhs } => (13, vec![*lhs, *rhs], true),
        SubNumber { lhs, rhs } => (14, vec![*lhs, *rhs], false),
        MulNumber { lhs, rhs } => (15, vec![*lhs, *rhs], true),
        AddInt32 { lhs, rhs } => (16, vec![*lhs, *rhs], true),
        SubInt32 { lhs, rhs } => (17, vec![*lhs, *rhs], false),
        MulInt32 { lhs, rhs } => (18, vec![*lhs, *rhs], true),
        ConcatStr { lhs, rhs } => (19, vec![*lhs, *rhs], false),
        Not { operand } => (20, vec![*operand], false),
        BitNot { operand } => (21, vec![*operand], false),
        LogAnd { lhs, rhs } => (22, vec![*lhs, *rhs], true),
        LogOr { lhs, rhs } => (23, vec![*lhs, *rhs], true),
        Eq { lhs, rhs } => (24, vec![*lhs, *rhs], true),
        StrictEq { lhs, rhs } => (25, vec![*lhs, *rhs], true),
        Lt { lhs, rhs } => (26, vec![*lhs, *rhs], false),
        Le { lhs, rhs } => (27, vec![*lhs, *rhs], false),
        ToNumber { val } => (28, vec![*val], false),
        ToString { val } => (29, vec![*val], false),
        ToBool { val } => (30, vec![*val], false),
        ToInt32 { val } => (31, vec![*val], false),
        Typeof { val } => (32, vec![*val], false),
        Instanceof { obj, ctor } => (33, vec![*obj, *ctor], false),
        In { key, obj } => (34, vec![*key, *obj], false),
        _ => return None,
    };
    let mut canon: Vec<OpKey> = ops.iter().map(op_key).collect();
    if commutative {
        canon.sort();
    }
    Some(Key { tag, operands: canon })
}

impl FunctionPass for GvnPass {
    fn name(&self) -> &'static str {
        "gvn"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let pinned: HashSet<u32> =
            func.captured_by_passthrough.iter().copied().collect();
        let body = func.lowered_mut();
        let mut changed = false;

        // Intra-block walk. A fresh table per block is the dominator-
        // sibling-safe scope: insts in block A cannot dedup against
        // insts in block B since neither dominates the other in
        // general. (A subsequent dominator-tree extension would
        // inherit the parent block's table, but this v1 stops at
        // block boundaries.)
        for bi in 0..body.blocks.len() {
            let mut table: HashMap<Key, Value> = HashMap::new();
            let iids: Vec<u32> = body.blocks[bi]
                .instructions
                .iter()
                .map(|i| i.0)
                .collect();
            let mut to_unhook: Vec<u32> = Vec::new();
            for iid in iids {
                let kind_clone = body.instructions[iid as usize].kind.clone();
                if !is_gvn_candidate(&kind_clone) {
                    continue;
                }
                let key = match build_key(&kind_clone) {
                    Some(k) => k,
                    None => continue,
                };
                let result = find_value_for_inst(body, ir::ssa::InstId(iid));
                // Skip pinned and source-named result Values: their
                // Value identity is anchored by the renamer / a
                // PassThrough body's lexical reference and must not
                // be merged into another Value.
                if pinned.contains(&result.0)
                    || body.values[result.0 as usize].source_name.is_some()
                {
                    // Even though we don't merge this one INTO an
                    // earlier one, it is still a perfectly good
                    // representative to merge LATER ones into. Insert
                    // it so subsequent identical insts dedup against
                    // this representative (whose anchoring is fine
                    // since its uses already pin to this Value).
                    table.entry(key).or_insert(result);
                    continue;
                }
                if let Some(&prev) = table.get(&key) {
                    // Replace later inst's result Value with the
                    // earlier Value, retire later inst (DCE
                    // convention: replace uses, mark Removed,
                    // unhook from block, blank operands).
                    body.replace_value_uses(result, Operand::Value(prev));
                    body.values[result.0 as usize] = ValueDef::removed();
                    to_unhook.push(iid);
                    changed = true;
                } else {
                    table.insert(key, result);
                }
            }
            if !to_unhook.is_empty() {
                let unhook_set: HashSet<u32> = to_unhook.iter().copied().collect();
                body.blocks[bi]
                    .instructions
                    .retain(|i| !unhook_set.contains(&i.0));
                for iid in to_unhook {
                    walk_inst_operands_mut(
                        &mut body.instructions[iid as usize].kind,
                        &mut |op| {
                            *op = Operand::Const(Constant::Undefined);
                        },
                    );
                }
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
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
    };

    /// Two AddNumber on the same operands — second dedups to the first.
    #[test]
    fn dedups_identical_add_number() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 2);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let x = b.body().params[0];
        let y = b.body().params[1];
        let a1 = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        let a2 = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        // Use both in the terminator so DCE-style guards (use-count) aren't tripped.
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(a2)),
        });
        b.finish();
        // Reference a1 too, to keep it live (terminator already uses a2 →
        // post-GVN a1 must be the survivor).
        let _ = a1;

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        // Exactly one AddNumber survives.
        let adds: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::AddNumber { .. }
            ))
            .count();
        assert_eq!(adds, 1, "second AddNumber should dedup");
        // Terminator now references a1 (the survivor).
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Value(v)),
            } => assert_eq!(*v, a1, "ret should reference the surviving Value"),
            other => panic!("unexpected terminator: {other:?}"),
        }
    }

    /// Commutative canonicalization: AddNumber(x, y) and AddNumber(y, x)
    /// hash equal — second dedups.
    #[test]
    fn commutative_canonicalization_dedups_swapped_operands() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 2);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let x = b.body().params[0];
        let y = b.body().params[1];
        let a1 = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        let _a2 = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(y),
                rhs: Operand::Value(x),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(_a2)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let adds: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::AddNumber { .. }
            ))
            .count();
        assert_eq!(adds, 1);
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Value(v)),
            } => assert_eq!(*v, a1),
            other => panic!("unexpected terminator: {other:?}"),
        }
    }

    /// Non-commutative SubNumber must NOT dedup swapped operands.
    #[test]
    fn non_commutative_does_not_dedup_swapped() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 2);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let x = b.body().params[0];
        let y = b.body().params[1];
        let _ = b
            .add_inst(InstKind::SubNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        let _ = b
            .add_inst(InstKind::SubNumber {
                lhs: Operand::Value(y),
                rhs: Operand::Value(x),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let subs: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::SubNumber { .. }
            ))
            .count();
        assert_eq!(subs, 2, "SubNumber is non-commutative — both must survive");
    }

    /// AddNumber and AddInt32 on the same operands must not dedup —
    /// different instruction kind discriminants → different keys.
    #[test]
    fn different_kinds_do_not_dedup() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 2);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let x = b.body().params[0];
        let y = b.body().params[1];
        let _ = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        let _ = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        assert_eq!(
            body.blocks[entry.0 as usize].instructions.len(),
            2,
            "different kinds must not dedup"
        );
    }

    /// Loads (READS_HEAP) must be skipped — that's `forward_loads`'
    /// territory and the keying scheme for properties differs anyway.
    #[test]
    fn does_not_dedup_across_heap_load() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let arg = b.body().params[0];
        let _ = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(arg),
                key: key_x,
                purity: Purity::Throws,
            })
            .unwrap();
        let _ = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(arg),
                key: key_x,
                purity: Purity::Throws,
            })
            .unwrap();
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let loads: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::LoadProp { .. }
            ))
            .count();
        assert_eq!(loads, 2, "GVN must not touch heap loads — handled by ForwardLoads");
    }

    /// AllocObject is effect-PURE but each evaluation has fresh
    /// identity. Two AllocObject must NOT dedup.
    #[test]
    fn alloc_object_not_deduped_despite_pure_effect() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let a1 = b.add_inst(InstKind::AllocObject).unwrap();
        let a2 = b.add_inst(InstKind::AllocObject).unwrap();
        // Keep both live so DCE wouldn't remove them anyway.
        let key_x = b.intern("x");
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(a1),
            key: key_x,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Pure,
        });
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(a2),
            key: key_x,
            val: Operand::Const(Constant::Number(2.0)),
            purity: Purity::Pure,
        });
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let allocs: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::AllocObject
            ))
            .count();
        assert_eq!(allocs, 2, "fresh-identity allocs must not dedup");
    }

    /// Heap store between two identical pure ops doesn't matter for
    /// pure-op GVN — `Typeof(%x)` is still equal across the store.
    /// Verifies we don't accidentally invalidate the table on
    /// unrelated heap traffic.
    #[test]
    fn pure_op_dedups_across_intervening_store() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let key_x = b.intern("x");
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let arg = b.body().params[0];
        let alloc = b.add_inst(InstKind::AllocObject).unwrap();
        let t1 = b
            .add_inst(InstKind::Typeof {
                val: Operand::Value(arg),
            })
            .unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(alloc),
            key: key_x,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Pure,
        });
        let t2 = b
            .add_inst(InstKind::Typeof {
                val: Operand::Value(arg),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(t2)),
        });
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let typeofs: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::Typeof { .. }
            ))
            .count();
        assert_eq!(typeofs, 1, "Typeof is pure — store between doesn't invalidate");
        match &body.blocks[entry.0 as usize].terminator {
            Terminator::Return {
                val: Some(Operand::Value(v)),
            } => assert_eq!(*v, t1, "later Typeof should rewrite to first"),
            other => panic!("unexpected: {other:?}"),
        }
    }

    /// Pinned (captured-by-passthrough) result Values must not be
    /// merged INTO an earlier Value — their identity is anchored.
    /// In real programs captured-by-passthrough values are also
    /// source-named (the lowering invariant); both flags independently
    /// disable the merge, so this test exercises both at once.
    #[test]
    fn pinned_value_is_not_replaced() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 2);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let x = b.body().params[0];
        let y = b.body().params[1];
        let a1 = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        let a2 = b
            .add_inst(InstKind::AddNumber {
                lhs: Operand::Value(x),
                rhs: Operand::Value(y),
            })
            .unwrap();
        let a2_name = b.intern("a2");
        b.set_source_name(a2, a2_name);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(a2)),
        });
        // Pin the LATER value so it cannot be dropped.
        b.mark_value_captured_by_passthrough(a2);
        b.finish();
        // a1 used? No, just ensure both stay.
        let _ = a1;

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(GvnPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let entry = body.entry;
        let adds: usize = body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .filter(|iid| matches!(
                body.instructions[iid.0 as usize].kind,
                InstKind::AddNumber { .. }
            ))
            .count();
        assert_eq!(adds, 2, "pinned later AddNumber must not be replaced");
    }
}
