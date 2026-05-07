//! Multi-block clonable-body inlining — sibling to `IdentityFnInlinePass`.
//!
//! Recognises functions whose body is a bounded, side-effect-tame
//! sequence we can splice at every Call site, and rewrites those Calls
//! by cloning the body into the caller. After cloning, downstream
//! function passes (`ForwardLoadsPass`, `ConstFoldPass`, `DcePass`,
//! …) collapse anything that resolves.
//!
//! Started as constructor-function inlining (a function returning a
//! freshly-built alloc); A.1 generalised it to any single-block body in
//! a bounded admissible language (alloc / store-on-tracked /
//! load-on-tracked / pure-or-may-throw expressions / Call to another
//! clonable function). #39 lifts the single-block restriction.
//!
//! ## Admissible per-instruction language (unchanged from A.1)
//!
//! - `AllocObject` / `AllocArray` — produces a *tracked alloc*. Each
//!   call site gets its own fresh alloc Value, preserving JS
//!   identity-per-call semantics.
//! - `StoreProp` / `StoreIndex` whose `obj` is a tracked alloc, key is
//!   `Const(_)`, and `val` is "allowed".
//! - `LoadProp` / `LoadIndex` whose `obj` is a tracked alloc.
//! - A `Call` whose callee is `Const(Func(g))` with `g` already in the
//!   summaries map. Recursively expanded at clone time (flat-summary
//!   targets only — see Phase-1 note below).
//! - Any other "expression" instruction whose effect is a subset of
//!   `PURE | MAY_THROW` and whose operands are all allowed.
//!
//! ## Multi-block extension (#39)
//!
//! - Body's blocks may form an acyclic (no back-edges) reducible CFG.
//!   Cyclic bodies (loops) are rejected for now — the dataflow shape
//!   below is monotone but the final clone-time materialisation needs
//!   loop-aware block reordering we haven't built.
//! - Terminators admitted: `Return`, `Branch`, `CondBranch`, `Switch`,
//!   `Throw`, `Unreachable`. `Try` is rejected outright (per-instruction
//!   exception edges (#34) aren't on this branch yet, and threading a
//!   try-bearing body through clone-time would silently lose the
//!   handler relationship).
//! - The matcher walks blocks in reverse-postorder; per-block state
//!   `(allowed_values, tracked_allocs)` is computed by intersecting
//!   predecessors' converged out-states. Block params are admissible
//!   iff every incoming edge arg is allowed; tracked iff every incoming
//!   `Operand::Value` is itself a tracked alloc (a single non-tracked
//!   incoming edge demotes the param).
//! - Recursive splice (`Call` to a clonable callee) is restricted to
//!   FLAT-summary callees in multi-block bodies. The clone path for
//!   multi-block callees would need its own block-allocation step
//!   inside an ongoing clone; deferred.
//!
//! ## Two clone-time paths
//!
//! 1. **Flat splice** (single-block body, sole terminator `Return`):
//!    cloned instructions are spliced directly into the caller's block
//!    at the call position; the call is retired and its uses are
//!    rewritten to the cloned return Operand. No block-graph edits.
//!    Existing back-compat path; preserves the IR shape that pre-#39
//!    tests assert against.
//! 2. **Multi-block splice** (any body that isn't single-block-Return):
//!    fresh BlockIds are allocated for every cloned block plus a
//!    *continuation* block that takes a single block-param holding the
//!    returned Value. The caller's containing block is split at the
//!    Call: pre-call instructions stay; the original terminator and
//!    post-call instructions move to the continuation; the original
//!    block now ends in `Branch(cloned_entry)`. Each cloned `Return`
//!    becomes `Branch(continuation, [return_op])`. The Call's result
//!    Value uses are rewritten to the continuation's block-param. The
//!    body's `body.params` Values map directly to the call-site arg
//!    Operands (entry block has no block-params, by matcher invariant).
//!
//! ## Phase-1 fixed point
//!
//! Each iteration grows the summaries map by trying to match every
//! not-yet-summarised function against the current map; stable when no
//! new entries. Bound is the function count. A function's matcher run
//! sees the current snapshot of summaries — including multi-block ones
//! — but only flat summaries are admissible as recursive-splice
//! targets inside another body (see above). This restriction is
//! enforced in the matcher's `Call` branch.
//!
//! ## Soundness notes
//!
//! - Each Call site receives freshly-cloned alloc Values; identity
//!   semantics preserved across `==`/`is`-style comparisons.
//! - Loads on tracked allocs remain Pure post-clone because the cloned
//!   alloc is unaliased in the caller (identity-per-call).
//! - Calls to non-clonable functions disqualify (would change
//!   observable side effects relative to the wrapping Call).
//! - Throw operands flow through the matcher's allowed-operand check;
//!   the throw's effect (`MAY_THROW`) is a subset of the wrapping
//!   Call's effect tier, so cloning preserves the observable.
//! - Multi-block matcher rejects bodies with back-edges, so RPO is a
//!   complete forward order and the analysis converges in one pass.

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    BlockId, Constant, Effect, FuncId, FunctionBody, InstId, InstKind, Module, Operand, StrId,
    SwitchCase, Terminator, Value, ValueDef, ValueDefKind, find_value_for_inst,
    walk_inst_operands_mut,
};

use crate::ModulePass;

pub struct ConstructorFnInlinePass;

#[derive(Clone, Debug)]
struct Step {
    kind: InstKind,
    /// `Some(body_value_idx)` if this step produces a Value.
    produced: Option<u32>,
    /// `Some(target_fid)` for `Call`-to-clonable steps; expanded
    /// recursively at clone time instead of cloned literally. Only
    /// populated for FLAT-summary callees.
    recurse_target: Option<u32>,
}

#[derive(Clone, Debug)]
struct FlatSummary {
    arity: usize,
    steps: Vec<Step>,
    /// Body Value index returned by the function.
    return_body_value: u32,
    /// True iff the return Value is a tracked alloc — used by callers'
    /// matchers to admit stores/loads against this function's result,
    /// and to chain through nested clonable-Call returns.
    returns_alloc: bool,
}

/// One block of a multi-block body, in matcher-order. Body BlockIds
/// (`u32`) are kept verbatim — at clone time they get remapped to fresh
/// caller-side BlockIds.
#[derive(Clone, Debug)]
struct BlockDesc {
    /// Body BlockId of this block.
    body_block: u32,
    /// Body Values that are this block's params. Each gets a fresh
    /// caller-side Value (and a fresh block-param slot) at clone time.
    params: Vec<u32>,
    steps: Vec<Step>,
    term: TermDesc,
}

/// Terminator description in body terms — operands reference body
/// Values / Consts; BlockId targets reference body BlockIds. Both are
/// remapped at clone time.
#[derive(Clone, Debug)]
enum TermDesc {
    Branch {
        target: u32,
        args: Vec<Operand>,
    },
    CondBranch {
        cond: Operand,
        then_b: u32,
        then_args: Vec<Operand>,
        else_b: u32,
        else_args: Vec<Operand>,
    },
    Switch {
        val: Operand,
        cases: Vec<(Constant, u32, Vec<Operand>)>,
        default: u32,
        default_args: Vec<Operand>,
        join: u32,
    },
    Return {
        val: Option<Operand>,
    },
    Throw {
        val: Operand,
    },
    Unreachable,
}

#[derive(Clone, Debug)]
struct MultiSummary {
    arity: usize,
    /// In matcher walk order (reverse-postorder from entry).
    blocks: Vec<BlockDesc>,
    /// Body BlockId of the entry.
    entry: u32,
    /// True iff EVERY Return in the body returns a tracked alloc.
    /// Reserved for future cross-summary chain tracking (a multi-block
    /// summary that "returns alloc" could let its caller's matcher
    /// admit stores/loads on the call result, like flat summaries do
    /// today). Multi-block summaries are NOT yet admissible as
    /// recursive-splice targets, so this field is currently unused;
    /// keep populating it so the analysis is correct when the gate is
    /// eventually lifted.
    #[allow(dead_code)]
    returns_alloc: bool,
}

#[derive(Clone, Debug)]
enum InlineSummary {
    Flat(FlatSummary),
    Multi(MultiSummary),
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
            // Skip self-recursion: don't try to inline a function's
            // calls into itself. Avoids a clone-during-walk soundness
            // headache; tail calls / single-step recursion are best
            // handled by separate optimisations.
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
                        && fid.0 != fid_idx as u32
                        && summaries.contains_key(&fid.0)
                    {
                        rewrites.push((bi, pos, iid.0, fid.0));
                    }
                }
            }

            if rewrites.is_empty() {
                continue;
            }

            // Group by block; per-block process in reverse position
            // order so earlier in-block splices don't shift later ones.
            // Across blocks, order is irrelevant (each block's splice
            // only edits its own instruction slice + appends new
            // blocks).
            rewrites.sort_by_key(|&(bi, pos, _, _)| (bi, std::cmp::Reverse(pos)));

            let mut needs_predecessors_rebuild = false;
            for (bi, pos, call_iid, target_fid) in rewrites {
                let summary = summaries.get(&target_fid).unwrap().clone();
                match &summary {
                    InlineSummary::Flat(f) => {
                        inline_flat_at_call_site(
                            body,
                            bi,
                            pos,
                            InstId(call_iid),
                            f,
                            &summaries,
                        );
                    }
                    InlineSummary::Multi(m) => {
                        inline_multi_at_call_site(
                            body,
                            bi,
                            pos,
                            InstId(call_iid),
                            m,
                        );
                        needs_predecessors_rebuild = true;
                    }
                }
                any_changed = true;
            }
            if needs_predecessors_rebuild {
                body.rebuild_predecessors();
            }
        }
        Ok(any_changed)
    }
}

/// Permissive shape predicate exposed for `DeadFnElimPass`. Returns
/// true iff `func`'s body could plausibly be a clonable wrapper. The
/// shape predicate's contract is "could be inlined" regardless of
/// block count — multi-block bodies are now in scope.
pub fn is_inlinable_shape(func: &ir::ssa::Function) -> bool {
    match_shape_only(func)
}

/// Back-compat alias.
pub fn is_constructor_shape(func: &ir::ssa::Function) -> bool {
    is_inlinable_shape(func)
}

fn match_shape_only(func: &ir::ssa::Function) -> bool {
    // Shape-only check: rerun the full matcher with an empty summaries
    // map (so the Call-to-clonable rule degrades to "Call to any Func
    // const, treated permissively"). This is over-acceptance, which is
    // fine for DeadFnElim's purposes (a function with non-clonable
    // inner calls won't have its callers inlined either, so its
    // Const(Func) uses won't drop to zero).
    //
    // We only need a yes/no answer, so we can fully reuse `match_inlinable`'s
    // structural guards by feeding it a fake summaries map that "knows"
    // about every callee. We approximate that by preflighting: if the
    // body has any blocks/steps that match the per-instruction language,
    // accept.
    if func.is_pass_through() {
        return false;
    }
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return false,
    };
    if body.blocks.is_empty() {
        return false;
    }
    let entry = &body.blocks[body.entry.0 as usize];
    if !entry.params.is_empty() || entry.instructions.is_empty() {
        return false;
    }
    // Try running the relaxed (multi-block) matcher with a permissive
    // summaries oracle that pretends every Call target is flat &
    // returns alloc. We approximate this by post-processing: do a
    // first-pass walk of all blocks to collect every Func target the
    // body Calls, then build a stub summaries map of those.
    let mut stub: HashMap<u32, InlineSummary> = HashMap::new();
    for blk in &body.blocks {
        for &iid in &blk.instructions {
            if let InstKind::Call {
                callee: Operand::Const(Constant::Func(fid)),
                ..
            } = &body.instructions[iid.0 as usize].kind
            {
                stub.entry(fid.0).or_insert_with(|| {
                    InlineSummary::Flat(FlatSummary {
                        arity: 0,
                        steps: Vec::new(),
                        return_body_value: 0,
                        returns_alloc: true,
                    })
                });
            }
        }
    }
    match_inlinable(func, &stub).is_some()
}

/// Match the inlinable shape on `func` against the current summaries
/// map; returns the summary if it matches. Dispatches between the
/// flat single-block-Return path and the multi-block path.
fn match_inlinable(
    func: &ir::ssa::Function,
    summaries: &HashMap<u32, InlineSummary>,
) -> Option<InlineSummary> {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    if body.blocks.is_empty() {
        return None;
    }
    let entry_idx = body.entry.0 as usize;
    let entry = &body.blocks[entry_idx];
    if !entry.params.is_empty() {
        return None;
    }

    // Single-block-Return → flat path (back-compat).
    if body.blocks.len() == 1 {
        if let Terminator::Return { val: Some(_) } = &entry.terminator {
            return match_flat(func, summaries).map(InlineSummary::Flat);
        }
    }
    match_multi(func, summaries).map(InlineSummary::Multi)
}

/// Single-block-Return matcher (the A.1 path). Produces a flat summary
/// suitable for in-place splice cloning.
fn match_flat(
    func: &ir::ssa::Function,
    summaries: &HashMap<u32, InlineSummary>,
) -> Option<FlatSummary> {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    let entry = &body.blocks[body.entry.0 as usize];
    if entry.instructions.is_empty() {
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
    for &iid in &entry.instructions {
        let kind = &body.instructions[iid.0 as usize].kind;
        let step = check_inst(
            kind,
            body,
            iid,
            &mut allowed_values,
            &mut tracked_allocs,
            summaries,
            /*allow_recurse=*/ true,
        )?;
        steps.push(step);
    }

    let return_body_value = match &entry.terminator {
        Terminator::Return {
            val: Some(Operand::Value(v)),
        } => v.0,
        _ => return None,
    };
    if !allowed_values.contains(&return_body_value) {
        return None;
    }
    let returns_alloc = tracked_allocs.contains(&return_body_value);

    Some(FlatSummary {
        arity,
        steps,
        return_body_value,
        returns_alloc,
    })
}

/// Multi-block matcher. Walks blocks in RPO; per-block state is the
/// intersection of predecessors' converged out-states. Rejects:
/// - any block with `Terminator::Try`,
/// - any back-edge (loops),
/// - any unreachable block,
/// - any inadmissible inst.
fn match_multi(
    func: &ir::ssa::Function,
    summaries: &HashMap<u32, InlineSummary>,
) -> Option<MultiSummary> {
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(_) => return None,
    };
    let n = body.blocks.len();
    let entry = body.entry.0 as usize;

    // Reject try-bearing bodies up front.
    for blk in &body.blocks {
        if matches!(blk.terminator, Terminator::Try { .. }) {
            return None;
        }
    }

    // RPO + back-edge detection via DFS. Returns None if any back-edge.
    let rpo = compute_rpo_acyclic(body, body.entry)?;
    if rpo.len() != n {
        // Some block is unreachable from entry.
        return None;
    }

    // Per-block in/out state: (allowed_values, tracked_allocs). We
    // also track allowed/tracked per block param. Indexed by body
    // BlockId (block idx).
    //
    // Special "uninit" sentinel: out_states[i] = None means we haven't
    // computed it yet. Since RPO visits each block after all its
    // predecessors (no back-edges), we never look at an uninit pred.
    let mut out_states: Vec<Option<(HashSet<u32>, HashSet<u32>)>> = vec![None; n];

    // Seed: every Arg is an allowed value (not tracked).
    let mut entry_allowed: HashSet<u32> = HashSet::new();
    for v_idx in 0..body.values.len() {
        if let ValueDefKind::Arg(_) = body.values[v_idx].kind {
            entry_allowed.insert(v_idx as u32);
        }
    }
    let entry_tracked: HashSet<u32> = HashSet::new();

    // Walk in RPO; build BlockDescs.
    let mut blocks_desc: Vec<BlockDesc> = Vec::with_capacity(n);
    // Map from body BlockId → index in blocks_desc, for ordered lookup.
    // We don't strictly need this — clone-time uses body BlockIds
    // directly — but it helps keep blocks_desc in RPO.
    let mut all_returns_alloc = true;
    let mut saw_return = false;

    for &bi in &rpo {
        let blk = &body.blocks[bi];
        // Compute in-state from predecessors' out-states (intersection).
        let (mut in_allowed, mut in_tracked) = if bi == entry {
            (entry_allowed.clone(), entry_tracked.clone())
        } else {
            let preds = &blk.predecessors;
            if preds.is_empty() {
                // Non-entry block with no preds is unreachable — should
                // have been caught by RPO check above.
                return None;
            }
            // Start with first pred's out, intersect with the rest.
            let mut iter = preds.iter().filter_map(|p| out_states[p.0 as usize].as_ref());
            let first = iter.next()?.clone();
            iter.fold(first, |(mut a_allowed, mut a_tracked), (b_allowed, b_tracked)| {
                a_allowed.retain(|v| b_allowed.contains(v));
                a_tracked.retain(|v| b_tracked.contains(v));
                (a_allowed, a_tracked)
            })
        };

        // Block params: each param is allowed iff every incoming edge
        // arg is allowed, and tracked iff every incoming Operand::Value
        // is itself tracked. A non-Value arg (Const) on any edge
        // demotes "tracked" but keeps "allowed" (Consts are allowed).
        let mut param_body_idxs: Vec<u32> = Vec::with_capacity(blk.params.len());
        for (p_idx, p_v) in blk.params.iter().enumerate() {
            let mut all_allowed = true;
            let mut all_tracked = true;
            // Entry block has no params (matcher invariant) — but if
            // someone forced it, treat preds as empty → reject (no edge
            // info for an "entry" param).
            if bi == entry && blk.predecessors.is_empty() {
                return None;
            }
            for pred in &blk.predecessors {
                let term = &body.blocks[pred.0 as usize].terminator;
                let args = term.edge_args_to(BlockId(bi as u32))?;
                if p_idx >= args.len() {
                    return None;
                }
                let arg = &args[p_idx];
                let pred_state = out_states[pred.0 as usize].as_ref()?;
                match arg {
                    Operand::Const(_) => {
                        all_tracked = false;
                    }
                    Operand::Value(v) => {
                        if !pred_state.0.contains(&v.0) {
                            all_allowed = false;
                        }
                        if !pred_state.1.contains(&v.0) {
                            all_tracked = false;
                        }
                    }
                    _ => {
                        all_allowed = false;
                        all_tracked = false;
                    }
                }
            }
            if !all_allowed {
                return None;
            }
            in_allowed.insert(p_v.0);
            if all_tracked {
                in_tracked.insert(p_v.0);
            }
            param_body_idxs.push(p_v.0);
        }

        // Walk the block's instructions.
        let mut steps: Vec<Step> = Vec::new();
        for &iid in &blk.instructions {
            let kind = &body.instructions[iid.0 as usize].kind;
            // In multi-block bodies we currently only allow recursive
            // splice for FLAT callees — multi-block recursive splice
            // isn't implemented (would need to allocate fresh blocks
            // mid-clone). Detected by `allow_recurse=false` for
            // multi-block targets in `check_inst`.
            let step = check_inst(
                kind,
                body,
                iid,
                &mut in_allowed,
                &mut in_tracked,
                summaries,
                /*allow_recurse=*/ true,
            )?;
            steps.push(step);
        }

        // Walk the terminator.
        let term = match &blk.terminator {
            Terminator::Branch { target, args } => {
                for a in args {
                    if !operand_allowed(a, &in_allowed) {
                        return None;
                    }
                }
                TermDesc::Branch {
                    target: target.0,
                    args: args.clone(),
                }
            }
            Terminator::CondBranch {
                cond,
                then_b,
                then_args,
                else_b,
                else_args,
            } => {
                if !operand_allowed(cond, &in_allowed) {
                    return None;
                }
                for a in then_args.iter().chain(else_args.iter()) {
                    if !operand_allowed(a, &in_allowed) {
                        return None;
                    }
                }
                TermDesc::CondBranch {
                    cond: *cond,
                    then_b: then_b.0,
                    then_args: then_args.clone(),
                    else_b: else_b.0,
                    else_args: else_args.clone(),
                }
            }
            Terminator::Switch {
                val,
                cases,
                default,
                default_args,
                join,
            } => {
                if !operand_allowed(val, &in_allowed) {
                    return None;
                }
                for a in default_args {
                    if !operand_allowed(a, &in_allowed) {
                        return None;
                    }
                }
                let mut cases_desc: Vec<(Constant, u32, Vec<Operand>)> = Vec::new();
                for c in cases {
                    for a in &c.args {
                        if !operand_allowed(a, &in_allowed) {
                            return None;
                        }
                    }
                    cases_desc.push((c.key.clone(), c.target.0, c.args.clone()));
                }
                TermDesc::Switch {
                    val: *val,
                    cases: cases_desc,
                    default: default.0,
                    default_args: default_args.clone(),
                    join: join.0,
                }
            }
            Terminator::Return { val } => {
                saw_return = true;
                let ret_alloc = match val {
                    None => false,
                    Some(op) => {
                        if !operand_allowed(op, &in_allowed) {
                            return None;
                        }
                        match op {
                            Operand::Value(v) => in_tracked.contains(&v.0),
                            _ => false,
                        }
                    }
                };
                if !ret_alloc {
                    all_returns_alloc = false;
                }
                TermDesc::Return { val: *val }
            }
            Terminator::Throw { val } => {
                if !operand_allowed(val, &in_allowed) {
                    return None;
                }
                TermDesc::Throw { val: *val }
            }
            Terminator::Unreachable => TermDesc::Unreachable,
            Terminator::Try { .. } => return None,
        };

        out_states[bi] = Some((in_allowed, in_tracked));
        blocks_desc.push(BlockDesc {
            body_block: bi as u32,
            params: param_body_idxs,
            steps,
            term,
        });
    }

    if !saw_return {
        // No Return reached → body always Throws / loops to Unreachable.
        // Conservative: reject (caller's value-substitute would be
        // unconstrained).
        return None;
    }

    Some(MultiSummary {
        arity: body.params.len(),
        blocks: blocks_desc,
        entry: entry as u32,
        returns_alloc: all_returns_alloc,
    })
}

/// Check a single instruction against the admissible language;
/// updates `allowed_values`/`tracked_allocs` in place and returns the
/// `Step` descriptor on success.
fn check_inst(
    kind: &InstKind,
    body: &ir::ssa::LoweredBody,
    iid: InstId,
    allowed_values: &mut HashSet<u32>,
    tracked_allocs: &mut HashSet<u32>,
    summaries: &HashMap<u32, InlineSummary>,
    allow_recurse: bool,
) -> Option<Step> {
    let allowed_effect = Effect::PURE | Effect::MAY_THROW;
    match kind {
        InstKind::AllocObject | InstKind::AllocArray { .. } => {
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            tracked_allocs.insert(v.0);
            Some(Step {
                kind: kind.clone(),
                produced: Some(v.0),
                recurse_target: None,
            })
        }
        InstKind::StoreProp { obj, val, .. } => {
            let obj_v = match obj {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            if !tracked_allocs.contains(&obj_v) {
                return None;
            }
            if !operand_allowed(val, allowed_values) {
                return None;
            }
            Some(Step {
                kind: kind.clone(),
                produced: None,
                recurse_target: None,
            })
        }
        InstKind::StoreIndex {
            obj, index, val, ..
        } => {
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
            if !operand_allowed(val, allowed_values) {
                return None;
            }
            Some(Step {
                kind: kind.clone(),
                produced: None,
                recurse_target: None,
            })
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
            Some(Step {
                kind: kind.clone(),
                produced: Some(v.0),
                recurse_target: None,
            })
        }
        InstKind::LoadIndex { obj, index, .. } => {
            let obj_v = match obj {
                Operand::Value(v) => v.0,
                _ => return None,
            };
            if !tracked_allocs.contains(&obj_v) {
                return None;
            }
            if !operand_allowed(index, allowed_values) {
                return None;
            }
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            Some(Step {
                kind: kind.clone(),
                produced: Some(v.0),
                recurse_target: None,
            })
        }
        InstKind::Call { callee, args } => {
            let target_fid = match callee {
                Operand::Const(Constant::Func(fid)) => fid.0,
                _ => return None,
            };
            // Only flat summaries are admissible as recursive-splice
            // targets (multi-block recursive splice isn't implemented).
            let inner = match summaries.get(&target_fid) {
                Some(InlineSummary::Flat(f)) if allow_recurse => f,
                _ => return None,
            };
            for a in args {
                if !operand_allowed(a, allowed_values) {
                    return None;
                }
            }
            let v = find_value_for_inst(body, iid);
            allowed_values.insert(v.0);
            if inner.returns_alloc {
                tracked_allocs.insert(v.0);
            }
            Some(Step {
                kind: kind.clone(),
                produced: Some(v.0),
                recurse_target: Some(target_fid),
            })
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
            Some(Step {
                kind: kind.clone(),
                produced: Some(produced_v.0),
                recurse_target: None,
            })
        }
    }
}

/// Compute reverse-postorder from `entry`. Returns `None` if any
/// back-edge is encountered (the body has a cycle). All blocks
/// reachable from `entry` are in the result; unreachable blocks are
/// excluded — caller compares len() to `body.blocks.len()` to detect
/// dead blocks.
fn compute_rpo_acyclic(body: &ir::ssa::LoweredBody, entry: BlockId) -> Option<Vec<usize>> {
    enum Color {
        White,
        Gray,
        Black,
    }
    let n = body.blocks.len();
    let mut color: Vec<Color> = (0..n).map(|_| Color::White).collect();
    let mut postorder: Vec<usize> = Vec::with_capacity(n);
    // Iterative DFS using a stack of (node, succ_iter_index).
    let mut stack: Vec<(usize, usize)> = Vec::new();
    color[entry.0 as usize] = Color::Gray;
    stack.push((entry.0 as usize, 0));

    while let Some(&(node, idx)) = stack.last() {
        let succs: Vec<usize> = body.blocks[node]
            .terminator
            .successors()
            .iter()
            .map(|(t, _)| t.0 as usize)
            .collect();
        if idx < succs.len() {
            let s = succs[idx];
            // Bump the iter index on the current frame.
            stack.last_mut().unwrap().1 = idx + 1;
            if s >= n {
                return None;
            }
            match color[s] {
                Color::White => {
                    color[s] = Color::Gray;
                    stack.push((s, 0));
                }
                Color::Gray => {
                    // Back-edge → cycle.
                    return None;
                }
                Color::Black => { /* cross/forward edge — fine */ }
            }
        } else {
            color[node] = Color::Black;
            postorder.push(node);
            stack.pop();
        }
    }

    let mut rpo = postorder;
    rpo.reverse();
    Some(rpo)
}

fn operand_allowed(op: &Operand, allowed_values: &HashSet<u32>) -> bool {
    match op {
        Operand::Const(_) => true,
        Operand::Value(v) => allowed_values.contains(&v.0),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Flat (single-block-Return) cloning
// ---------------------------------------------------------------------------

/// Splice `summary` into the caller at `pos`, retire the original Call,
/// and replace the Call's result Value uses with the cloned return.
fn inline_flat_at_call_site(
    body: &mut ir::ssa::LoweredBody,
    bi: usize,
    pos: usize,
    call_iid: InstId,
    summary: &FlatSummary,
    summaries: &HashMap<u32, InlineSummary>,
) {
    let call_args: Vec<Operand> = match &body.instructions[call_iid.0 as usize].kind {
        InstKind::Call { args, .. } => args.clone(),
        _ => unreachable!("inline_flat_at_call_site called on non-Call inst"),
    };
    let (new_iids, return_op) = build_flat_clone(body, summary, &call_args, summaries);

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

/// Materialise a flat summary's body into the caller's instruction/
/// value arrays, with operands remapped from inner-body Values to
/// caller-side Operands. Recurses through `recurse_target` steps by
/// splicing the inner FLAT summary instead of cloning the Call.
fn build_flat_clone(
    body: &mut ir::ssa::LoweredBody,
    summary: &FlatSummary,
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
            // Multi-block summaries are NOT admissible as recurse
            // targets; the matcher's allow_recurse=true gate enforces
            // this — only Flat summaries can populate `recurse_target`.
            let inner = match summaries.get(&target_fid) {
                Some(InlineSummary::Flat(f)) => f.clone(),
                _ => unreachable!("recurse_target must point to a Flat summary"),
            };
            let (inner_iids, inner_return_op) =
                build_flat_clone(body, &inner, &translated, summaries);
            new_iids.extend(inner_iids);
            if let Some(orig) = step.produced {
                remap.insert(orig, inner_return_op);
            }
        } else {
            let new_iid = InstId(body.instructions.len() as u32);
            let new_kind = clone_inst_with_remap(&step.kind, &remap);
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

fn clone_inst_with_remap(src: &InstKind, remap: &HashMap<u32, Operand>) -> InstKind {
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

// ---------------------------------------------------------------------------
// Multi-block cloning
// ---------------------------------------------------------------------------

/// Materialise a multi-block summary into the caller. Splits the
/// caller's containing block at the call position, allocates fresh
/// caller-side blocks for each cloned body block plus a continuation,
/// rewires terminators, and rewrites the Call's result Value uses to
/// the continuation's block-param.
fn inline_multi_at_call_site(
    body: &mut ir::ssa::LoweredBody,
    caller_bi: usize,
    call_pos: usize,
    call_iid: InstId,
    summary: &MultiSummary,
) {
    let call_args: Vec<Operand> = match &body.instructions[call_iid.0 as usize].kind {
        InstKind::Call { args, .. } => args.clone(),
        _ => unreachable!("inline_multi_at_call_site called on non-Call inst"),
    };

    // Step 1: allocate fresh BlockIds for every cloned body block plus
    // a continuation block. Body BlockId → caller BlockId.
    //
    // The cloned region lives inside the caller's lexical scope, so it
    // inherits the call-site block's `exception_target`. The matcher
    // rejected try-bearing bodies, so the cloned body itself contains
    // no try, but if the CALLER's call site is inside a try, every
    // cloned block must still route exceptions to the enclosing handler.
    let inherited_xtarget = body.blocks[caller_bi].exception_target;
    let mut block_remap: HashMap<u32, BlockId> = HashMap::new();
    for desc in &summary.blocks {
        let new_bid = BlockId(body.blocks.len() as u32);
        block_remap.insert(desc.body_block, new_bid);
        body.blocks.push(ir::ssa::BasicBlock {
            params: Vec::new(),
            instructions: Vec::new(),
            terminator: Terminator::Unreachable,
            predecessors: Vec::new(),
            exception_target: inherited_xtarget,
        });
    }
    let cont_bid = BlockId(body.blocks.len() as u32);
    body.blocks.push(ir::ssa::BasicBlock {
        params: Vec::new(),
        instructions: Vec::new(),
        terminator: Terminator::Unreachable,
        predecessors: Vec::new(),
        exception_target: inherited_xtarget,
    });

    // Step 2: continuation block-param holding the returned Value.
    let cont_param_v = Value(body.values.len() as u32);
    body.values.push(ValueDef::block_param(cont_bid, 0));
    body.blocks[cont_bid.0 as usize].params.push(cont_param_v);

    // Step 3: build per-body-Value remap. Args first; cloned-block
    // params and instruction-produced values are filled as we walk.
    let mut remap: HashMap<u32, Operand> = HashMap::new();
    for i in 0..summary.arity {
        let sub = call_args
            .get(i)
            .copied()
            .unwrap_or(Operand::Const(Constant::Undefined));
        remap.insert(i as u32, sub);
    }

    // Step 4: allocate fresh block-params for every cloned block (in
    // RPO order, i.e., summary.blocks order). We allocate before
    // walking instructions so that incoming-edge args from later blocks
    // can reference these already-mapped params.
    for desc in &summary.blocks {
        let new_bid = block_remap[&desc.body_block];
        for (p_idx, &orig_v) in desc.params.iter().enumerate() {
            let new_v = Value(body.values.len() as u32);
            body.values.push(ValueDef::block_param(new_bid, p_idx as u32));
            body.blocks[new_bid.0 as usize].params.push(new_v);
            remap.insert(orig_v, Operand::Value(new_v));
        }
    }

    // Step 5: walk each block, cloning instructions (recording fresh
    // Values) and terminators (with operand + BlockId remapping).
    for desc in &summary.blocks {
        let new_bid = block_remap[&desc.body_block];
        let mut new_iids: Vec<InstId> = Vec::with_capacity(desc.steps.len());
        for step in &desc.steps {
            // recurse_target inside multi-block bodies — only flat
            // summaries are valid here. We don't support nested multi-
            // splice (matcher already enforces).
            //
            // For uniformity we splice flat summaries inline (same as
            // build_flat_clone does in flat-mode), feeding the result
            // Operand back into the remap.
            if let Some(target_fid) = step.recurse_target {
                // Translate inner call args via the current remap.
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
                // The recurse target is guaranteed Flat by the matcher.
                // We need a non-mutable HashMap reference to summaries
                // here — but we only have `summary` in scope. Reusing
                // build_flat_clone without the summaries map is fine
                // because flat callees don't recurse into multi-block
                // ones; if THEY recurse to other flat callees, those
                // were fully expanded into THEIR steps at their own
                // matcher run? No — matcher doesn't expand transitively;
                // it leaves recurse_target steps for clone time. So we
                // DO need access to the global summaries map.
                //
                // Sound-but-conservative shortcut: if the matcher
                // recorded a `recurse_target` we can't expand here,
                // emit the Call literally instead. Since we don't have
                // the summaries map, do this. The downstream pipeline
                // will run another fixed-point iteration of
                // ConstructorFnInline and pick this Call up.
                //
                // (Plumbing the summaries map through here is feasible
                // but would significantly complicate the per-block
                // splice; for now, fall back to literal cloning.)
                let _ = (target_fid, translated);
                let new_iid = InstId(body.instructions.len() as u32);
                let new_kind = clone_inst_with_remap(&step.kind, &remap);
                body.instructions
                    .push(ir::ssa::Instruction { kind: new_kind });
                if let Some(orig) = step.produced {
                    body.values.push(ValueDef::inst(new_iid));
                    let new_v = Value(body.values.len() as u32 - 1);
                    remap.insert(orig, Operand::Value(new_v));
                }
                new_iids.push(new_iid);
            } else {
                let new_iid = InstId(body.instructions.len() as u32);
                let new_kind = clone_inst_with_remap(&step.kind, &remap);
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
        body.blocks[new_bid.0 as usize].instructions = new_iids;

        // Terminator translation.
        let new_term = translate_term(&desc.term, &remap, &block_remap, cont_bid);
        body.blocks[new_bid.0 as usize].terminator = new_term;
    }

    // Step 6: split the caller's containing block. Move post-call
    // instructions and the original terminator into the continuation
    // block; terminate the original block with Branch(cloned_entry).
    let cloned_entry = block_remap[&summary.entry];

    // Save the original terminator and post-call instructions.
    let orig_term = std::mem::replace(
        &mut body.blocks[caller_bi].terminator,
        Terminator::Unreachable, // placeholder
    );
    // Instructions after the Call (positions call_pos+1 ..) move to
    // the continuation. The Call itself is at call_pos and gets
    // dropped from the original block.
    let n_in_orig = body.blocks[caller_bi].instructions.len();
    let post_call: Vec<InstId> = body.blocks[caller_bi]
        .instructions
        .drain((call_pos + 1).min(n_in_orig)..)
        .collect();
    // Drop the Call from the original block. After the drain above,
    // the Call is at call_pos (last surviving inst).
    let popped = body.blocks[caller_bi].instructions.pop();
    debug_assert_eq!(popped.map(|x| x.0), Some(call_iid.0));

    // Continuation block: post-call instructions, original terminator.
    body.blocks[cont_bid.0 as usize].instructions = post_call;
    body.blocks[cont_bid.0 as usize].terminator = orig_term;

    // Original block now branches to the cloned entry. The cloned
    // entry has no params (matcher invariant), so empty edge args.
    body.blocks[caller_bi].terminator = Terminator::Branch {
        target: cloned_entry,
        args: Vec::new(),
    };

    // Step 7: rewrite the Call's result Value uses to the continuation
    // block-param.
    let call_result_v = find_value_for_inst(body, call_iid);
    body.replace_value_uses(call_result_v, Operand::Value(cont_param_v));
    body.values[call_result_v.0 as usize] = ValueDef::removed();
    walk_inst_operands_mut(
        &mut body.instructions[call_iid.0 as usize].kind,
        &mut |op| {
            *op = Operand::Const(Constant::Undefined);
        },
    );

    // Predecessors are stale across the entire affected sub-CFG; the
    // top-level driver calls rebuild_predecessors() once after each
    // multi-block splice.
}

/// Translate a `TermDesc` into a real `Terminator` against the caller's
/// blocks. Operands go through the body-Value remap; BlockIds go
/// through the block-remap. `Return { val }` becomes
/// `Branch(cont_bid, [val])`.
fn translate_term(
    term: &TermDesc,
    remap: &HashMap<u32, Operand>,
    block_remap: &HashMap<u32, BlockId>,
    cont_bid: BlockId,
) -> Terminator {
    let map_op = |op: &Operand| -> Operand {
        match op {
            Operand::Value(v) => remap
                .get(&v.0)
                .copied()
                .unwrap_or(Operand::Const(Constant::Undefined)),
            other => *other,
        }
    };
    let map_args = |args: &[Operand]| -> Vec<Operand> { args.iter().map(map_op).collect() };
    let map_block = |b: u32| -> BlockId {
        *block_remap
            .get(&b)
            .expect("term targets a body BlockId not in block_remap")
    };

    match term {
        TermDesc::Branch { target, args } => Terminator::Branch {
            target: map_block(*target),
            args: map_args(args),
        },
        TermDesc::CondBranch {
            cond,
            then_b,
            then_args,
            else_b,
            else_args,
        } => Terminator::CondBranch {
            cond: map_op(cond),
            then_b: map_block(*then_b),
            then_args: map_args(then_args),
            else_b: map_block(*else_b),
            else_args: map_args(else_args),
        },
        TermDesc::Switch {
            val,
            cases,
            default,
            default_args,
            join,
        } => {
            let cases: Vec<SwitchCase> = cases
                .iter()
                .map(|(k, t, a)| SwitchCase {
                    key: k.clone(),
                    target: map_block(*t),
                    args: map_args(a),
                })
                .collect();
            Terminator::Switch {
                val: map_op(val),
                cases,
                default: map_block(*default),
                default_args: map_args(default_args),
                join: map_block(*join),
            }
        }
        TermDesc::Return { val } => {
            let ret_arg = match val {
                None => Operand::Const(Constant::Undefined),
                Some(op) => map_op(op),
            };
            Terminator::Branch {
                target: cont_bid,
                args: vec![ret_arg],
            }
        }
        TermDesc::Throw { val } => Terminator::Throw { val: map_op(val) },
        TermDesc::Unreachable => Terminator::Unreachable,
    }
}

// ============================================================================
// Function summaries (issue #40) — cross-procedural forwarding
// ============================================================================
//
// Public infrastructure that lets `ForwardLoadsPass` resolve
// `LoadProp/LoadIndex(call_result, k)` against a constructor-shape
// callee's stored shape WITHOUT physically inlining the constructor at
// the call site. Useful when the constructor is large or called from
// many sites — physical inlining would N×M-bloat the IR before
// downstream passes collapse it.
//
// v1 scope:
//   - Single-block constructor-shape bodies (alloc + stores + return alloc).
//   - Stores whose val is `Const(_)` or directly references an `Arg(i)`.
//   - Loads at the call site whose key is a known `(Idx | Str)` constant.
//   - All Call args at the resolution site must be `Const(_)` (so any
//     `ArgRef(i)` summary entry resolves to a Const).
//
// Out of scope:
//   - Multi-block bodies (would need block-level dataflow).
//   - Stores whose val is the result of an in-body expression — encoded
//     as `Unresolvable` so resolve returns None for that key.
//   - Recursive summary chains (decoder calls decoder calls constructor).

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
pub enum SummaryKey {
    Idx(i32),
    Str(StrId),
}

#[derive(Clone, Copy, Debug)]
enum SummaryVal {
    Const(Constant),
    ArgRef(u32),
    Unresolvable,
}

#[derive(Clone, Debug)]
pub struct FunctionSummary {
    stores: Vec<(SummaryKey, SummaryVal)>,
    arity: usize,
}

impl FunctionSummary {
    /// Resolve a load at `key` given the Call's `args`. Walks stores
    /// in reverse (latest wins). Returns None for keys never stored
    /// or whose latest store's val is unresolvable.
    pub fn resolve(&self, args: &[Operand], key: SummaryKey) -> Option<Operand> {
        let stored = self
            .stores
            .iter()
            .rev()
            .find_map(|(k, v)| if *k == key { Some(*v) } else { None })?;
        match stored {
            SummaryVal::Const(c) => Some(Operand::Const(c)),
            SummaryVal::ArgRef(i) => args.get(i as usize).copied(),
            SummaryVal::Unresolvable => None,
        }
    }

    pub fn arity(&self) -> usize {
        self.arity
    }
}

/// Helper for callers: extract `FuncId` from a `Call.callee` Operand
/// when it's a static `Func(_)` constant.
pub fn callee_func_id(callee: &Operand) -> Option<FuncId> {
    match callee {
        Operand::Const(Constant::Func(fid)) => Some(*fid),
        _ => None,
    }
}

/// Build summaries for every constructor-shape function in `module`.
/// Self-contained matcher: single block, optional zero-or-more
/// non-store insts (silently treated as opaque so any store referencing
/// their result becomes Unresolvable), and `Return(Some(Value(alloc)))`.
pub fn module_summaries(module: &Module) -> HashMap<u32, FunctionSummary> {
    let mut out = HashMap::new();
    for (i, func) in module.functions.iter().enumerate() {
        if let Some(summary) = match_simple_constructor(func) {
            out.insert(i as u32, summary);
        }
    }
    out
}

/// Single-block alloc-and-stores matcher. Distinct from
/// `match_inlinable`: this one is purely about extracting the
/// (key → val) shape for cross-procedural forwarding; it doesn't
/// need to track admissible-effect semantics or multi-block dataflow.
fn match_simple_constructor(func: &ir::ssa::Function) -> Option<FunctionSummary> {
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
    let mut alloc_value: Option<Value> = None;
    let mut stores: Vec<(SummaryKey, SummaryVal)> = Vec::new();

    for &iid in &entry.instructions {
        let kind = &body.instructions[iid.0 as usize].kind;
        match kind {
            InstKind::AllocObject | InstKind::AllocArray { .. } => {
                if alloc_value.is_some() {
                    return None;
                }
                alloc_value = Some(find_value_for_inst(body, iid));
            }
            InstKind::StoreProp { obj, key, val, .. } => {
                let av = alloc_value?;
                if !matches!(obj, Operand::Value(v) if *v == av) {
                    return None;
                }
                stores.push((SummaryKey::Str(*key), encode_summary_val(body, val)));
            }
            InstKind::StoreIndex { obj, index, val, .. } => {
                let av = alloc_value?;
                if !matches!(obj, Operand::Value(v) if *v == av) {
                    return None;
                }
                let k = match index {
                    Operand::Const(Constant::Int32(k)) => SummaryKey::Idx(*k),
                    Operand::Const(Constant::Number(n))
                        if n.is_finite()
                            && n.trunc() == *n
                            && *n >= i32::MIN as f64
                            && *n <= i32::MAX as f64 =>
                    {
                        SummaryKey::Idx(*n as i32)
                    }
                    Operand::Const(Constant::String(s)) => SummaryKey::Str(*s),
                    _ => continue,
                };
                stores.push((k, encode_summary_val(body, val)));
            }
            // Other instructions are tolerated (they'd block the
            // physical inliner's matcher, but for summary purposes
            // any store whose val references their result will encode
            // as Unresolvable, which is correct).
            _ => {}
        }
    }

    let alloc_value = alloc_value?;
    if !matches!(
        &entry.terminator,
        Terminator::Return { val: Some(Operand::Value(v)) } if *v == alloc_value
    ) {
        return None;
    }

    Some(FunctionSummary { stores, arity })
}

fn encode_summary_val(body: &ir::ssa::LoweredBody, val: &Operand) -> SummaryVal {
    match val {
        Operand::Const(c) => SummaryVal::Const(*c),
        Operand::Value(v) => match body.values[v.0 as usize].kind {
            ValueDefKind::Arg(i) => SummaryVal::ArgRef(i),
            _ => SummaryVal::Unresolvable,
        },
        _ => SummaryVal::Unresolvable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{
        Constant, FuncId, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator,
    };

    /// `function mk() { return {foo: 1}; }` — flat path.
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
        // helper has two blocks AND no Return-of-allowed-value, so it
        // doesn't match the multi-block matcher either.
        {
            let mut b = FunctionBuilder::new(&mut module, "helper", 0);
            let bb0 = b.new_block();
            let bb1 = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Branch { target: bb1, args: vec![] });
            b.switch_to_block(bb1);
            // helper now has a Call to itself (back-edge would cycle —
            // but a plain Return None passes multi-block matcher's
            // saw_return only if there's a returned-Value). Use a Call
            // to make it definitely non-clonable.
            let _ = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![],
                })
                .unwrap();
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

    /// **The decoder shape (A.1)** — flat-on-flat recursive splice.
    #[test]
    fn inlines_decoder_that_calls_constructor_then_indexes() {
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

    /// Diamond body — multi-Return collapses through continuation
    /// block-param. `pickPath(c) { if (c) return 1; else return 2; }`.
    /// Caller `f() { return pickPath(true); }` should resolve to 1.
    #[test]
    fn inlines_diamond_body_with_two_returns() {
        let mut module = Module::new();
        // FuncId(0) pickPath(c) — multi-block with diamond.
        {
            let mut b = FunctionBuilder::new(&mut module, "pickPath", 1);
            let arg0 = b.body().params[0];
            let bb0 = b.new_block();
            let bb_then = b.new_block();
            let bb_else = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Value(arg0),
                then_b: bb_then,
                then_args: vec![],
                else_b: bb_else,
                else_args: vec![],
            });
            b.switch_to_block(bb_then);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(1))),
            });
            b.switch_to_block(bb_else);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Const(Constant::Int32(2))),
            });
            b.finish();
        }
        // FuncId(1) f() = pickPath(true).
        {
            let mut b = FunctionBuilder::new(&mut module, "f", 0);
            let bb0 = b.new_block();
            b.switch_to_block(bb0);
            let call_v = b
                .add_inst(InstKind::Call {
                    callee: Operand::Const(Constant::Func(FuncId(0))),
                    args: vec![Operand::Const(Constant::Bool(true))],
                })
                .unwrap();
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(call_v)),
            });
            b.finish();
        }

        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(ConstructorFnInlinePass));
        // No const-fold yet — this test asserts the structural splice
        // and that the call has been retired; the Return now flows
        // through a continuation-param-from-CondBranch chain. Adding
        // const-fold + branch-folding would let it resolve to a const,
        // but those passes are out of this test's scope.
        pm.run_module(&mut module).unwrap();

        let f_body = module.functions[1].lowered();
        // Call must be retired.
        for blk in &f_body.blocks {
            for &iid in &blk.instructions {
                assert!(
                    !matches!(
                        f_body.instructions[iid.0 as usize].kind,
                        InstKind::Call { .. }
                    ),
                    "Call should be retired"
                );
            }
        }
        // There must be at least a CondBranch in the cloned graph.
        let has_cbr = f_body
            .blocks
            .iter()
            .any(|b| matches!(b.terminator, Terminator::CondBranch { .. }));
        assert!(has_cbr, "diamond clone should include a CondBranch");
    }

    /// Body with `Try` rejected by the multi-block matcher.
    #[test]
    fn rejects_try_bearing_body() {
        let mut module = Module::new();
        // mk has Try terminator → rejected.
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 0);
            let bb0 = b.new_block();
            let bb_try = b.new_block();
            let bb_catch = b.new_block();
            let bb_join = b.new_block();
            b.switch_to_block(bb0);
            b.set_terminator(Terminator::Try {
                try_entry: bb_try,
                try_args: vec![],
                catch_entry: Some(bb_catch),
                finally_entry: None,
                join: bb_join,
            });
            b.switch_to_block(bb_try);
            b.set_terminator(Terminator::Branch {
                target: bb_join,
                args: vec![],
            });
            b.switch_to_block(bb_catch);
            b.set_terminator(Terminator::Branch {
                target: bb_join,
                args: vec![],
            });
            b.switch_to_block(bb_join);
            b.set_terminator(Terminator::Return { val: None });
            b.finish();
        }
        // f() = mk()
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

        // Call to mk must remain.
        let f_body = module.functions[1].lowered();
        let entry = f_body.entry;
        let has_call = f_body.blocks[entry.0 as usize]
            .instructions
            .iter()
            .any(|iid| matches!(
                f_body.instructions[iid.0 as usize].kind,
                InstKind::Call { .. }
            ));
        assert!(has_call, "try-bearing body must NOT be inlined");
    }

    /// Decoder with arithmetic on the index: `dec(i) { return
    /// mkArr()[i + 1]; }; f() { return dec(0); }` → "b". Flat-on-flat
    /// recursive splice. Smoke test for the A.1 chain still working.
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

    /// Layered flat decoders (mkArr → dec1 → dec2 → f). Internal
    /// fixed-point iteration must accept dec2 by adding dec1 to the
    /// summaries first.
    #[test]
    fn inlines_layered_decoders() {
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

    /// Multi-block body with a tracked-alloc that survives across the
    /// diamond join (both branches store on the same alloc); the
    /// summary's tracked-alloc set propagates correctly. Caller's
    /// `obj.x` load resolves to `1`.
    ///
    /// `function mk() { var o = {}; if (true) o.x = 1; else o.x = 1;
    ///  return o; }`
    #[test]
    fn inlines_diamond_body_with_tracked_alloc() {
        let mut module = Module::new();
        let key_x;
        // FuncId(0) mk()
        {
            let mut b = FunctionBuilder::new(&mut module, "mk", 0);
            key_x = b.intern("x");
            let bb0 = b.new_block();
            let bb_then = b.new_block();
            let bb_else = b.new_block();
            let bb_join = b.new_block();
            b.switch_to_block(bb0);
            let alloc = b.add_inst(InstKind::AllocObject).unwrap();
            b.set_terminator(Terminator::CondBranch {
                cond: Operand::Const(Constant::Bool(true)),
                then_b: bb_then,
                then_args: vec![],
                else_b: bb_else,
                else_args: vec![],
            });
            b.switch_to_block(bb_then);
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_x,
                val: Operand::Const(Constant::Int32(1)),
                purity: Purity::Pure,
            });
            b.set_terminator(Terminator::Branch {
                target: bb_join,
                args: vec![],
            });
            b.switch_to_block(bb_else);
            b.add_inst(InstKind::StoreProp {
                obj: Operand::Value(alloc),
                key: key_x,
                val: Operand::Const(Constant::Int32(1)),
                purity: Purity::Pure,
            });
            b.set_terminator(Terminator::Branch {
                target: bb_join,
                args: vec![],
            });
            b.switch_to_block(bb_join);
            b.set_terminator(Terminator::Return {
                val: Some(Operand::Value(alloc)),
            });
            b.finish();
        }
        // FuncId(1) f() = mk().x
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
                .add_inst(InstKind::LoadProp {
                    obj: Operand::Value(call_v),
                    key: key_x,
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
        pm.run_module(&mut module).unwrap();

        // Structural assertion: the Call must be retired and at least
        // one CondBranch must be present in f's CFG (i.e., cloning
        // happened). Resolution of the load to Const(1) is the job of
        // ForwardLoadsPass + branch-fold; out of scope here.
        let f_body = module.functions[1].lowered();
        for blk in &f_body.blocks {
            for &iid in &blk.instructions {
                assert!(
                    !matches!(
                        f_body.instructions[iid.0 as usize].kind,
                        InstKind::Call { .. }
                    ),
                    "Call should be retired"
                );
            }
        }
    }
}
