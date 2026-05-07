//! Scalar Replacement of Aggregates (SROA / mem2reg) — issues #51, #52.
//!
//! Rewrites unaliased `AllocObject` / `AllocArray` whose accesses all use
//! const keys into pure SSA value flow. Each (alloc, key) pair becomes a
//! direct producer/consumer chain: the value stored at key `k` becomes the
//! result of every subsequent load at key `k`, the alloc + stores + loads
//! retire, and downstream passes see a flat dataflow IR with no heap.
//!
//! ## Scope
//!
//! - **v1 single-block fast path** (issue #51): when the alloc and all
//!   its accesses live in one block, walk the block linearly maintaining
//!   `current[key]`; replace loads, retire stores + alloc.
//! - **v2 cross-block path** (issue #52): when accesses span multiple
//!   blocks, do Braun-style on-the-fly SSA reconstruction at the slot
//!   level. Per (alloc, key) slot, walk blocks in RPO over the sub-CFG
//!   dominated by the alloc; at each block with multiple in-region
//!   predecessors, insert a block-param ("phi") for every slot the
//!   alloc tracks, with edge args from each predecessor's `current[key]`.
//!   Back-edge predecessors (loop latches) get patched after their
//!   out-state is known. Trivial block-params (single distinct
//!   incoming) are removed via `try_remove_trivial_block_param`.
//!
//! ## Soundness
//!
//! For an alloc `a` to be SROA-eligible (both paths):
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
//!
//! Additional cross-block conditions (v2):
//! 3. The alloc-defining block dominates every block containing a
//!    load or store on the alloc.
//! 4. No exception-edge complications: if the alloc-defining block, or
//!    any of its use blocks, has a non-`None` `exception_target` (i.e.,
//!    is lexically inside a `try`-body), bail. The catch handler can
//!    observe slot state at any may-throw site inside the try-body;
//!    modeling that requires exception-edge phi-args, deferred to a
//!    later ticket.
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
    BlockId, Constant, FuncId, InstId, InstKind, LoweredBody, Module, Operand, Terminator,
    Value, ValueDef, ValueDefKind, walk_inst_operands, walk_inst_operands_mut,
    walk_term_operands,
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

            // Decide path: single-block fast path vs cross-block.
            let same_block = loads
                .iter()
                .map(|(iid, _, _)| iid.0)
                .chain(stores.iter().map(|(iid, _)| iid.0))
                .all(|iid| inst_block.get(&iid).copied() == Some(alloc_bi));

            let alloc_changed = if same_block {
                rewrite_single_block(
                    body,
                    BlockId(alloc_bi),
                    alloc_iid,
                    av,
                    &loads,
                    &stores,
                    &mut inst_block,
                )
            } else {
                rewrite_cross_block(
                    body,
                    BlockId(alloc_bi),
                    alloc_iid,
                    av,
                    &loads,
                    &stores,
                    &mut inst_block,
                )
            };

            if alloc_changed {
                changed = true;
            }
        }

        Ok(changed)
    }
}

// ============================================================================
// Single-block path (v1, issue #51)
// ============================================================================

fn rewrite_single_block(
    body: &mut LoweredBody,
    alloc_bid: BlockId,
    alloc_iid: InstId,
    alloc_vi: u32,
    loads: &[(InstId, Key, Value)],
    stores: &[(InstId, Key)],
    inst_block: &mut HashMap<u32, u32>,
) -> bool {
    let block_iids: Vec<u32> = body.blocks[alloc_bid.0 as usize]
        .instructions
        .iter()
        .map(|i| i.0)
        .collect();
    let store_iids: HashSet<u32> = stores.iter().map(|(i, _)| i.0).collect();
    let load_set: HashMap<u32, Key> = loads.iter().map(|(i, k, _)| (i.0, *k)).collect();

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

    to_unhook.push(alloc_iid.0);
    body.values[alloc_vi as usize] = ValueDef::removed();

    let unhook_set: HashSet<u32> = to_unhook.iter().copied().collect();
    body.blocks[alloc_bid.0 as usize]
        .instructions
        .retain(|i| !unhook_set.contains(&i.0));
    for iid in &to_unhook {
        walk_inst_operands_mut(&mut body.instructions[*iid as usize].kind, &mut |op| {
            *op = Operand::Const(Constant::Undefined);
        });
        inst_block.remove(iid);
    }

    alloc_changed
}

// ============================================================================
// Cross-block path (v2, issue #52)
// ============================================================================
//
// Braun-style on-the-fly SSA reconstruction specialised to (alloc, key)
// slots. Walk the sub-CFG dominated by the alloc-defining block in RPO;
// per slot, maintain `block_out[bid][key]`. At each multi-pred join,
// insert a block-param per tracked key with edge args from each
// predecessor's slot value (defaulting to `Undefined` when the
// predecessor never stored the key — matches v1's never-stored
// semantics: own-prop miss on a fresh `{}` is `undefined`; sparse
// array holes read as `undefined`).
//
// Loop back-edges: a back-edge predecessor's `block_out` is computed
// AFTER we visit the loop header in RPO. We provisionally insert the
// block-params at the header with edge args from forward preds only,
// then patch the back-edge's edge arg in a second pass once the
// back-pred has been processed. Trivial-phi cleanup folds the params
// that turn out to have a single distinct incoming.

fn rewrite_cross_block(
    body: &mut LoweredBody,
    alloc_bid: BlockId,
    alloc_iid: InstId,
    alloc_vi: u32,
    loads: &[(InstId, Key, Value)],
    stores: &[(InstId, Key)],
    inst_block: &mut HashMap<u32, u32>,
) -> bool {
    let n = body.blocks.len();

    // --- Soundness gate: exception-edge guard ---
    if body.blocks[alloc_bid.0 as usize].exception_target.is_some() {
        return false;
    }
    let mut use_blocks: HashSet<u32> = HashSet::new();
    use_blocks.insert(alloc_bid.0);
    for (iid, _, _) in loads {
        if let Some(&bi) = inst_block.get(&iid.0) {
            use_blocks.insert(bi);
        }
    }
    for (iid, _) in stores {
        if let Some(&bi) = inst_block.get(&iid.0) {
            use_blocks.insert(bi);
        }
    }
    for &bi in &use_blocks {
        if (bi as usize) < n
            && body.blocks[bi as usize].exception_target.is_some()
        {
            return false;
        }
    }

    // --- Dominance gate ---
    let dom = compute_dominators_simple(body);
    for &bi in &use_blocks {
        if (bi as usize) >= n {
            return false;
        }
        if !dom_contains(&dom, BlockId(bi), alloc_bid) {
            return false;
        }
    }

    // --- Tracked keys ---
    let mut keys: Vec<Key> = Vec::new();
    let mut keyset: HashSet<Key> = HashSet::new();
    for (_, k, _) in loads {
        if keyset.insert(*k) {
            keys.push(*k);
        }
    }
    for (_, k) in stores {
        if keyset.insert(*k) {
            keys.push(*k);
        }
    }

    // --- Region: blocks reachable from alloc_bid AND dominated by it. ---
    let mut in_region: Vec<bool> = vec![false; n];
    {
        let mut stack = vec![alloc_bid.0 as usize];
        in_region[alloc_bid.0 as usize] = true;
        while let Some(bi) = stack.pop() {
            for (s, _) in body.blocks[bi].terminator.successors() {
                let si = s.0 as usize;
                if si < n && !in_region[si] && dom_contains(&dom, s, alloc_bid) {
                    in_region[si] = true;
                    stack.push(si);
                }
            }
        }
    }

    // --- Region RPO ---
    let rpo = compute_region_rpo(body, alloc_bid, &in_region);

    let mut rpo_pos: HashMap<u32, usize> = HashMap::new();
    for (i, &bi) in rpo.iter().enumerate() {
        rpo_pos.insert(bi as u32, i);
    }

    let region_preds: Vec<Vec<BlockId>> = (0..n)
        .map(|bi| {
            if !in_region[bi] {
                return Vec::new();
            }
            body.blocks[bi]
                .predecessors
                .iter()
                .copied()
                .filter(|p| in_region[p.0 as usize])
                .collect()
        })
        .collect();

    // --- Per-block per-key out-state, tracked phi insertions, pending
    // back-edge args. ---
    let mut block_out: HashMap<u32, HashMap<Key, Operand>> = HashMap::new();
    let mut phi_param: HashMap<(u32, Key), Value> = HashMap::new();
    let mut phi_pending_backpreds: HashMap<u32, Vec<BlockId>> = HashMap::new();

    let store_iids: HashSet<u32> = stores.iter().map(|(i, _)| i.0).collect();
    let load_set: HashMap<u32, Key> = loads.iter().map(|(i, k, _)| (i.0, *k)).collect();
    let mut to_unhook: Vec<u32> = Vec::new();

    // --- RPO walk. ---
    for &bi_u in &rpo {
        let bid = BlockId(bi_u as u32);

        // Compute IN state.
        let mut current: HashMap<Key, Operand> = HashMap::new();
        if bid == alloc_bid {
            // Region root: slots are Undefined until written.
        } else {
            let preds = &region_preds[bi_u];
            if preds.len() == 1 {
                if let Some(out) = block_out.get(&preds[0].0) {
                    current = out.clone();
                }
            } else if preds.len() >= 2 {
                let pos_self = rpo_pos[&bid.0];
                let mut backpreds: Vec<BlockId> = Vec::new();
                for k in &keys {
                    let param = add_block_param(body, bid);
                    phi_param.insert((bid.0, *k), param);
                    current.insert(*k, Operand::Value(param));
                }
                for &p in preds {
                    let pos_p = rpo_pos.get(&p.0).copied();
                    let is_back = match pos_p {
                        Some(pp) => pp >= pos_self,
                        None => true,
                    };
                    if is_back {
                        backpreds.push(p);
                    } else {
                        let out = block_out.get(&p.0).cloned().unwrap_or_default();
                        for k in &keys {
                            let arg = out
                                .get(k)
                                .copied()
                                .unwrap_or(Operand::Const(Constant::Undefined));
                            push_edge_arg(
                                &mut body.blocks[p.0 as usize].terminator,
                                bid,
                                arg,
                            );
                        }
                    }
                }
                if !backpreds.is_empty() {
                    phi_pending_backpreds.insert(bid.0, backpreds);
                }
            }
        }

        // Walk instructions: stores update `current`, loads get
        // rewritten, both retired.
        let block_iids: Vec<u32> = body.blocks[bi_u]
            .instructions
            .iter()
            .map(|i| i.0)
            .collect();
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
                let stored = current
                    .get(&k)
                    .copied()
                    .unwrap_or(Operand::Const(Constant::Undefined));
                if !matches!(stored, Operand::Value(sv) if sv == result_v) {
                    body.replace_value_uses(result_v, stored);
                    body.values[result_v.0 as usize] = ValueDef::removed();
                    to_unhook.push(*iid);
                }
            }
        }

        block_out.insert(bid.0, current);
    }

    // --- Patch back-edges. ---
    let pending: Vec<(u32, Vec<BlockId>)> = phi_pending_backpreds.into_iter().collect();
    for (bid_u, backpreds) in pending {
        let bid = BlockId(bid_u);
        for p in backpreds {
            let out = block_out.get(&p.0).cloned().unwrap_or_default();
            for k in &keys {
                let arg = out
                    .get(k)
                    .copied()
                    .unwrap_or(Operand::Const(Constant::Undefined));
                push_edge_arg(&mut body.blocks[p.0 as usize].terminator, bid, arg);
            }
        }
    }

    // --- Trivial-phi cleanup. ---
    let mut params_to_check: Vec<Value> = phi_param.values().copied().collect();
    let mut stable = false;
    while !stable {
        stable = true;
        let mut next: Vec<Value> = Vec::new();
        for v in params_to_check.drain(..) {
            if body.values[v.0 as usize].is_removed() {
                continue;
            }
            if body.try_remove_trivial_block_param(v).is_some() {
                stable = false;
            } else {
                next.push(v);
            }
        }
        params_to_check = next;
    }

    // --- Retire alloc + stores + loads. ---
    to_unhook.push(alloc_iid.0);
    body.values[alloc_vi as usize] = ValueDef::removed();
    let alloc_changed = true;

    let mut by_block: HashMap<u32, HashSet<u32>> = HashMap::new();
    for iid in &to_unhook {
        if let Some(&bi) = inst_block.get(iid) {
            by_block.entry(bi).or_default().insert(*iid);
        }
    }
    for (bi, set) in &by_block {
        body.blocks[*bi as usize]
            .instructions
            .retain(|i| !set.contains(&i.0));
    }
    for iid in &to_unhook {
        walk_inst_operands_mut(&mut body.instructions[*iid as usize].kind, &mut |op| {
            *op = Operand::Const(Constant::Undefined);
        });
        inst_block.remove(iid);
    }

    alloc_changed
}

// ============================================================================
// Helpers: dominators, RPO over a region, edge-arg push
// ============================================================================

/// Iterative dominator computation rooted at `body.entry`. Mirrors
/// `licm::compute_dominators`'s algorithm; duplicated here to keep
/// SROA self-contained.
fn compute_dominators_simple(body: &LoweredBody) -> Vec<Option<HashSet<u32>>> {
    let n = body.blocks.len();
    let entry = body.entry.0 as usize;
    let mut dom: Vec<Option<HashSet<u32>>> = vec![None; n];
    if entry >= n {
        return dom;
    }
    let mut reachable = vec![false; n];
    let mut stack = vec![entry];
    reachable[entry] = true;
    while let Some(bi) = stack.pop() {
        for (s, _) in body.blocks[bi].terminator.successors() {
            let si = s.0 as usize;
            if si < n && !reachable[si] {
                reachable[si] = true;
                stack.push(si);
            }
        }
    }
    let full: HashSet<u32> = (0..n).filter(|i| reachable[*i]).map(|i| i as u32).collect();
    for i in 0..n {
        if !reachable[i] {
            continue;
        }
        if i == entry {
            let mut s = HashSet::new();
            s.insert(entry as u32);
            dom[i] = Some(s);
        } else {
            dom[i] = Some(full.clone());
        }
    }
    let mut changed = true;
    let mut iters = 0usize;
    let max_iters = (n.saturating_mul(n)).saturating_add(64);
    while changed {
        iters += 1;
        if iters > max_iters {
            break;
        }
        changed = false;
        for bi in 0..n {
            if !reachable[bi] || bi == entry {
                continue;
            }
            let preds = &body.blocks[bi].predecessors;
            let mut new_set: Option<HashSet<u32>> = None;
            for &p in preds {
                let pi = p.0 as usize;
                if pi >= n || !reachable[pi] {
                    continue;
                }
                let pset = match &dom[pi] {
                    Some(s) => s,
                    None => continue,
                };
                new_set = Some(match new_set {
                    None => pset.clone(),
                    Some(acc) => acc.intersection(pset).copied().collect(),
                });
            }
            let mut new_set = new_set.unwrap_or_default();
            new_set.insert(bi as u32);
            if dom[bi].as_ref() != Some(&new_set) {
                dom[bi] = Some(new_set);
                changed = true;
            }
        }
    }
    dom
}

fn dom_contains(dom: &[Option<HashSet<u32>>], b: BlockId, a: BlockId) -> bool {
    match dom.get(b.0 as usize).and_then(|s| s.as_ref()) {
        Some(s) => s.contains(&a.0),
        None => false,
    }
}

/// RPO of the region: blocks marked `in_region`, rooted at `root`.
/// Tolerant of cycles — back-edges show up as forward-references in
/// RPO position, which the cross-block path handles via deferred
/// edge-arg patching.
fn compute_region_rpo(
    body: &LoweredBody,
    root: BlockId,
    in_region: &[bool],
) -> Vec<usize> {
    let n = body.blocks.len();
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: Vec<Color> = vec![Color::White; n];
    let mut postorder: Vec<usize> = Vec::new();
    let mut stack: Vec<(usize, usize)> = Vec::new();
    let root_i = root.0 as usize;
    if root_i >= n || !in_region[root_i] {
        return postorder;
    }
    color[root_i] = Color::Gray;
    stack.push((root_i, 0));
    while let Some(&(node, idx)) = stack.last() {
        let succs: Vec<usize> = body.blocks[node]
            .terminator
            .successors()
            .iter()
            .map(|(t, _)| t.0 as usize)
            .filter(|&i| i < n && in_region[i])
            .collect();
        if idx < succs.len() {
            let s = succs[idx];
            stack.last_mut().unwrap().1 = idx + 1;
            match color[s] {
                Color::White => {
                    color[s] = Color::Gray;
                    stack.push((s, 0));
                }
                Color::Gray | Color::Black => {
                    // Back-edge (Gray) or cross/forward-edge (Black) —
                    // both fine; we don't bail on cycles here.
                }
            }
        } else {
            color[node] = Color::Black;
            postorder.push(node);
            stack.pop();
        }
    }
    postorder.reverse();
    postorder
}

/// Append a fresh block-param to `block` and return its `Value`.
/// Equivalent to `FunctionBuilder::add_block_param` but operates
/// directly on a `LoweredBody` (which is what passes hold).
fn add_block_param(body: &mut LoweredBody, block: BlockId) -> Value {
    let index = body.blocks[block.0 as usize].params.len() as u32;
    let v = Value(body.values.len() as u32);
    body.values.push(ValueDef::block_param(block, index));
    body.blocks[block.0 as usize].params.push(v);
    v
}

/// Append `op` to `src`'s edge args targeting `dst`. Mirrors the
/// Converter's `push_edge_arg` helper in `parse::lower_ssa` — kept
/// local to avoid a cross-crate dependency.
fn push_edge_arg(t: &mut Terminator, dst: BlockId, op: Operand) {
    match t {
        Terminator::Branch { target, args } if *target == dst => args.push(op),
        Terminator::CondBranch {
            then_b,
            then_args,
            else_b,
            else_args,
            ..
        } => {
            if *then_b == dst {
                then_args.push(op);
            }
            if *else_b == dst {
                else_args.push(op);
            }
        }
        Terminator::Switch {
            cases,
            default,
            default_args,
            ..
        } => {
            for c in cases.iter_mut() {
                if c.target == dst {
                    c.args.push(op);
                }
            }
            if *default == dst {
                default_args.push(op);
            }
        }
        Terminator::Try {
            try_entry,
            try_args,
            ..
        } if *try_entry == dst => {
            try_args.push(op);
        }
        _ => panic!("push_edge_arg: terminator does not target bb{}", dst.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, LoweredBody, Module, Operand, Purity,
        Terminator,
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

    /// Cross-block (v2): stores in entry, load in successor. Single
    /// in-region predecessor → no phi needed; the load resolves
    /// directly to the stored value via `block_out` propagation.
    #[test]
    fn cross_block_linear_chain_sroa_v2() {
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
            (0, 0, 0),
            "cross-block alloc must SROA via v2 path"
        );
        let term = &body.blocks[bb1.0 as usize].terminator;
        match term {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 5.0),
            other => panic!("expected Return(Number(5)), got {other:?}"),
        }
        assert!(module.validate().is_ok());
    }

    /// Diamond: alloc in entry; if-branch stores 1, else-branch
    /// stores 2; load in join. Cross-block path inserts a block-param
    /// at the join with edge args from both branches.
    #[test]
    fn cross_block_diamond_inserts_phi() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let arg0 = b.body.params[0];
        let key_a = b.intern("a");
        let bb_entry = b.new_block();
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb_join = b.new_block();
        b.switch_to_block(bb_entry);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(arg0),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.switch_to_block(bb_then);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_else);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(2.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_join);
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
            (0, 0, 0),
            "diamond alloc must fully SROA"
        );
        let join_blk = &body.blocks[bb_join.0 as usize];
        assert_eq!(
            join_blk.params.len(),
            1,
            "join must have exactly one phi (one tracked key)"
        );
        let phi = join_blk.params[0];
        match &join_blk.terminator {
            Terminator::Return {
                val: Some(Operand::Value(v)),
            } => assert_eq!(*v, phi),
            other => panic!("expected Return(phi), got {other:?}"),
        }
        let then_args = body.blocks[bb_then.0 as usize]
            .terminator
            .edge_args_to(bb_join)
            .unwrap();
        let else_args = body.blocks[bb_else.0 as usize]
            .terminator
            .edge_args_to(bb_join)
            .unwrap();
        assert!(matches!(
            then_args[0],
            Operand::Const(Constant::Number(n)) if n == 1.0
        ));
        assert!(matches!(
            else_args[0],
            Operand::Const(Constant::Number(n)) if n == 2.0
        ));
        assert!(module.validate().is_ok());
    }

    /// Diamond where both branches store the same const → block-param
    /// is trivial and gets folded by `try_remove_trivial_block_param`.
    #[test]
    fn cross_block_diamond_same_store_folds_phi() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let arg0 = b.body.params[0];
        let key_a = b.intern("a");
        let bb_entry = b.new_block();
        let bb_then = b.new_block();
        let bb_else = b.new_block();
        let bb_join = b.new_block();
        b.switch_to_block(bb_entry);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(arg0),
            then_b: bb_then,
            then_args: vec![],
            else_b: bb_else,
            else_args: vec![],
        });
        b.switch_to_block(bb_then);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(7.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_else);
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(7.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_join);
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
        assert_eq!((allocs, stores, loads), (0, 0, 0));
        let join_blk = &body.blocks[bb_join.0 as usize];
        assert!(
            join_blk.params.is_empty(),
            "trivial phi (both edges = same const) must fold"
        );
        match &join_blk.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => assert_eq!(*n, 7.0),
            other => panic!("expected Return(Number(7)), got {other:?}"),
        }
        assert!(module.validate().is_ok());
    }

    /// Loop: alloc in pre-header, load+store inside body. The loop
    /// header is a multi-pred join (entry + back-edge) so cross-block
    /// inserts a phi at the header for the slot, then patches the
    /// back-edge with the body's out-state.
    #[test]
    fn cross_block_loop_phi_at_header() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let arg0 = b.body.params[0];
        let key_a = b.intern("a");
        let bb_entry = b.new_block();
        let bb_header = b.new_block();
        let bb_body = b.new_block();
        let bb_exit = b.new_block();
        b.switch_to_block(bb_entry);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(0.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_header);
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(arg0),
            then_b: bb_body,
            then_args: vec![],
            else_b: bb_exit,
            else_args: vec![],
        });
        b.switch_to_block(bb_body);
        let l = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_header,
            args: vec![],
        });
        b.switch_to_block(bb_exit);
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
            (0, 0, 0),
            "loop alloc must fully SROA"
        );
        let header_blk = &body.blocks[bb_header.0 as usize];
        assert!(
            !header_blk.params.is_empty(),
            "loop header must carry a phi for the slot"
        );
        // Validation should pass — back-edge args were patched.
        assert!(module.validate().is_ok());
    }

    /// Cross-block alloc inside a try-body must bail (the catch
    /// handler can observe slot state at any may-throw site in the
    /// try-body, which requires exception-edge phi-arg modeling we
    /// don't yet support). v2-only guard: single-block allocs in a
    /// try-body remain sound (unaliased ⇒ catch can't observe them).
    #[test]
    fn cross_block_alloc_inside_try_bails() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let key_a = b.intern("a");
        let bb_entry = b.new_block();
        let bb_try = b.new_block();
        let bb_try_tail = b.new_block();
        let bb_catch = b.new_block();
        let bb_join = b.new_block();
        b.switch_to_block(bb_entry);
        b.set_terminator(Terminator::Try {
            try_entry: bb_try,
            try_args: vec![],
            catch_entry: Some(bb_catch),
            finally_entry: None,
            join: bb_join,
        });
        b.switch_to_block(bb_try);
        let o = b.add_inst(InstKind::AllocObject).unwrap();
        b.add_inst(InstKind::StoreProp {
            obj: Operand::Value(o),
            key: key_a,
            val: Operand::Const(Constant::Number(1.0)),
            purity: Purity::Opaque,
        });
        b.set_terminator(Terminator::Branch {
            target: bb_try_tail,
            args: vec![],
        });
        b.switch_to_block(bb_try_tail);
        // Cross-block load on the alloc inside the try-body.
        let _l = b
            .add_inst(InstKind::LoadProp {
                obj: Operand::Value(o),
                key: key_a,
                purity: Purity::Opaque,
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_catch);
        b.add_inst(InstKind::CatchValue);
        b.set_terminator(Terminator::Branch {
            target: bb_join,
            args: vec![],
        });
        b.switch_to_block(bb_join);
        b.set_terminator(Terminator::Return { val: None });
        // Stamp every try-body block's exception_target.
        b.body.blocks[bb_try.0 as usize].exception_target = Some(bb_catch);
        b.body.blocks[bb_try_tail.0 as usize].exception_target = Some(bb_catch);
        b.finish();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(crate::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(SroaPass));
        pm.run(&mut module, FuncId(0)).unwrap();

        let body = module.functions[0].lowered();
        let (allocs, stores, loads) = count_kinds(body);
        assert_eq!(
            allocs, 1,
            "cross-block alloc inside try-body must bail (v2 exception guard)"
        );
        assert_eq!(stores, 1);
        assert_eq!(loads, 1);
    }
}
