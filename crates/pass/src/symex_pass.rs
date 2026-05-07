//! Symbolic-execution-driven loop collapse. Issue #53.
//!
//! Builds on the `symex` framework (issue #50): for each function we
//! identify natural loops whose entire summary is encodable as integer
//! affine recurrences with an integer-comparison exit condition, ask
//! Z3 for the unique iteration count `n` that makes the exit condition
//! true, and — when the model is unique — rewrite the pre-header to
//! branch directly to the exit target with the resolved Const args.
//! The now-unreachable header / body / latch are cleaned up by the
//! existing DCE infrastructure on subsequent pipeline iterations.
//!
//! ## Eligibility (v1, intentionally narrow)
//!
//! A loop is symex-collapsible iff:
//! - It is a natural loop with a single header, a single pre-header
//!   (one outside-the-loop predecessor of the header), and a single
//!   latch (one back-edge predecessor of the header).
//! - The header's terminator is a `CondBranch` whose two successors
//!   split the loop's exit: exactly one in-loop, one out-of-loop. The
//!   out-of-loop successor is the loop's "exit target". No other block
//!   in the loop body has a successor outside the loop.
//! - Every loop-body block (header + body blocks) is a `Branch` or
//!   `CondBranch`; no Switch / Try / Throw / Return inside the loop.
//! - Every instruction inside the loop body has effect ⊆
//!   `PURE | MAY_THROW`. No heap reads/writes, no calls, no allocations
//!   or coercions other than the ones modeled by `eval_inst`.
//! - Every instruction's `InstKind` is one the `symex` walker can model
//!   precisely (`AddInt32`, `SubInt32`, `MulInt32`, `Eq`, `StrictEq`,
//!   `Lt`, `Le`, `Not`).
//! - For every header block-param `c_i`, the latch's edge-arg `next_i`
//!   has an affine shape: `c_i + Const(k)`, `c_i - Const(k)`, or
//!   `Const(k) + c_i`. (Pure-step recurrences. We refuse mul-step
//!   recurrences in v1: closed-forms over `c * k^n` need a separate
//!   Z3 encoding and the demo cases don't need them.)
//! - The pre-header's edge-arg `init_i` for each loop-carried param is
//!   either a compile-time `Const` or a Value defined outside the
//!   loop. We only need its symbolic value to be expressible to Z3 —
//!   we encode it as a fresh BV when it isn't a Const, then read its
//!   value from the model.
//! - The header's CondBranch condition is the result Value of one of
//!   `StrictEq{c,Const}`, `Eq{c,Const}`, `Lt{c,Const}`, `Le{c,Const}`,
//!   where `c` is a header block-param. We also handle a `Not` of any
//!   of those. The comparison's other operand may be a Const or any
//!   outside-the-loop Value.
//! - The header's exit-edge args reference only Constants, header
//!   block-params (which we have closed forms for), or outside-the-loop
//!   Values (which the pre-header already sees as the same Value).
//!
//! On any failed check the pass leaves the loop unchanged.
//!
//! ## Z3 query shape
//!
//! Per loop: introduce one BV variable `n` (BV_WIDTH bits, ≥ 0). For
//! each header block-param `c_i` define
//!   `c_i_at(n) := init_i + n * step_i`
//! as a Z3 BV. Build the exit-condition Bool from those closed forms,
//! and ask the solver for `n` such that the condition is true AND
//! `n >= 0`. Then ask "is there a different `n'` that also satisfies?"
//! by asserting the original solution holds and adding `n != n_solved`
//! plus the same condition; if SAT, bail (non-unique).
//!
//! On a unique solution, compute each loop-carried param's value at
//! `n_solved` (`init_i + n_solved * step_i`). Substitute those into
//! the header's exit-edge arg list and rewrite the pre-header's
//! `Branch(header, _)` into `Branch(exit_target, [resolved...])`.
//!
//! ## What this pass deliberately does NOT do
//!
//! - Heap modeling. No way to crack the rotation-IIFE on real obf
//!   samples without it; that's a separate ticket.
//! - Multi-exit loops, nested loops, irreducible CFGs.
//! - Path merging or summarisation across non-affine recurrences.
//! - Floating-point / BigInt / String reasoning.
//! - Mutating block layout. We rewrite ONE terminator (the
//!   pre-header's); orphaned blocks are left for DCE to clean up on
//!   subsequent pipeline iterations.

#![cfg(feature = "smt")]

use std::collections::{HashMap, HashSet};

use ir::error::Error;
use ir::ssa::{
    BlockId, Constant, Effect, FuncId, InstKind, LoweredBody, Module, Operand, Terminator, Value,
    ValueDefKind,
};

use z3::{
    SatResult, Solver,
    ast::{BV, Bool},
};

use crate::FunctionPass;
use crate::symex::BV_WIDTH;

pub struct SymExPass;

impl FunctionPass for SymExPass {
    fn name(&self) -> &'static str {
        "symex"
    }

    fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<bool, Error> {
        let func = &mut module.functions[fid.0 as usize];
        if func.is_pass_through() {
            return Ok(false);
        }
        let body = func.lowered_mut();
        if body.blocks.is_empty() {
            return Ok(false);
        }

        let dom = compute_dominators(body);
        let loops = find_collapsible_loops(body, &dom);

        let mut changed = false;
        for lp in loops {
            if try_collapse(body, &lp) {
                changed = true;
            }
        }
        if changed {
            body.rebuild_predecessors();
        }
        Ok(changed)
    }
}

// ============================================================================
// Loop discovery — same shape as licm.rs, narrowed to single-latch loops.
// ============================================================================

#[derive(Debug)]
struct LoopShape {
    header: BlockId,
    pre_header: BlockId,
    latch: BlockId,
    /// All loop-body blocks INCLUDING the header.
    body: HashSet<u32>,
    /// The block outside the loop reached via the header's CondBranch
    /// exit edge.
    exit_target: BlockId,
}

fn compute_dominators(body: &LoweredBody) -> Vec<Option<HashSet<u32>>> {
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
    let max_iters = (n.saturating_mul(n)).saturating_add(64);
    let mut iters = 0usize;
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

fn dominates(dom: &[Option<HashSet<u32>>], a: BlockId, b: BlockId) -> bool {
    match dom.get(b.0 as usize).and_then(|s| s.as_ref()) {
        Some(s) => s.contains(&a.0),
        None => false,
    }
}

fn find_collapsible_loops(
    body: &LoweredBody,
    dom: &[Option<HashSet<u32>>],
) -> Vec<LoopShape> {
    let n = body.blocks.len();
    // Group back-edges by header.
    let mut header_latches: HashMap<u32, Vec<BlockId>> = HashMap::new();
    for bi in 0..n {
        let from = BlockId(bi as u32);
        for (succ, _) in body.blocks[bi].terminator.successors() {
            if dominates(dom, succ, from) {
                header_latches.entry(succ.0).or_default().push(from);
            }
        }
    }

    let mut loops: Vec<LoopShape> = Vec::new();
    for (h_idx, latches) in header_latches {
        let header = BlockId(h_idx);
        if latches.len() != 1 {
            // Multi-latch: bail per v1 scope (single back-edge only).
            continue;
        }
        let latch = latches[0];

        // Body: reverse-BFS from latch bounded by header.
        let mut body_set: HashSet<u32> = HashSet::new();
        body_set.insert(h_idx);
        body_set.insert(latch.0);
        let mut work: Vec<BlockId> = vec![latch];
        while let Some(b) = work.pop() {
            if b == header {
                continue;
            }
            for &p in &body.blocks[b.0 as usize].predecessors {
                if body_set.insert(p.0) {
                    work.push(p);
                }
            }
        }

        // Reject loops that contain a nested loop header. A nested
        // loop would be detected as a body block (other than the
        // outer header AND other than the latch's edge to the outer
        // header) that itself heads a back-edge inside the body.
        let mut has_nested = false;
        for bi in &body_set {
            let b = BlockId(*bi);
            if b == header {
                continue;
            }
            for (succ, _) in body.blocks[*bi as usize].terminator.successors() {
                // The latch's back-edge to `header` is the outer
                // loop's own back-edge; skip it.
                if succ == header {
                    continue;
                }
                if body_set.contains(&succ.0) && dominates(dom, succ, b) {
                    has_nested = true;
                    break;
                }
            }
            if has_nested {
                break;
            }
        }
        if has_nested {
            continue;
        }

        // Pre-header: unique non-body header predecessor.
        let mut outside_preds: Vec<BlockId> = Vec::new();
        for &p in &body.blocks[h_idx as usize].predecessors {
            if !body_set.contains(&p.0) {
                outside_preds.push(p);
            }
        }
        if outside_preds.len() != 1 {
            continue;
        }
        let pre_header = outside_preds[0];

        // Header must be a CondBranch with one in-loop and one out-of-loop edge.
        let exit_target = match &body.blocks[h_idx as usize].terminator {
            Terminator::CondBranch {
                then_b, else_b, ..
            } => {
                let t_in = body_set.contains(&then_b.0);
                let e_in = body_set.contains(&else_b.0);
                match (t_in, e_in) {
                    (true, false) => *else_b,
                    (false, true) => *then_b,
                    _ => continue,
                }
            }
            _ => continue,
        };

        // Reject if any non-header block in the loop has a successor
        // outside the loop body (multi-exit). Also reject if any block
        // uses Switch / Try / Return / Throw.
        let mut multi_exit_or_complex = false;
        for bi in &body_set {
            let term = &body.blocks[*bi as usize].terminator;
            match term {
                Terminator::Branch { .. } | Terminator::CondBranch { .. } => {}
                _ => {
                    multi_exit_or_complex = true;
                    break;
                }
            }
            if BlockId(*bi) != header {
                for (succ, _) in term.successors() {
                    if !body_set.contains(&succ.0) {
                        multi_exit_or_complex = true;
                        break;
                    }
                }
            }
            if multi_exit_or_complex {
                break;
            }
        }
        if multi_exit_or_complex {
            continue;
        }

        loops.push(LoopShape {
            header,
            pre_header,
            latch,
            body: body_set,
            exit_target,
        });
    }
    loops.sort_by_key(|l| l.header.0);
    loops
}

// ============================================================================
// Recurrence analysis
// ============================================================================

/// `c' = c + step` for some Int32 constant `step`. Negative steps
/// (subtraction) are encoded as a negative `step`.
struct Affine {
    step: i32,
}

/// Allowed instruction kinds inside the loop body. Anything else
/// causes a bail.
fn inst_modeled_by_symex(kind: &InstKind) -> bool {
    matches!(
        kind,
        InstKind::AddInt32 { .. }
            | InstKind::SubInt32 { .. }
            | InstKind::MulInt32 { .. }
            | InstKind::Eq { .. }
            | InstKind::StrictEq { .. }
            | InstKind::Lt { .. }
            | InstKind::Le { .. }
            | InstKind::Not { .. }
    )
}

/// True iff `v` is defined inside `lp.body` (an inst, or a block-param
/// of a body block).
fn value_defined_in_loop(body: &LoweredBody, v: Value, lp: &LoopShape) -> bool {
    let def = match body.values.get(v.0 as usize) {
        Some(d) => d,
        None => return false,
    };
    match &def.kind {
        ValueDefKind::Inst(iid) => {
            for bi in lp.body.iter().copied() {
                let blk = &body.blocks[bi as usize];
                if blk.instructions.iter().any(|i| i.0 == iid.0) {
                    return true;
                }
            }
            false
        }
        ValueDefKind::BlockParam { block, .. } => lp.body.contains(&block.0),
        ValueDefKind::Arg(_) | ValueDefKind::Removed => false,
    }
}

/// If `op` is a constant integer literal we can lift to BV, return it.
fn const_i32(op: &Operand) -> Option<i32> {
    if let Operand::Const(Constant::Int32(i)) = op {
        return Some(*i);
    }
    if let Operand::Const(Constant::Number(n)) = op
        && n.fract() == 0.0
        && n.is_finite()
    {
        let i = *n as i64;
        if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
            return Some(i as i32);
        }
    }
    None
}

/// Look up the inst kind for a Value, if it was produced by an inst.
fn inst_kind_for(body: &LoweredBody, v: Value) -> Option<&InstKind> {
    let def = body.values.get(v.0 as usize)?;
    if let ValueDefKind::Inst(iid) = def.kind {
        Some(&body.instructions[iid.0 as usize].kind)
    } else {
        None
    }
}

/// Try to express `next` as `header_param + Const`. Returns the constant
/// step. Handles `AddInt32(c, k)`, `AddInt32(k, c)`, `SubInt32(c, k)`.
fn affine_step(
    body: &LoweredBody,
    next: &Operand,
    header_param: Value,
) -> Option<Affine> {
    // Identity recurrence: next == c. Step = 0.
    if let Operand::Value(v) = next
        && *v == header_param
    {
        return Some(Affine { step: 0 });
    }
    let next_v = match next {
        Operand::Value(v) => *v,
        _ => return None,
    };
    let kind = inst_kind_for(body, next_v)?;
    match kind {
        InstKind::AddInt32 { lhs, rhs } => {
            if let (Operand::Value(v), k) = (lhs, rhs)
                && *v == header_param
                && let Some(k) = const_i32(k)
            {
                return Some(Affine { step: k });
            }
            if let (k, Operand::Value(v)) = (lhs, rhs)
                && *v == header_param
                && let Some(k) = const_i32(k)
            {
                return Some(Affine { step: k });
            }
            None
        }
        InstKind::SubInt32 { lhs, rhs } => {
            if let (Operand::Value(v), k) = (lhs, rhs)
                && *v == header_param
                && let Some(k) = const_i32(k)
            {
                return Some(Affine { step: k.wrapping_neg() });
            }
            None
        }
        _ => None,
    }
}

/// The cond Value should resolve to one of the modeled comparisons
/// against a header block-param. Returns the param + builder for the
/// Z3 Bool (true when "exit taken"), parameterised over the current
/// closed-form BV values.
struct ExitCondShape {
    /// Set of header block-params referenced by the exit condition.
    /// We need closed forms for at least these.
    used_params: Vec<Value>,
    /// Builder closure: given `params_at_n` (for each header block
    /// param, its closed-form BV), return a Z3 Bool that is true iff
    /// the exit branch is taken at iteration n.
    builder: Box<dyn Fn(&HashMap<u32, BV>) -> Option<Bool>>,
}

/// Build a Z3 BV expression for an arbitrary operand:
/// - constant integer → `BV::from_i64`
/// - header block-param → looked up in `params_at_n`
/// - any other Value → fresh free BV (we'll let Z3 quantify
///   universally; for this v1 we only call this on operands that are
///   either constants or header params, plus any outside-loop Values
///   we want to use the original Value of).
fn op_to_bv(
    op: &Operand,
    params_at_n: &HashMap<u32, BV>,
    outside: &HashMap<u32, BV>,
) -> Option<BV> {
    match op {
        Operand::Const(Constant::Int32(i)) => Some(BV::from_i64(*i as i64, BV_WIDTH)),
        Operand::Const(Constant::Number(n)) if n.fract() == 0.0 && n.is_finite() => {
            let i = *n as i64;
            if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
                Some(BV::from_i64(i, BV_WIDTH))
            } else {
                None
            }
        }
        Operand::Const(Constant::Bool(b)) => {
            Some(BV::from_i64(if *b { 1 } else { 0 }, BV_WIDTH))
        }
        Operand::Value(v) => {
            if let Some(bv) = params_at_n.get(&v.0) {
                Some(bv.clone())
            } else {
                outside.get(&v.0).cloned()
            }
        }
        _ => None,
    }
}

/// Check whether `cond_value` resolves to one of the modeled
/// comparisons whose lhs is a header block-param. If so, return an
/// ExitCondShape whose builder generates the "exit-taken" Bool.
fn analyze_exit_cond(
    body: &LoweredBody,
    cond: &Operand,
    header_params: &HashSet<u32>,
    cond_polarity_exit_when_true: bool,
) -> Option<ExitCondShape> {
    let v = match cond {
        Operand::Value(v) => *v,
        _ => return None,
    };
    let (kind, negated) = match inst_kind_for(body, v)? {
        InstKind::Not { operand: Operand::Value(inner) } => {
            (inst_kind_for(body, *inner)?, true)
        }
        k => (k, false),
    };

    // Polarity: exit_taken = (cond evaluated) XOR (NOT(then-edge-is-exit))?
    // We're given `cond_polarity_exit_when_true` meaning if cond is
    // true, the exit branch is taken. After accounting for the `Not`
    // wrapper (if any), the underlying comparison's exit polarity is:
    let exit_when_inner_true = cond_polarity_exit_when_true ^ negated;

    let (lhs, rhs, op_kind) = match kind {
        InstKind::StrictEq { lhs, rhs } | InstKind::Eq { lhs, rhs } => (lhs, rhs, "eq"),
        InstKind::Lt { lhs, rhs } => (lhs, rhs, "lt"),
        InstKind::Le { lhs, rhs } => (lhs, rhs, "le"),
        _ => return None,
    };

    // We require ONE side to be a header block-param. The other side
    // can be a const or any outside-loop Value.
    let (param_side, other_side, swap) = match (lhs, rhs) {
        (Operand::Value(lv), _) if header_params.contains(&lv.0) => (lhs, rhs, false),
        (_, Operand::Value(rv)) if header_params.contains(&rv.0) => (rhs, lhs, true),
        _ => return None,
    };
    let param_v = match param_side {
        Operand::Value(v) => *v,
        _ => return None,
    };

    let other = other_side.clone();
    let op_kind = op_kind.to_string();
    let used_params = vec![param_v];

    let builder: Box<dyn Fn(&HashMap<u32, BV>) -> Option<Bool>> =
        Box::new(move |params_at_n: &HashMap<u32, BV>| {
            let outside = HashMap::new();
            // Header param is always in params_at_n; outside operands
            // need to be passed in by the caller — for v1, we require
            // the comparison's "other" side to be a constant. This keeps
            // the closed-form Z3 query well-defined without dragging in
            // arbitrary symbolic free variables.
            let lhs_bv = params_at_n.get(&param_v.0)?.clone();
            let rhs_bv = op_to_bv(&other, params_at_n, &outside)?;
            let (a, b) = if swap { (rhs_bv, lhs_bv) } else { (lhs_bv, rhs_bv) };
            let inner_true: Bool = match op_kind.as_str() {
                "eq" => a.eq(&b),
                "lt" => a.bvslt(&b),
                "le" => a.bvsle(&b),
                _ => return None,
            };
            Some(if exit_when_inner_true {
                inner_true
            } else {
                inner_true.not()
            })
        });

    Some(ExitCondShape {
        used_params,
        builder,
    })
}

/// True iff every instruction in the loop body is symex-modeled and
/// effect ⊆ PURE | MAY_THROW.
fn body_is_clean(body: &LoweredBody, lp: &LoopShape) -> bool {
    let allowed = Effect::PURE | Effect::MAY_THROW;
    for bi in &lp.body {
        let blk = &body.blocks[*bi as usize];
        for &iid in &blk.instructions {
            let inst = &body.instructions[iid.0 as usize];
            let eff = inst.kind.effect();
            // Effect ⊆ allowed iff every bit of `eff` is in `allowed`,
            // equivalently `eff & !allowed == empty`.
            if !(eff - allowed).is_empty() {
                return false;
            }
            if !inst_modeled_by_symex(&inst.kind) {
                return false;
            }
        }
    }
    true
}

// ============================================================================
// Collapse — the main rewrite
// ============================================================================

fn try_collapse(body: &mut LoweredBody, lp: &LoopShape) -> bool {
    // Body cleanliness gate.
    if !body_is_clean(body, lp) {
        return false;
    }

    // Header block-params are the loop-carried values.
    let header_params: Vec<Value> =
        body.blocks[lp.header.0 as usize].params.clone();
    let header_param_set: HashSet<u32> =
        header_params.iter().map(|v| v.0).collect();
    if header_params.is_empty() {
        return false;
    }

    // Pre-header edge args: initial values for header params.
    let pre_header_args: Vec<Operand> = match body.blocks[lp.pre_header.0 as usize]
        .terminator
        .edge_args_to(lp.header)
    {
        Some(a) => a.to_vec(),
        None => return false,
    };
    if pre_header_args.len() != header_params.len() {
        return false;
    }

    // Latch edge args: recurrence values.
    let latch_args: Vec<Operand> = match body.blocks[lp.latch.0 as usize]
        .terminator
        .edge_args_to(lp.header)
    {
        Some(a) => a.to_vec(),
        None => return false,
    };
    if latch_args.len() != header_params.len() {
        return false;
    }

    // For each loop-carried param, derive an affine recurrence.
    let mut affines: Vec<Affine> = Vec::with_capacity(header_params.len());
    for (i, p) in header_params.iter().enumerate() {
        match affine_step(body, &latch_args[i], *p) {
            Some(a) => affines.push(a),
            None => return false,
        }
    }

    // Inits: every init must be either a Const i32 or a Value defined
    // outside the loop. Outside-defined non-const inits are valid for
    // computing the param's value at exit, but only if the resolved
    // value at exit happens to be expressible as a Const — i.e. its
    // step must be 0 AND we can evaluate the exit edge args without
    // it. Simplest v1 rule: require every init to be a compile-time
    // Const i32. Future work can relax this to outside-loop Values
    // for params that are never referenced by the exit-cond comparison.
    let mut inits: Vec<i32> = Vec::with_capacity(header_params.len());
    for op in &pre_header_args {
        match const_i32(op) {
            Some(k) => inits.push(k),
            None => return false,
        }
    }

    // Header CondBranch: figure out which edge is the exit edge and
    // build the Z3 exit-condition shape.
    let (cond_op, exit_when_true): (Operand, bool) =
        match &body.blocks[lp.header.0 as usize].terminator {
            Terminator::CondBranch {
                cond,
                then_b,
                else_b,
                ..
            } => {
                if *then_b == lp.exit_target {
                    (cond.clone(), true)
                } else if *else_b == lp.exit_target {
                    (cond.clone(), false)
                } else {
                    return false;
                }
            }
            _ => return false,
        };

    let shape = match analyze_exit_cond(body, &cond_op, &header_param_set, exit_when_true) {
        Some(s) => s,
        None => return false,
    };

    // Header's exit-edge args: must reference only constants, header
    // block-params, or outside-loop values. We'll resolve them after
    // Z3 gives us `n_solved`.
    let exit_edge_args: Vec<Operand> = match &body.blocks[lp.header.0 as usize].terminator {
        Terminator::CondBranch {
            then_b,
            then_args,
            else_b,
            else_args,
            ..
        } => {
            if *then_b == lp.exit_target {
                then_args.clone()
            } else if *else_b == lp.exit_target {
                else_args.clone()
            } else {
                return false;
            }
        }
        _ => return false,
    };
    for op in &exit_edge_args {
        if let Operand::Value(v) = op
            && header_param_set.get(&v.0).is_none()
            && value_defined_in_loop(body, *v, lp)
        {
            // References a loop-internal value that isn't a header
            // param — we can't resolve it.
            return false;
        }
    }

    // ============================================================
    // Z3 query: find the smallest non-negative n at which the exit
    // condition holds; assert uniqueness.
    // ============================================================
    let n = BV::new_const("symex_n", BV_WIDTH);
    let zero = BV::from_i64(0, BV_WIDTH);
    let solver = Solver::new();
    solver.assert(&n.bvsge(&zero));

    // Build closed forms c_i_at(n) = init_i + n * step_i.
    let mut params_at_n: HashMap<u32, BV> = HashMap::new();
    for (i, p) in header_params.iter().enumerate() {
        let init_bv = BV::from_i64(inits[i] as i64, BV_WIDTH);
        let step_bv = BV::from_i64(affines[i].step as i64, BV_WIDTH);
        let val = init_bv.bvadd(&n.bvmul(&step_bv));
        params_at_n.insert(p.0, val);
    }

    // Sanity: cond shape only consults `used_params`; ensure each is
    // a header param we've encoded.
    for v in &shape.used_params {
        if !params_at_n.contains_key(&v.0) {
            return false;
        }
    }

    let exit_bool = match (shape.builder)(&params_at_n) {
        Some(b) => b,
        None => return false,
    };
    solver.assert(&exit_bool);

    if solver.check() != SatResult::Sat {
        return false;
    }
    let model = match solver.get_model() {
        Some(m) => m,
        None => return false,
    };
    let n_solved = match model.eval(&n, true).and_then(|v| v.as_i64()) {
        Some(i) => i as i32,
        None => return false,
    };

    // Uniqueness check: forbid this `n` and re-ask.
    let n_solved_bv = BV::from_i64(n_solved as i64, BV_WIDTH);
    solver.assert(&n.eq(&n_solved_bv).not());
    if solver.check() == SatResult::Sat {
        // Multiple valid n values — bail. (Loops with `<` exit
        // conditions hit this: many n satisfy `c < limit`.)
        return false;
    }

    // ============================================================
    // Resolve exit-edge args using the model.
    // ============================================================
    let mut resolved_args: Vec<Operand> = Vec::with_capacity(exit_edge_args.len());
    for op in &exit_edge_args {
        match op {
            Operand::Const(_) => resolved_args.push(op.clone()),
            Operand::Capture(_) => resolved_args.push(op.clone()),
            Operand::Value(v) => {
                if let Some(bv) = params_at_n.get(&v.0) {
                    let val = match model.eval(bv, true).and_then(|x| x.as_i64()) {
                        Some(i) => i as i32,
                        None => return false,
                    };
                    resolved_args.push(Operand::Const(Constant::Int32(val)));
                } else if !value_defined_in_loop(body, *v, lp) {
                    // Outside-loop Value: pre-header sees it too,
                    // pass through unchanged.
                    resolved_args.push(op.clone());
                } else {
                    return false;
                }
            }
        }
    }

    // ============================================================
    // Rewrite the pre-header's terminator to skip the loop entirely.
    // ============================================================
    let pre_h_term = &mut body.blocks[lp.pre_header.0 as usize].terminator;
    match pre_h_term {
        Terminator::Branch { target, args } if *target == lp.header => {
            *target = lp.exit_target;
            *args = resolved_args;
        }
        _ => {
            // The pre-header is a non-trivial terminator (CondBranch
            // with one edge to the header). Splitting that edge would
            // require manufacturing a new block; v1 declines — bail.
            return false;
        }
    }
    true
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PassManager;
    use ir::ssa::{Constant, FunctionBuilder, InstKind, Module, Operand, Purity, Terminator};

    /// Counter-loop SSA shape:
    ///
    ///   entry:        Branch header(0)
    ///   header(c):    cmp = StrictEq(c, 7)
    ///                 CondBranch cmp -> exit(c), body
    ///   body:         next = AddInt32(c, 1)
    ///                 Branch header(next)
    ///   exit(r):      Return r
    ///
    /// SymExPass should rewrite entry's terminator to
    /// `Branch exit(7)`, making header/body unreachable. We verify by
    /// checking that entry now points at exit and the edge arg is
    /// `Const(Int32(7))`.
    #[test]
    fn counter_loop_collapses_to_const_exit() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let entry = b.new_block();
        let header = b.new_block();
        let body_b = b.new_block();
        let exit_b = b.new_block();
        let counter = b.add_block_param(header);
        let result = b.add_block_param(exit_b);

        b.switch_to_block(entry);
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Const(Constant::Int32(0))],
        });

        b.switch_to_block(header);
        let cmp = b
            .add_inst(InstKind::StrictEq {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(7)),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cmp),
            then_b: exit_b,
            then_args: vec![Operand::Value(counter)],
            else_b: body_b,
            else_args: vec![],
        });

        b.switch_to_block(body_b);
        let next = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(1)),
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Value(next)],
        });

        b.switch_to_block(exit_b);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(result)),
        });
        let fid = b.finish();
        module.functions[fid.0 as usize].lowered_mut().entry = entry;
        module.functions[fid.0 as usize]
            .lowered_mut()
            .rebuild_predecessors();

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(SymExPass));
        pm.run(&mut module, fid).unwrap();

        let lowered = module.functions[fid.0 as usize].lowered();
        match &lowered.blocks[entry.0 as usize].terminator {
            Terminator::Branch { target, args } => {
                assert_eq!(*target, exit_b, "entry must branch to exit");
                assert_eq!(args.len(), 1);
                assert!(
                    matches!(args[0], Operand::Const(Constant::Int32(7))),
                    "entry edge arg must be Const(Int32(7)), got {:?}",
                    args[0]
                );
            }
            other => panic!("expected Branch, got {other:?}"),
        }
        module.validate().expect("post-symex IR must validate");
    }

    /// Counter loop with `Lt` exit condition has many satisfying n
    /// values — uniqueness check must bail. Pass leaves IR unchanged.
    #[test]
    fn lt_exit_condition_is_not_unique_and_bails() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let entry = b.new_block();
        let header = b.new_block();
        let body_b = b.new_block();
        let exit_b = b.new_block();
        let counter = b.add_block_param(header);
        let _result = b.add_block_param(exit_b);

        b.switch_to_block(entry);
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Const(Constant::Int32(0))],
        });

        b.switch_to_block(header);
        // `c < 10`: many `c` satisfy this, so n is not unique.
        // Note: With CondBranch, exit_when_true means "exit when c<10".
        // Since c starts at 0 and increases, the condition is true at
        // the start; we want exit_when_false. Wire the comparison so
        // that exit happens when c == 10 fails — i.e. while c < 10
        // stay in loop. This makes `exit-taken` correspond to
        // `not (c < 10)` = `c >= 10`. Many c values satisfy that;
        // n=10 is the smallest but n=11,12,... also satisfy.
        let cmp = b
            .add_inst(InstKind::Lt {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(10)),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cmp),
            then_b: body_b,
            then_args: vec![],
            else_b: exit_b,
            else_args: vec![Operand::Value(counter)],
        });

        b.switch_to_block(body_b);
        let next = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(1)),
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Value(next)],
        });

        b.switch_to_block(exit_b);
        b.set_terminator(Terminator::Return { val: None });
        let fid = b.finish();
        module.functions[fid.0 as usize].lowered_mut().entry = entry;
        module.functions[fid.0 as usize]
            .lowered_mut()
            .rebuild_predecessors();

        // Snapshot entry's terminator before the pass.
        let before = format!(
            "{:?}",
            module.functions[fid.0 as usize].lowered().blocks[entry.0 as usize].terminator
        );

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(SymExPass));
        pm.run(&mut module, fid).unwrap();

        let after = format!(
            "{:?}",
            module.functions[fid.0 as usize].lowered().blocks[entry.0 as usize].terminator
        );
        assert_eq!(before, after, "non-unique exit must leave IR unchanged");
    }

    /// Heap-effect inside the loop body: pass must bail (eligibility
    /// requires effect ⊆ PURE | MAY_THROW).
    #[test]
    fn heap_inst_in_loop_disqualifies_collapse() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let entry = b.new_block();
        let header = b.new_block();
        let body_b = b.new_block();
        let exit_b = b.new_block();
        let counter = b.add_block_param(header);
        let _result = b.add_block_param(exit_b);
        let arg = b.body().params[0];

        b.switch_to_block(entry);
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Const(Constant::Int32(0))],
        });

        b.switch_to_block(header);
        let cmp = b
            .add_inst(InstKind::StrictEq {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(7)),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cmp),
            then_b: exit_b,
            then_args: vec![Operand::Value(counter)],
            else_b: body_b,
            else_args: vec![],
        });

        b.switch_to_block(body_b);
        // Heap LoadProp inside body — disqualifies the loop.
        let key = b.intern("x");
        b.add_inst(InstKind::LoadProp {
            obj: Operand::Value(arg),
            key,
            purity: Purity::Throws,
        });
        let next = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(1)),
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Value(next)],
        });

        b.switch_to_block(exit_b);
        b.set_terminator(Terminator::Return { val: None });
        let fid = b.finish();
        module.functions[fid.0 as usize].lowered_mut().entry = entry;
        module.functions[fid.0 as usize]
            .lowered_mut()
            .rebuild_predecessors();

        let before = format!(
            "{:?}",
            module.functions[fid.0 as usize].lowered().blocks[entry.0 as usize].terminator
        );

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(SymExPass));
        pm.run(&mut module, fid).unwrap();

        let after = format!(
            "{:?}",
            module.functions[fid.0 as usize].lowered().blocks[entry.0 as usize].terminator
        );
        assert_eq!(before, after, "heap-bearing loop body must not collapse");
    }

    /// Multiplicative recurrence (`c' = c * 2`) is non-affine in v1
    /// scope and must be left alone.
    #[test]
    fn multiplicative_recurrence_bails() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let entry = b.new_block();
        let header = b.new_block();
        let body_b = b.new_block();
        let exit_b = b.new_block();
        let counter = b.add_block_param(header);
        let _result = b.add_block_param(exit_b);

        b.switch_to_block(entry);
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Const(Constant::Int32(1))],
        });

        b.switch_to_block(header);
        let cmp = b
            .add_inst(InstKind::StrictEq {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(64)),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cmp),
            then_b: exit_b,
            then_args: vec![Operand::Value(counter)],
            else_b: body_b,
            else_args: vec![],
        });

        b.switch_to_block(body_b);
        let next = b
            .add_inst(InstKind::MulInt32 {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Value(next)],
        });

        b.switch_to_block(exit_b);
        b.set_terminator(Terminator::Return { val: None });
        let fid = b.finish();
        module.functions[fid.0 as usize].lowered_mut().entry = entry;
        module.functions[fid.0 as usize]
            .lowered_mut()
            .rebuild_predecessors();

        let before = format!(
            "{:?}",
            module.functions[fid.0 as usize].lowered().blocks[entry.0 as usize].terminator
        );

        let mut pm = PassManager::new();
        pm.add_pass(Box::new(SymExPass));
        pm.run(&mut module, fid).unwrap();

        let after = format!(
            "{:?}",
            module.functions[fid.0 as usize].lowered().blocks[entry.0 as usize].terminator
        );
        assert_eq!(before, after, "non-affine recurrence must not collapse");
    }
}
