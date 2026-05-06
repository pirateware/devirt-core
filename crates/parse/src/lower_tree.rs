//! SSA-IR → tree-IR conversion (out-of-SSA).
//!
//! Two conceptual phases performed simultaneously during the structure-trace
//! walk:
//!
//! 1. **Eliminate block params.** Block params have no syntactic equivalent in
//!    JS, so each is replaced by a fresh tree-IR variable (`_t0`, `_t1`, …).
//!    For every `Branch { target, args }` whose target carries params, we
//!    emit `name(target.params[i]) = expr(args[i])` immediately before
//!    transferring control to `target` in the tree-IR. Conceptually this is
//!    the "always-insert-a-copy-block" simplification of standard out-of-SSA
//!    — it avoids the swap problem because each predecessor's copies live in
//!    a syntactically separate location.
//!
//! 2. **Reconstruct structured control flow.** We walk the CFG and recognize
//!    the specific shapes that the upstream SSA converter produces:
//!     - linear chains (single-successor `Branch`),
//!     - if/else diamonds (cbr to two blocks meeting at a common
//!       post-dominator),
//!     - while loops (cbr where one branch reaches back to the cbr's block,
//!       the other exits).
//!
//! Anything else panics with `unsupported CFG shape`. This is deliberate: the
//! converter only emits these shapes today; future passes that produce
//! arbitrary CFGs will need a real relooper, which is a later milestone.
//!
//! What survives the round-trip:
//!   - **Behavior** — the only invariant that matters; verified by the
//!     roundtrip tests at the bottom of this file.
//!   - **Original variable names** — NOT preserved. SSA conversion already
//!     erased them; this pass allocates fresh `_a0`/`_t0` names.
//!   - **`const` vs `let`** — NOT preserved; everything emerges as `let`.
//!   - **Block-shape exactness** — mostly preserved by the structure-trace
//!     heuristics; small differences (extra temporaries, redundant blocks)
//!     are fine.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use ir::ssa::{
    self, BlockId, Constant, FuncId, Function, InstId, InstKind, Operand, Terminator, Value,
    ValueDefKind,
};
use ir::tree::{self, BinOp, Expr, IrFunction, Item, Stmt, UnOp};

// ============================================================================
// Public entry points
// ============================================================================

pub fn lower_module(sm: &ssa::Module) -> tree::Module {
    lower_module_with_config(sm, &EmitConfig::default())
}

/// Lowering walks structured CFG via recursion proportional to the deepest
/// nested conditional in the source. Heavily-obfuscated minified inputs
/// (e.g. tier-3 lodash bundles) routinely nest 10⁴–10⁵ levels and overflow
/// the default 2–8 MB thread stack. Run lowering on a dedicated 64 MB
/// stack so real-world inputs don't crash the host. Any deeper input
/// indicates a genuine bug — fail loud rather than silently grow.
const LOWER_STACK_SIZE: usize = 64 * 1024 * 1024;

pub fn lower_module_with_config(sm: &ssa::Module, cfg: &EmitConfig) -> tree::Module {
    std::thread::scope(|s| {
        let handle = std::thread::Builder::new()
            .stack_size(LOWER_STACK_SIZE)
            .spawn_scoped(s, || lower_module_inner(sm, cfg))
            .expect("failed to spawn lower_tree worker thread");
        match handle.join() {
            Ok(m) => m,
            // Propagate the original panic so `#[should_panic(expected = ...)]`
            // and any caller-side panic-message inspection still see the
            // unwrapped payload.
            Err(payload) => std::panic::resume_unwind(payload),
        }
    })
}

fn lower_module_inner(sm: &ssa::Module, cfg: &EmitConfig) -> tree::Module {
    // Functions that are the target of an `AllocClosure` are NOT
    // top-level — they're emitted inline as `Expr::Func` / `Stmt::FuncDecl`
    // by their containing function's lowering. Skip them here so the
    // module doesn't double-emit.
    let inner_targets = collect_inner_targets(sm);

    let lower_one = |fid: FuncId, sm: &ssa::Module, cfg: &EmitConfig| -> Option<Item> {
        if inner_targets.contains(&fid) {
            return None;
        }
        let func = &sm.functions[fid.0 as usize];
        Some(match &func.body {
            ssa::FunctionBody::Lowered(_) => {
                let mut local_cfg = cfg.clone();
                for &vi in &func.captured_by_passthrough {
                    let def = &func.lowered().values[vi as usize];
                    let strid = def.source_name.expect(
                        "captured-by-passthrough Value missing source_name — \
                         should have been caught at FunctionBuilder::finish()",
                    );
                    let name = sm.strings.resolve(&strid).to_string();
                    local_cfg.immovable_names.insert(vi, name);
                }
                Item::Function(lower_function(sm, fid, func, &local_cfg))
            }
            ssa::FunctionBody::PassThrough(pt) => {
                Item::PassThroughFunction(passthrough_to_literal(sm, func, pt))
            }
        })
    };

    // When the caller populated `top_level_layout` (the OXC frontend
    // does this for every program), walk it in source order so
    // top-level passthrough statements interleave correctly with
    // functions. Otherwise (hand-built tests that construct
    // `ssa::Module` directly) fall back to functions-only emission
    // in FuncId order, which preserves the historical behavior.
    let items = if !sm.top_level_layout.is_empty() {
        let mut items = Vec::with_capacity(sm.top_level_layout.len());
        for entry in &sm.top_level_layout {
            match entry {
                ssa::TopLevelEntry::Function(fid) => {
                    if let Some(item) = lower_one(*fid, sm, cfg) {
                        items.push(item);
                    }
                }
                ssa::TopLevelEntry::PassThroughStmt(src) => {
                    items.push(Item::PassThroughStmt(src.clone()));
                }
            }
        }
        items
    } else {
        let mut items = Vec::with_capacity(sm.functions.len());
        for i in 0..sm.functions.len() {
            if let Some(item) = lower_one(FuncId(i as u32), sm, cfg) {
                items.push(item);
            }
        }
        items
    };

    tree::Module { items }
}

/// Collect every FuncId that appears as the target of an `AllocClosure`
/// anywhere in the module. These are the inner literals; the outer's
/// `lower_function` resugars them inline, so they must not be emitted
/// at module top-level.
fn collect_inner_targets(sm: &ssa::Module) -> HashSet<FuncId> {
    let mut targets = HashSet::new();
    for f in &sm.functions {
        let body = match &f.body {
            ssa::FunctionBody::Lowered(b) => b,
            ssa::FunctionBody::PassThrough(_) => continue,
        };
        for inst in &body.instructions {
            if let InstKind::AllocClosure { func, .. } = &inst.kind {
                targets.insert(*func);
            }
        }
    }
    targets
}

fn passthrough_to_literal(
    sm: &ssa::Module,
    func: &Function,
    pt: &ssa::PassThroughBody,
) -> tree::FuncLiteral {
    // Top-level functions always have a name (SSA `Function.name` is
    // populated for every top-level item). The SSA-side
    // `PassThroughBody.function_name` is `Option` because it's also
    // used for inner anonymous function expressions; for the top-level
    // mapping use `func.name` so the emit `Item::PassThroughFunction`
    // arm always finds a name. Empty string sentinel (unnamed inner
    // arrow that ended up as a top-level item via fallback) maps to
    // None to preserve the existing emit invariant.
    let name = pt
        .function_name
        .map(|n| sm.strings.resolve(&n).to_string())
        .or_else(|| {
            if func.name.is_empty() {
                None
            } else {
                Some(func.name.clone())
            }
        });
    tree::FuncLiteral {
        name,
        params: pt
            .param_names
            .iter()
            .map(|n| sm.strings.resolve(n).to_string())
            .collect(),
        is_async: pt.is_async,
        is_generator: pt.is_generator,
        is_arrow: pt.is_arrow,
        body: tree::FuncBody::PassThrough(pt.source.clone()),
        func_id: std::cell::Cell::new(None),
        passthrough_source: Some(pt.source.clone()),
        force_passthrough: std::cell::Cell::new(false),
        blockers: std::collections::BTreeSet::new(),
    }
    .into_with_func_name_check(&func.name)
}

trait FuncLiteralCheckExt {
    fn into_with_func_name_check(self, expected: &str) -> Self;
}
impl FuncLiteralCheckExt for tree::FuncLiteral {
    fn into_with_func_name_check(self, expected: &str) -> Self {
        debug_assert!(
            self.name.as_deref().unwrap_or(expected) == expected,
            "PassThrough function name mismatch: tree={:?} ssa={:?}",
            self.name,
            expected
        );
        self
    }
}

/// Configuration for the out-of-SSA renamer.
///
/// `immovable_names` pins specific Values to specific names — typically
/// because some downstream emit (a PassThrough closure body, a debug
/// pin) lexically references the name and cannot tolerate a
/// suffix-renamed substitute. The pinned name is reserved: other Values
/// that prefer it via `source_name` get suffix-disambiguated, and the
/// suffix-collision path skips the pinned name when generating
/// candidates. Pinning two distinct Values to the same name is a caller
/// bug and panics.
///
/// Implementation note: the ticket spec described this as
/// `HashSet<String>`, but a per-Value mapping is required to express
/// "this specific Value owns this name" — without it, the conflict
/// (test 8) and route-around (test 7) scenarios are indistinguishable
/// from the renamer's perspective.
#[derive(Default, Debug, Clone)]
pub struct EmitConfig {
    pub immovable_names: HashMap<u32, String>,
}

fn lower_function(sm: &ssa::Module, _id: FuncId, func: &Function, cfg: &EmitConfig) -> IrFunction {
    let mut ctx = LowerCtx::new(sm, func, cfg, Vec::new());
    let param_names: Vec<String> = func
        .lowered()
        .params
        .iter()
        .map(|v| ctx.name_for(*v).to_string())
        .collect();

    let mut body = Vec::new();
    // Hoist all temporary declarations to the top of the function. Hoisting
    // is the simplest way to ensure that names assigned inside if/while
    // bodies are visible in subsequent code: a `let` declared inside a JS
    // block scope is invisible after the block. Cosmetic ugliness; later
    // passes can sink lets back to first-use.
    let hoisted: Vec<String> = ctx.hoisted_names_in_decl_order();
    let already_declared: std::collections::HashSet<String> =
        hoisted.iter().cloned().collect();
    for name in hoisted {
        body.push(Stmt::Let { name, init: None });
    }

    // Forced let-declarations: names referenced lexically by some
    // nested PassThrough body, declared via `Stmt::Let` in this
    // function. The synthesized `StoreProp(GlobalThis, name, val)`
    // emits the assignment; without the explicit `let` here, the
    // assignment would silently target the global. See
    // `Function.forced_let_decls` doc.
    //
    // Skip names that already have an `Stmt::Let` from
    // `hoisted_names_in_decl_order` (e.g. the source-level `var x;`
    // declaration of an `Expr::Assign`-targeted name) — re-declaring
    // a let in the same block is a JS SyntaxError.
    for &sid in &func.forced_let_decls {
        let n = ctx.sm.strings.resolve(&sid).to_string();
        if already_declared.contains(&n) {
            continue;
        }
        body.push(Stmt::Let {
            name: n,
            init: None,
        });
    }

    ctx.walk(func.lowered().entry, None, &mut body);

    IrFunction {
        name: func.name.clone(),
        params: param_names,
        body,
        // Source slice is meaningless for tree-IR reconstructed from
        // SSA — the SSA round-trip is lossy and the original slice no
        // longer corresponds to `body`.
        source: None,
    }
}

// ============================================================================
// LowerCtx — per-function state for the walk
// ============================================================================

struct LowerCtx<'a> {
    sm: &'a ssa::Module,
    func: &'a Function,

    /// Per-Value name. Indexed by `Value.0`. `Removed` slots are skipped.
    value_names: HashMap<u32, String>,
    /// Names to hoist at function-body top, in allocation order.
    hoisted: Vec<String>,
    /// Cached forward-reachability per block (for join detection).
    reachable_cache: HashMap<BlockId, HashSet<BlockId>>,
    /// Cached "is loop header" decisions.
    loop_info_cache: HashMap<BlockId, Option<LoopInfo>>,
    /// Cached domination relation.
    dom_cache: HashMap<(BlockId, BlockId), bool>,
    /// Capture-name overrides for `Operand::Capture(idx)`. Empty for
    /// top-level functions; populated when `AllocClosure` recursively
    /// lowers an inner Lowered body — each entry is the source-level
    /// expression text the outer scope uses for that capture slot.
    capture_names: Vec<String>,

    // ----- runaway-recursion diagnostics --------------------------------
    // Active call-stack of `walk` invocations and per-block visit counter.
    // Three failure modes we want to distinguish:
    //   1. true deep nesting — `walk_stack.len()` huge, all entries distinct
    //   2. cycle / re-traversal — same BlockId appears at multiple stack depths
    //   3. exponential re-traversal without cycles — total `visit_counts` blow
    //      up while `walk_stack` stays modest (missing memoization)
    walk_stack: Vec<BlockId>,
    visit_counts: HashMap<BlockId, u64>,
    walk_total_visits: u64,

    /// Loop headers whose bodies are currently being emitted. Innermost
    /// last. When `walk` re-enters one of these (e.g. from a back-edge
    /// nested inside an if/else inside the body), the header has already
    /// been emitted as a `Stmt::While`; the right thing is to emit
    /// `continue` and return rather than recursively re-emitting the
    /// loop. Without this, `pass_through_stop` from a sibling diamond
    /// shadows the loop's header-stop and walk recurses through the
    /// back-edge forever.
    loop_headers_open: Vec<BlockId>,
    /// Active enclosing switch join blocks. When the walker encounters a
    /// `Branch { target }` where `target` matches the top of this stack,
    /// the branch encodes `break;` from the surrounding source-level
    /// switch — emitted as `Stmt::Break { label: None }` and walking
    /// stops. Mirrors `loop_headers_open`'s role for `Stmt::Continue`.
    switch_break_targets: Vec<BlockId>,
}

const WALK_STACK_LIMIT: usize = 50_000;
const WALK_TOTAL_VISIT_LIMIT: u64 = 5_000_000;

#[derive(Clone, Copy, Debug)]
struct LoopInfo {
    body_b: BlockId,
    exit_b: BlockId,
}

/// Operands captured during `detect_forin_loop` and used by
/// `emit_for_in` to reconstruct `Stmt::ForIn { binding, obj, body }`.
#[derive(Clone, Debug)]
struct ForInPattern {
    obj: Operand,
    iter_op: Operand,
    init_iid: InstId,
    done_iid: InstId,
    next_iid: InstId,
    pre_header_b: BlockId,
}

impl<'a> LowerCtx<'a> {
    fn new(
        sm: &'a ssa::Module,
        func: &'a Function,
        cfg: &EmitConfig,
        capture_names: Vec<String>,
    ) -> Self {
        let (value_names, hoisted) = assign_names(sm, func, cfg);
        Self {
            sm,
            func,
            value_names,
            hoisted,
            reachable_cache: HashMap::new(),
            loop_info_cache: HashMap::new(),
            dom_cache: HashMap::new(),
            capture_names,
            walk_stack: Vec::new(),
            visit_counts: HashMap::new(),
            walk_total_visits: 0,
            loop_headers_open: Vec::new(),
            switch_break_targets: Vec::new(),
        }
    }

    fn diagnose_walk_runaway(&self, reason: &str) -> ! {
        let stack_len = self.walk_stack.len();
        let distinct: HashSet<BlockId> = self.walk_stack.iter().copied().collect();
        let distinct_len = distinct.len();
        let total_visits: u64 = self.walk_total_visits;

        // Repeats currently on the active call-stack — case 2 signal.
        let mut on_stack_counts: HashMap<BlockId, u32> = HashMap::new();
        for &b in &self.walk_stack {
            *on_stack_counts.entry(b).or_insert(0) += 1;
        }
        let mut repeats: Vec<(BlockId, u32)> = on_stack_counts
            .into_iter()
            .filter(|(_, n)| *n > 1)
            .collect();
        repeats.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let top_repeats: Vec<String> = repeats
            .iter()
            .take(10)
            .map(|(b, n)| format!("bb{}×{}", b.0, n))
            .collect();

        // Most-visited blocks across the whole walk — case 3 signal.
        let mut by_visits: Vec<(BlockId, u64)> =
            self.visit_counts.iter().map(|(b, n)| (*b, *n)).collect();
        by_visits.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        let top_visited: Vec<String> = by_visits
            .iter()
            .take(10)
            .map(|(b, n)| format!("bb{}={}", b.0, n))
            .collect();

        // CFG dump for the offending function: each block, terminator kind,
        // successors. Bounded to the visited blocks so output stays small.
        let mut visited_blocks: Vec<BlockId> = self.visit_counts.keys().copied().collect();
        visited_blocks.sort_by_key(|b| b.0);
        let mut cfg_dump = String::new();
        for b in &visited_blocks {
            let term = &self.func.lowered().blocks[b.0 as usize].terminator;
            let succs: Vec<String> = term
                .successors()
                .into_iter()
                .map(|(s, _)| format!("bb{}", s.0))
                .collect();
            let kind = match term {
                Terminator::Branch { .. } => "Branch",
                Terminator::CondBranch { .. } => "CondBranch",
                Terminator::Switch { .. } => "Switch",
                Terminator::Return { .. } => "Return",
                Terminator::Throw { .. } => "Throw",
                Terminator::Try { .. } => "Try",
                Terminator::Unreachable => "Unreachable",
            };
            let preds: Vec<String> = self.func.lowered().blocks[b.0 as usize]
                .predecessors
                .iter()
                .map(|p| format!("bb{}", p.0))
                .collect();
            cfg_dump.push_str(&format!(
                "\n    bb{}: {} preds=[{}] -> [{}]",
                b.0,
                kind,
                preds.join(", "),
                succs.join(", ")
            ));
            // For CondBranches, show why loop_info decided what it did:
            // back-edge dominator-check + reachability of each successor
            // back to the candidate header.
            if matches!(term, Terminator::CondBranch { .. }) {
                let preds_list = self.func.lowered().blocks[b.0 as usize].predecessors.clone();
                let dominated_preds: Vec<String> = preds_list
                    .iter()
                    .filter_map(|&p| {
                        // Cheap hack: read cached dom result without &mut
                        // (compute_loop_info populated it for visited
                        // headers). If absent, mark "?" so we know.
                        match self.dom_cache.get(&(*b, p)).copied() {
                            Some(true) => Some(format!("bb{}=dom", p.0)),
                            Some(false) => Some(format!("bb{}=NOTdom", p.0)),
                            None => Some(format!("bb{}=uncached", p.0)),
                        }
                    })
                    .collect();
                cfg_dump.push_str(&format!(
                    "\n        cbr-dom-check: [{}]",
                    dominated_preds.join(", ")
                ));
            }
        }

        let case = if !repeats.is_empty() {
            "CASE 2 (cycle: BlockId repeats on active stack)"
        } else if total_visits > (distinct_len.saturating_mul(50)) as u64 && distinct_len > 0 {
            "CASE 3 (exponential re-traversal: total_visits >> distinct_blocks)"
        } else {
            "CASE 1 (true deep nesting)"
        };

        panic!(
            "lower_tree::walk runaway recursion ({reason})\n  \
             func = {:?} (entry bb{})\n  \
             likely: {case}\n  \
             active_stack_len = {stack_len}, distinct_on_stack = {distinct_len}\n  \
             total_visits = {total_visits}, distinct_blocks_visited = {}\n  \
             top_repeats_on_stack = [{}]\n  \
             top_visited_overall = [{}]\n  \
             cfg (visited blocks):{}",
            self.func.name,
            self.func.lowered().entry.0,
            self.visit_counts.len(),
            top_repeats.join(", "),
            top_visited.join(", "),
            cfg_dump,
        );
    }

    fn hoisted_names_in_decl_order(&self) -> Vec<String> {
        self.hoisted.clone()
    }

    fn name_for(&self, v: Value) -> &str {
        self.value_names
            .get(&v.0)
            .unwrap_or_else(|| panic!("no name allocated for value %{}", v.0))
    }

    /// If `bb`'s first instruction is `InstKind::CatchValue`, return
    /// the catch-param's source name (resolved through the result
    /// Value's hoisted name). Returns `None` if the catch has no
    /// binding (ES2019 `} catch { … }`) — represented by a CatchValue
    /// whose result is unused, or no leading CatchValue at all.
    fn first_inst_catch_param_name(&self, bb: BlockId) -> Option<String> {
        let blk = &self.func.lowered().blocks[bb.0 as usize];
        let first_iid = *blk.instructions.first()?;
        let kind = &self.func.lowered().instructions[first_iid.0 as usize].kind;
        if !matches!(kind, InstKind::CatchValue) {
            return None;
        }
        let v = find_value_for_inst(self.func, first_iid);
        // A `name_for` lookup panics if the name isn't allocated. For
        // ES2019 binding-less catch (where the construction site emits
        // CatchValue but the result is never bound to a source name),
        // the name will still be a synthesized hoisted name like
        // `__v3` — which is fine: that's the catch-param name we
        // emit. The renamer downstream can recover the source name
        // when the source had one (set via `set_source_name` at
        // construction time).
        Some(self.name_for(v).to_string())
    }

    fn operand_to_expr(&self, op: &Operand) -> Expr {
        match op {
            Operand::Value(v) => Expr::Ident(self.name_for(*v).to_string()),
            Operand::Const(c) => self.constant_to_expr(c),
            Operand::Capture(idx) => {
                // Lowered inner functions reference enclosing-scope values
                // via Operand::Capture(idx). The outer's `AllocClosure`
                // recursive lowering supplies a `capture_names` vector
                // — element `i` is the source-level expression text the
                // outer uses for that capture slot. When this LowerCtx
                // is the outermost (top-level) one, no captures are
                // populated; rendering a placeholder is the debuggable
                // fallback.
                if let Some(name) = self.capture_names.get(*idx) {
                    Expr::Ident(name.clone())
                } else {
                    Expr::Ident(format!("__cap{}", idx))
                }
            }
        }
    }

    fn constant_to_expr(&self, c: &Constant) -> Expr {
        match c {
            Constant::Number(n) => Expr::Number(*n),
            Constant::Int32(n) => Expr::Number(*n as f64),
            Constant::Bool(b) => Expr::Bool(*b),
            Constant::String(s) => Expr::String(self.sm.strings.resolve(s).to_string()),
            Constant::Null => Expr::Null,
            Constant::Undefined => Expr::Undefined,
            // Direct references to top-level functions become identifier
            // reads of the function name. Function declarations are
            // top-level, so the binding is in scope at every call site.
            Constant::Func(id) => Expr::Ident(self.sm.functions[id.0 as usize].name.clone()),
            // `GlobalThis` used as a value (not as the obj of a LoadProp /
            // StoreProp — those short-circuit at the inst level) emits as
            // the literal `globalThis` identifier. Note this is
            // intentionally a lossy normalization: `window`, `self`, and
            // `global` collapse to the same Const(GlobalThis) and therefore
            // all re-emit as `globalThis`.
            Constant::GlobalThis => Expr::Ident("globalThis".to_string()),
            Constant::This => Expr::This,
        }
    }
}

// ============================================================================
// Structure-trace walk
// ============================================================================

impl LowerCtx<'_> {
    /// Walk the CFG starting at `start`, appending statements to `out`. Stops
    /// when reaching `stop` (after emitting any required edge copies into
    /// `stop`) or when the path terminates internally.
    fn walk(&mut self, start: BlockId, stop: Option<BlockId>, out: &mut Vec<Stmt>) {
        self.walk_stack.push(start);
        if self.walk_stack.len() > WALK_STACK_LIMIT {
            self.diagnose_walk_runaway("active stack exceeded limit");
        }
        self.walk_inner(start, stop, out);
        self.walk_stack.pop();
    }

    fn walk_inner(&mut self, start: BlockId, stop: Option<BlockId>, out: &mut Vec<Stmt>) {
        let mut cur = start;
        loop {
            if Some(cur) == stop {
                return;
            }
            // Hit an open loop's header from a non-stop path (typically
            // a back-edge nested inside an if/else inside the loop
            // body): emit `continue` and return. Only the innermost open
            // loop is supported — the tree-IR has no `break` and no
            // labelled `continue`, so an outer-loop continue would need
            // a bigger feature.
            if let Some(idx) = self.loop_headers_open.iter().position(|h| *h == cur) {
                let innermost = self.loop_headers_open.len() - 1;
                if idx != innermost {
                    panic!(
                        "lower_tree: back-edge to non-innermost open loop \
                         header bb{} (innermost = bb{}) — outer-loop continue \
                         not supported by tree-IR",
                        cur.0, self.loop_headers_open[innermost].0
                    );
                }
                out.push(Stmt::Continue);
                return;
            }
            // Count every block-iteration the walker visits. Bumping per
            // iteration (not per recursive entry) catches case 3 where the
            // linear loop re-walks shared tails through repeated diamonds.
            *self.visit_counts.entry(cur).or_insert(0) += 1;
            self.walk_total_visits += 1;
            if self.walk_total_visits > WALK_TOTAL_VISIT_LIMIT {
                self.diagnose_walk_runaway("total visits exceeded limit");
            }

            // Loop header check happens BEFORE emitting instructions: the
            // header's instructions are part of the loop's per-iteration
            // condition evaluation, not pre-loop work.
            if let Some(info) = self.loop_info(cur) {
                // Try the ForIn pattern first — it's a special-case
                // four-block loop (pre-header / header / body / exit)
                // distinguished by ForInInit/Done/Next instructions.
                // Falls through to the generic while-loop emission if
                // the pattern doesn't match.
                if let Some(pat) = self.detect_forin_loop(cur, info) {
                    self.emit_for_in(cur, info, pat, out);
                } else {
                    self.emit_loop(cur, info, out);
                }
                cur = info.exit_b;
                continue;
            }

            self.emit_block_insts(cur, out);

            let term = self.func.lowered().blocks[cur.0 as usize]
                .terminator
                .clone();
            match term {
                Terminator::Return { val } => {
                    let e = val.as_ref().map(|v| self.operand_to_expr(v));
                    out.push(Stmt::Return(e));
                    return;
                }
                Terminator::Throw { val } => {
                    // Stale comment claimed tree-IR had no throw; now it
                    // does (Stmt::Throw(Expr)). Replace the __throw__
                    // placeholder call with the real Stmt::Throw — this
                    // arm becomes hot the moment lower_ssa starts
                    // constructing Terminator::Throw for Stmt::Throw.
                    let arg = self.operand_to_expr(&val);
                    out.push(Stmt::Throw(arg));
                    return;
                }
                Terminator::Unreachable => {
                    out.push(Stmt::ExprStmt(Expr::Call {
                        callee: Box::new(Expr::Ident("__unreachable__".to_string())),
                        args: vec![],
                    }));
                    return;
                }
                Terminator::Try {
                    try_entry,
                    try_args,
                    catch_entry,
                    finally_entry,
                    join,
                } => {
                    // Reconstruct `try { … } catch (param) { … } finally { … }`
                    // from the structural framing. Each body walks up
                    // to its successor landmark:
                    //   - try-body stops at finally_entry (if present)
                    //     or join.
                    //   - catch-body stops at finally_entry (if present)
                    //     or join.
                    //   - finally-body stops at join.
                    // See docs/design-try-phase-a.md §1 (Phase A) and
                    // docs/design-try-phase-b.md §1 (Phase B).

                    let body_stop = finally_entry.unwrap_or(join);

                    // Walk try-body.
                    let mut try_body_stmts = Vec::new();
                    self.emit_edge_copies(try_entry, &try_args, &mut try_body_stmts);
                    self.walk(try_entry, Some(body_stop), &mut try_body_stmts);

                    // Walk catch-body if present. CatchValue's
                    // hoisted-name lookup yields the catch-param name;
                    // emit_inst suppresses the CatchValue binding
                    // itself (the JS `catch (param)` syntax binds it
                    // implicitly).
                    let catch_clause = catch_entry.map(|ce| {
                        let param_name = self.first_inst_catch_param_name(ce);
                        let mut stmts = Vec::new();
                        self.walk(ce, Some(body_stop), &mut stmts);
                        (param_name, stmts)
                    });

                    // Walk finally-body if present.
                    let finally_clause = finally_entry.map(|fe| {
                        let mut stmts = Vec::new();
                        self.walk(fe, Some(join), &mut stmts);
                        stmts
                    });

                    out.push(Stmt::Try {
                        body: try_body_stmts,
                        catch: catch_clause,
                        finally: finally_clause,
                    });

                    if Some(join) == stop {
                        return;
                    }
                    cur = join;
                }
                Terminator::Branch { target, args } => {
                    self.emit_edge_copies(target, &args, out);
                    // Branch into the innermost open switch's join is
                    // a source-level `break;`. Recognize this BEFORE
                    // the generic stop check, since the switch
                    // reconstructor passes its join as `stop` and
                    // would otherwise return silently, dropping the
                    // `break` from the case body.
                    if self.switch_break_targets.last() == Some(&target) {
                        out.push(Stmt::Break { label: None });
                        return;
                    }
                    if Some(target) == stop {
                        return;
                    }
                    cur = target;
                }
                Terminator::CondBranch {
                    cond,
                    then_b,
                    then_args,
                    else_b,
                    else_args,
                } => {
                    let cond_expr = self.operand_to_expr(&cond);
                    let inner_stop = self.find_diamond_join(cur, then_b, else_b, stop);
                    let pass_through_stop = inner_stop.or(stop);

                    let mut then_stmts = Vec::new();
                    self.emit_edge_copies(then_b, &then_args, &mut then_stmts);
                    self.walk(then_b, pass_through_stop, &mut then_stmts);

                    let mut else_stmts = Vec::new();
                    self.emit_edge_copies(else_b, &else_args, &mut else_stmts);
                    self.walk(else_b, pass_through_stop, &mut else_stmts);

                    out.push(Stmt::If {
                        cond: cond_expr,
                        then_body: then_stmts,
                        else_body: Some(else_stmts),
                    });

                    match inner_stop {
                        Some(j) => {
                            cur = j;
                        }
                        None => return,
                    }
                }
                Terminator::Switch {
                    val,
                    cases,
                    default,
                    default_args,
                    join,
                } => {
                    let disc_expr = self.operand_to_expr(&val);
                    let mut tree_cases: Vec<tree::SwitchCase> = Vec::with_capacity(cases.len() + 1);

                    // Push the switch frame so case bodies that end
                    // in `break;` get reconstructed as `Stmt::Break`
                    // (see Terminator::Branch arm above).
                    self.switch_break_targets.push(join);

                    for c in &cases {
                        let mut body_stmts = Vec::new();
                        self.emit_edge_copies(c.target, &c.args, &mut body_stmts);
                        self.walk(c.target, Some(join), &mut body_stmts);
                        let test = self.constant_to_expr(&c.key);
                        tree_cases.push(tree::SwitchCase {
                            test: Some(test),
                            body: body_stmts,
                        });
                    }

                    // Source `default:` clause iff the default edge
                    // points at a real default-body block (not the
                    // join). Source-default-less switches lower with
                    // `default = join`, so this check round-trips
                    // exactly.
                    if default != join {
                        let mut default_body = Vec::new();
                        self.emit_edge_copies(default, &default_args, &mut default_body);
                        self.walk(default, Some(join), &mut default_body);
                        tree_cases.push(tree::SwitchCase {
                            test: None,
                            body: default_body,
                        });
                    }

                    self.switch_break_targets.pop();

                    out.push(Stmt::Switch {
                        discriminant: disc_expr,
                        cases: tree_cases,
                    });

                    // Continue walking from the join. If join is the
                    // current stop, we're done.
                    if Some(join) == stop {
                        return;
                    }
                    cur = join;
                }
            }
        }
    }

    /// Emit a basic block's straight-line instructions (no terminator). Each
    /// instruction's result Value is bound to its hoisted name via plain
    /// assignment — the `let` was emitted at function-body top.
    fn emit_block_insts(&self, cur: BlockId, out: &mut Vec<Stmt>) {
        let blk = &self.func.lowered().blocks[cur.0 as usize];
        for &iid in &blk.instructions {
            self.emit_inst(iid, out);
        }
    }

    fn emit_inst(&self, iid: InstId, out: &mut Vec<Stmt>) {
        let kind = &self.func.lowered().instructions[iid.0 as usize].kind;
        // CatchValue is the implicit "read the active exception" at a
        // catch entry. Its result Value carries the catch-param's
        // source name; references to it inside the catch body resolve
        // through `name_for` automatically. The binding itself is
        // implicit in the JS `catch (param)` syntax — do NOT emit a
        // synthetic `let __vN = catch_value();` statement here, since
        // there's no such function and the param is already bound by
        // the surrounding `Stmt::Try`'s catch clause.
        if matches!(kind, InstKind::CatchValue) {
            return;
        }
        // For-in iteration InstKinds are consumed by the surrounding
        // `Stmt::ForIn` reconstruction; their binding-statement
        // emissions are suppressed here. `ForInInit` is in the
        // pre-header (would surface as `let __iter = for_in_init(obj);`),
        // `ForInDone` is the lone header inst (would surface as
        // `let __done = for_in_done(__iter);`), and `ForInNext` is
        // the body's first inst (consumed by emit_for_in via
        // emit_block_insts_skip_first; this branch covers the case
        // where ForInNext leaks through some other path). All three
        // are encoded into the surrounding `for (binding in obj)`.
        if matches!(
            kind,
            InstKind::ForInInit { .. }
                | InstKind::ForInDone { .. }
                | InstKind::ForInNext { .. }
        ) {
            return;
        }
        if kind.is_void() {
            // Stores produce no value. Emit as a side-effecting statement.
            match kind {
                InstKind::StoreProp { obj, key, val, .. } => {
                    let prop = self.sm.strings.resolve(key).to_string();
                    // Stores rooted at `Const(GlobalThis)` re-emit as bare
                    // `name = value`, undoing the structural indirection
                    // introduced by the unbound-write lowering.
                    let target = match obj {
                        Operand::Const(Constant::GlobalThis) => Expr::Ident(prop),
                        _ if is_valid_js_ident(&prop) => Expr::Member {
                            obj: Box::new(self.operand_to_expr(obj)),
                            prop,
                        },
                        // Non-identifier key (e.g. "3", "foo bar") would
                        // be a syntax error as `obj.<key>`. Emit as
                        // computed access with the key as a string literal.
                        _ => Expr::Index {
                            obj: Box::new(self.operand_to_expr(obj)),
                            index: Box::new(Expr::String(prop)),
                        },
                    };
                    out.push(Stmt::Assign {
                        target,
                        value: self.operand_to_expr(val),
                    });
                }
                InstKind::StoreIndex {
                    obj, index, val, ..
                } => {
                    out.push(Stmt::Assign {
                        target: Expr::Index {
                            obj: Box::new(self.operand_to_expr(obj)),
                            index: Box::new(self.operand_to_expr(index)),
                        },
                        value: self.operand_to_expr(val),
                    });
                }
                _ => unreachable!("is_void() classified non-void instruction"),
            }
            return;
        }

        let name = {
            let v = find_value_for_inst(self.func, iid);
            self.name_for(v).to_string()
        };
        // Inst lowering may need to synthesize predecessor statements
        // (e.g. PassThrough closures capturing const-bound names that
        // never materialize as Inst values; without a synthetic
        // `let x = <const>;` here, the PassThrough body's lexical
        // reference to `x` would dangle in the emitted code).
        let mut pre_stmts: Vec<Stmt> = Vec::new();
        let value_expr = self.inst_kind_to_expr(kind, &mut pre_stmts);
        out.extend(pre_stmts);
        out.push(Stmt::Assign {
            target: Expr::Ident(name),
            value: value_expr,
        });
    }

    /// Lower an InstKind into a tree-IR expression. Only the kinds the
    /// current SSA converter actually emits are handled fully; the rest
    /// either fall through to a reasonable approximation or panic with a
    /// TODO message.
    fn inst_kind_to_expr(&self, kind: &InstKind, pre_stmts: &mut Vec<Stmt>) -> Expr {
        use InstKind::*;
        match kind {
            // Generic arithmetic — frontend's primary emit shape.
            AddGeneric { lhs, rhs } | AddNumber { lhs, rhs } | AddInt32 { lhs, rhs } => {
                self.binop(BinOp::Add, lhs, rhs)
            }
            SubGeneric { lhs, rhs } | SubNumber { lhs, rhs } | SubInt32 { lhs, rhs } => {
                self.binop(BinOp::Sub, lhs, rhs)
            }
            MulGeneric { lhs, rhs } | MulNumber { lhs, rhs } | MulInt32 { lhs, rhs } => {
                self.binop(BinOp::Mul, lhs, rhs)
            }
            DivGeneric { lhs, rhs } => self.binop(BinOp::Div, lhs, rhs),
            ModGeneric { lhs, rhs } => self.binop(BinOp::Mod, lhs, rhs),
            PowGeneric { lhs, rhs } => self.binop(BinOp::Pow, lhs, rhs),
            BitAnd { lhs, rhs } => self.binop(BinOp::BitAnd, lhs, rhs),
            BitOr { lhs, rhs } => self.binop(BinOp::BitOr, lhs, rhs),
            BitXor { lhs, rhs } => self.binop(BinOp::BitXor, lhs, rhs),
            Shl { lhs, rhs } => self.binop(BinOp::Shl, lhs, rhs),
            Shr { lhs, rhs } => self.binop(BinOp::Shr, lhs, rhs),
            UShr { lhs, rhs } => self.binop(BinOp::UShr, lhs, rhs),
            ConcatStr { lhs, rhs } => self.binop(BinOp::Add, lhs, rhs),
            Not { operand } => Expr::Unary {
                op: UnOp::Not,
                operand: Box::new(self.operand_to_expr(operand)),
            },
            BitNot { operand } => Expr::Unary {
                op: UnOp::BitNot,
                operand: Box::new(self.operand_to_expr(operand)),
            },
            // Value-producing logical ops shouldn't normally appear (the
            // converter routes && / || through a CFG diamond). If one does
            // sneak in, the both-sides-eval semantics differ slightly from
            // JS short-circuit, but for inputs without side effects it's
            // equivalent.
            LogAnd { lhs, rhs } => self.binop(BinOp::LogAnd, lhs, rhs),
            LogOr { lhs, rhs } => self.binop(BinOp::LogOr, lhs, rhs),
            Eq { lhs, rhs } => self.binop(BinOp::Eq, lhs, rhs),
            StrictEq { lhs, rhs } => self.binop(BinOp::StrictEq, lhs, rhs),
            Lt { lhs, rhs } => self.binop(BinOp::Lt, lhs, rhs),
            Le { lhs, rhs } => self.binop(BinOp::Le, lhs, rhs),
            LoadProp { obj, key, .. } => {
                let prop = self.sm.strings.resolve(key).to_string();
                // Reads rooted at `Const(GlobalThis)` re-emit as a bare
                // identifier (`console`, not `globalThis.console`),
                // undoing the structural indirection introduced by the
                // unbound-read lowering.
                match obj {
                    Operand::Const(Constant::GlobalThis) => Expr::Ident(prop),
                    _ if is_valid_js_ident(&prop) => Expr::Member {
                        obj: Box::new(self.operand_to_expr(obj)),
                        prop,
                    },
                    _ => Expr::Index {
                        obj: Box::new(self.operand_to_expr(obj)),
                        index: Box::new(Expr::String(prop)),
                    },
                }
            }
            LoadIndex { obj, index, .. } => Expr::Index {
                obj: Box::new(self.operand_to_expr(obj)),
                index: Box::new(self.operand_to_expr(index)),
            },
            Call { callee, args } => Expr::Call {
                callee: Box::new(self.operand_to_expr(callee)),
                args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
            },
            CallSpread { callee, args, spread_mask } => Expr::Call {
                callee: Box::new(self.operand_to_expr(callee)),
                args: args.iter().zip(spread_mask.iter()).map(|(a, &is_spread)| {
                    let e = self.operand_to_expr(a);
                    if is_spread { Expr::Spread(Box::new(e)) } else { e }
                }).collect(),
            },
            CallMethod { obj, method, args } => {
                let callee = Expr::Member {
                    obj: Box::new(self.operand_to_expr(obj)),
                    prop: self.sm.strings.resolve(method).to_string(),
                };
                Expr::Call {
                    callee: Box::new(callee),
                    args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
                }
            }
            CallMethodSpread { obj, method, args, spread_mask } => {
                let callee = Expr::Member {
                    obj: Box::new(self.operand_to_expr(obj)),
                    prop: self.sm.strings.resolve(method).to_string(),
                };
                Expr::Call {
                    callee: Box::new(callee),
                    args: args.iter().zip(spread_mask.iter()).map(|(a, &is_spread)| {
                        let e = self.operand_to_expr(a);
                        if is_spread { Expr::Spread(Box::new(e)) } else { e }
                    }).collect(),
                }
            }
            ToNumber { val } => Expr::Unary {
                op: UnOp::Pos,
                operand: Box::new(self.operand_to_expr(val)),
            },
            Typeof { val } => Expr::Unary {
                op: UnOp::Typeof,
                operand: Box::new(self.operand_to_expr(val)),
            },
            // The rest aren't emitted by the current converter. Keep loud
            // panics so a future addition surfaces here rather than silently
            // generating wrong code.
            StoreProp { .. } | StoreIndex { .. } => {
                unreachable!("void inst handled in emit_inst")
            }
            AllocObject => Expr::Call {
                callee: Box::new(Expr::Ident("Object".to_string())),
                args: vec![],
            },
            AllocArray { .. } => Expr::Call {
                callee: Box::new(Expr::Ident("Array".to_string())),
                args: vec![],
            },
            AllocRegExp { source } => {
                Expr::RegExp(self.sm.strings.resolve(source).to_string())
            }
            ArrayWithSpread { elements, spread_mask } => {
                use ir::tree::Expr as TreeExpr;
                let elems: Vec<TreeExpr> = elements.iter().zip(spread_mask.iter()).map(|(e, &is_spread)| {
                    let te = self.operand_to_expr(e);
                    if is_spread { TreeExpr::Spread(Box::new(te)) } else { te }
                }).collect();
                TreeExpr::Array { elements: elems }
            }
            ObjectWithSpread { keys, vals } => {
                use ir::tree::{Expr as TreeExpr, ObjectKey, ObjectProp};
                use ir::ssa::SpreadObjectKey;
                let props: Vec<ObjectProp> = keys.iter().zip(vals.iter()).map(|(k, v)| {
                    let val = self.operand_to_expr(v);
                    match k {
                        None => ObjectProp { key: ObjectKey::Spread, value: val },
                        Some(SpreadObjectKey::Static(sid)) => ObjectProp {
                            key: ObjectKey::Static(self.sm.strings.resolve(sid).to_string()),
                            value: val,
                        },
                        Some(SpreadObjectKey::Computed(key_op)) => ObjectProp {
                            key: ObjectKey::Computed(Box::new(self.operand_to_expr(key_op))),
                            value: val,
                        },
                    }
                }).collect();
                TreeExpr::Object { props }
            }
            AllocClosure { func, captures } => {
                self.alloc_closure_to_expr(*func, captures, pre_stmts)
            }
            Construct { ctor, args } => Expr::New {
                callee: Box::new(self.operand_to_expr(ctor)),
                args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
            },
            ToString { val } => Expr::Call {
                callee: Box::new(Expr::Ident("String".to_string())),
                args: vec![self.operand_to_expr(val)],
            },
            ToBool { val } => Expr::Call {
                callee: Box::new(Expr::Ident("Boolean".to_string())),
                args: vec![self.operand_to_expr(val)],
            },
            ToInt32 { val } => Expr::Binary {
                op: BinOp::BitOr,
                lhs: Box::new(self.operand_to_expr(val)),
                rhs: Box::new(Expr::Number(0.0)),
            },
            Instanceof { obj, ctor } => Expr::Binary {
                op: BinOp::Instanceof,
                lhs: Box::new(self.operand_to_expr(obj)),
                rhs: Box::new(self.operand_to_expr(ctor)),
            },
            In { key, obj } => Expr::Binary {
                op: BinOp::In,
                lhs: Box::new(self.operand_to_expr(key)),
                rhs: Box::new(self.operand_to_expr(obj)),
            },
            OpaqueEval { source } => Expr::Call {
                callee: Box::new(Expr::Ident("eval".to_string())),
                args: vec![self.operand_to_expr(source)],
            },
            // CatchValue is consumed by the Stmt::Try reconstruction
            // (see the Terminator::Try arm in `walk`); the inst's
            // emission is suppressed at the start of catch_entry.
            // Reaching this fallback means the pattern-match missed
            // — the JS `catch (param)` syntax binds the value
            // implicitly, so a free CatchValue is meaningless.
            CatchValue => unreachable!(
                "lower_tree: CatchValue reached inst_kind_to_expr — \
                 Stmt::Try reconstruction should have consumed it"
            ),
            // For-in iteration InstKinds are consumed by the
            // four-block `Stmt::ForIn` reconstruction pattern in
            // `walk` (loop_info detection). Reaching this fallback
            // means the pattern-match missed — surface as a clear
            // panic so the next maintainer knows where to look.
            ForInInit { .. } | ForInDone { .. } | ForInNext { .. } => unreachable!(
                "lower_tree: ForIn iteration InstKind reached \
                 inst_kind_to_expr — the four-block ForIn loop \
                 pattern in `walk` should have consumed it. See \
                 docs/design-forin.md."
            ),
        }
    }

    /// Recursively lower an inner function (Lowered or PassThrough) into
    /// an `Expr::Func(FuncLiteral)`. Captures are resolved by rendering
    /// each capture operand through `operand_to_expr` and stringifying
    /// the resulting expression — the inner body's `Operand::Capture(i)`
    /// reads will pick up the same name through the inner LowerCtx's
    /// `capture_names` vector.
    fn alloc_closure_to_expr(
        &self,
        func: FuncId,
        captures: &[Operand],
        pre_stmts: &mut Vec<Stmt>,
    ) -> Expr {
        let inner = &self.sm.functions[func.0 as usize];

        // Look up the SSA-side capture-name list. Inner Lowered bodies
        // need `capture_names` to resolve `Operand::Capture(idx)`;
        // PassThrough bodies use lexical names from their source
        // slice, but we still need the names here to synthesize
        // pre-let bindings for any const-only captures (otherwise the
        // outer's emit lacks a `let x = ...` for names referenced by
        // the PassThrough body — DCE has already removed any
        // would-be Inst, and without a let the body's `x` dangles).
        let cap_names: Vec<String> = match &inner.body {
            ssa::FunctionBody::PassThrough(_) => self.passthrough_capture_names(func),
            ssa::FunctionBody::Lowered(_) => captures
                .iter()
                .map(|o| match self.operand_to_expr(o) {
                    Expr::Ident(n) => n,
                    other => render_expr_to_string(&other),
                })
                .collect(),
        };

        // Note: outer's let-bindings + assignments for names
        // referenced by this PassThrough body are emitted via the
        // outer Function's `forced_let_decls` (declaration) and
        // synthesized `StoreProp(GlobalThis, name, val)` instructions
        // produced at SSA lowering time. See `pt_referenced_names`
        // in lower_ssa.rs. We don't prepend pre-lets here.
        let _ = &pre_stmts; // pre_stmts may still be used by other arms

        let lit = self.build_func_literal(inner, cap_names);
        Expr::Func(Box::new(lit))
    }

    /// Recover the alphabetically-sorted capture-name list for a
    /// PassThrough function. The list was computed during SSA
    /// lowering's pass 1 and stashed in `inner_captures`; here we
    /// re-derive it from the function's body source by re-running
    /// the PassThrough free-var analysis. Cheap (small slice, single
    /// parse) and avoids threading a side table through the
    /// SSA→tree boundary.
    fn passthrough_capture_names(&self, func: FuncId) -> Vec<String> {
        let f = &self.sm.functions[func.0 as usize];
        let pt = match &f.body {
            ssa::FunctionBody::PassThrough(pt) => pt,
            _ => return Vec::new(),
        };
        let params: Vec<String> = pt
            .param_names
            .iter()
            .map(|n| self.sm.strings.resolve(n).to_string())
            .collect();
        let name = pt
            .function_name
            .map(|s| self.sm.strings.resolve(&s).to_string());
        ir::pt_free_vars::analyze_passthrough(
            &pt.source,
            &params,
            name.as_deref(),
            pt.is_async,
            pt.is_generator,
        )
    }

    fn build_func_literal(
        &self,
        inner: &Function,
        capture_names: Vec<String>,
    ) -> tree::FuncLiteral {
        match &inner.body {
            ssa::FunctionBody::PassThrough(pt) => tree::FuncLiteral {
                name: pt
                    .function_name
                    .map(|s| self.sm.strings.resolve(&s).to_string()),
                params: pt
                    .param_names
                    .iter()
                    .map(|n| self.sm.strings.resolve(n).to_string())
                    .collect(),
                is_async: pt.is_async,
                is_generator: pt.is_generator,
                is_arrow: pt.is_arrow,
                body: tree::FuncBody::PassThrough(pt.source.clone()),
                func_id: std::cell::Cell::new(None),
                passthrough_source: Some(pt.source.clone()),
                force_passthrough: std::cell::Cell::new(false),
                blockers: std::collections::BTreeSet::new(),
            },
            ssa::FunctionBody::Lowered(_) => {
                // Recursively lower the inner body. We use the default
                // EmitConfig — pin propagation is Stream C's concern.
                let cfg = EmitConfig::default();
                let mut ctx = LowerCtx::new(self.sm, inner, &cfg, capture_names);
                let param_names: Vec<String> = inner
                    .lowered()
                    .params
                    .iter()
                    .map(|v| ctx.name_for(*v).to_string())
                    .collect();
                let mut body = Vec::new();
                for name in ctx.hoisted_names_in_decl_order() {
                    body.push(Stmt::Let { name, init: None });
                }
                for &sid in &inner.forced_let_decls {
                    let n = ctx.sm.strings.resolve(&sid).to_string();
                    body.push(Stmt::Let {
                        name: n,
                        init: None,
                    });
                }
                ctx.walk(inner.lowered().entry, None, &mut body);

                // Detect arrow-shaped output? Inner Lowered functions
                // came from either a function expression or an arrow.
                // The SSA layer doesn't preserve which; emit as a
                // function expression conservatively (named if a name
                // exists, anonymous otherwise). Stream B accepts this
                // round-trip diff: an arrow input may re-emit as a
                // function expression. Both are observably equivalent
                // modulo `this` binding, which we don't model.
                let name = if !inner.name.is_empty() {
                    Some(inner.name.clone())
                } else {
                    None
                };
                tree::FuncLiteral {
                    name,
                    params: param_names,
                    is_async: false,
                    is_generator: false,
                    is_arrow: false,
                    body: tree::FuncBody::Stmts(body),
                    func_id: std::cell::Cell::new(None),
                    passthrough_source: None,
                    force_passthrough: std::cell::Cell::new(false),
                    blockers: std::collections::BTreeSet::new(),
                }
            }
        }
    }

    fn binop(&self, op: BinOp, lhs: &Operand, rhs: &Operand) -> Expr {
        Expr::Binary {
            op,
            lhs: Box::new(self.operand_to_expr(lhs)),
            rhs: Box::new(self.operand_to_expr(rhs)),
        }
    }

    /// Emit `name(target.params[i]) = expr(args[i])` assignments for a
    /// `Branch { target, args }` edge. The names are hoisted at function
    /// top, so `Stmt::Assign` is always sufficient — no `let` ever needed
    /// here.
    fn emit_edge_copies(&self, target: BlockId, args: &[Operand], out: &mut Vec<Stmt>) {
        let target_blk = &self.func.lowered().blocks[target.0 as usize];
        debug_assert_eq!(
            target_blk.params.len(),
            args.len(),
            "edge arity mismatch into bb{}",
            target.0
        );
        for (param, arg) in target_blk.params.iter().zip(args.iter()) {
            let lhs_name = self.name_for(*param).to_string();
            let rhs_expr = self.operand_to_expr(arg);
            // Suppress trivial self-copies (`x = x`) — they're noise after
            // trivial-param removal would have caught them, but the SSA
            // form may legitimately keep a param whose name now matches one
            // of its own incomings.
            if let Expr::Ident(rhs_name) = &rhs_expr
                && rhs_name == &lhs_name
            {
                continue;
            }
            out.push(Stmt::Assign {
                target: Expr::Ident(lhs_name),
                value: rhs_expr,
            });
        }
    }

    // -----------------------------------------------------------------
    // Loop emission
    // -----------------------------------------------------------------

    /// Emit a `while` loop for `header` (already known to be a loop header).
    /// The body walks until reaching `header` again (the back-edge), at
    /// which point the back-edge copies are emitted at the body's tail.
    fn emit_loop(&mut self, header: BlockId, info: LoopInfo, out: &mut Vec<Stmt>) {
        let header_blk = &self.func.lowered().blocks[header.0 as usize];
        let cond_op = match &header_blk.terminator {
            Terminator::CondBranch { cond, .. } => *cond,
            _ => unreachable!("loop_info already validated CondBranch terminator"),
        };

        // Try to inline the cond. If the header has exactly one instruction
        // whose result IS the cbr's cond, we can inline it into the
        // `while (...)` expression — the typical lower_while shape produces
        // this. Otherwise we fall back to evaluating the header's
        // instructions both before the loop AND at the body's tail (and
        // reading the cond Value name in the while header).
        let (cond_expr, header_insts_handled_inline) = self.try_inline_cond(header, &cond_op);

        let mut body_stmts = Vec::new();

        // Walk through the cond-edge's copies (cbr edges have no args in
        // our converter, but we go through emit_edge_copies for symmetry
        // and future-proofing).
        let cbr_then_args: Vec<Operand>;
        let cbr_else_args: Vec<Operand>;
        match &header_blk.terminator {
            Terminator::CondBranch {
                then_args,
                else_args,
                ..
            } => {
                cbr_then_args = then_args.clone();
                cbr_else_args = else_args.clone();
            }
            _ => unreachable!(),
        };

        // Determine which cbr branch is the body and which is the exit (the
        // ordering depends on how lower_while's cbr ended up).
        let (body_args, exit_args) = match &header_blk.terminator {
            Terminator::CondBranch { then_b, else_b, .. } => {
                if *then_b == info.body_b {
                    (cbr_then_args.clone(), cbr_else_args.clone())
                } else {
                    debug_assert_eq!(*else_b, info.body_b);
                    (cbr_else_args.clone(), cbr_then_args.clone())
                }
            }
            _ => unreachable!(),
        };

        // Inside the loop body: emit copies for the body-edge from header,
        // then walk body until reaching the header (back-edge stop).
        self.emit_edge_copies(info.body_b, &body_args, &mut body_stmts);
        // Mark this header as an open loop so nested if/else branches whose
        // back-edge re-enters the header emit `continue` instead of
        // recursing into a fresh loop emission.
        self.loop_headers_open.push(header);
        self.walk(info.body_b, Some(header), &mut body_stmts);
        let popped = self.loop_headers_open.pop();
        debug_assert_eq!(popped, Some(header));

        // If we couldn't inline the cond, the header's instructions need to
        // re-evaluate at the end of the body so the next iteration's check
        // sees up-to-date values. (And BEFORE the loop so the first check
        // sees initial values — emitted just below.)
        if !header_insts_handled_inline {
            self.emit_block_insts(header, &mut body_stmts);
        }

        // For the same reason, emit header's instructions BEFORE the loop too.
        if !header_insts_handled_inline {
            self.emit_block_insts(header, out);
        }

        out.push(Stmt::While {
            cond: cond_expr,
            body: body_stmts,
        });

        // Emit copies for the exit-edge into exit_b. (lower_while produces
        // empty exit_args, so this is a no-op in the typical case.)
        self.emit_edge_copies(info.exit_b, &exit_args, out);
    }

    /// Pattern detected at a loop header that distinguishes a `for-in`
    /// loop from a generic while-loop. Carries the operands needed by
    /// `emit_for_in` to reconstruct `Stmt::ForIn { kind, binding, obj, body }`.
    fn detect_forin_loop(&mut self, header: BlockId, info: LoopInfo) -> Option<ForInPattern> {
        let header_blk = &self.func.lowered().blocks[header.0 as usize];
        // Header must end in `CondBranch cond, exit, body` where cond is
        // a `ForInDone(iter)` instruction. Construction emits
        // `ForInDone` then `CondBranch cond=done, then=exit, else=body`.
        let (cond_op, then_b, else_b) = match &header_blk.terminator {
            Terminator::CondBranch { cond, then_b, else_b, .. } => (*cond, *then_b, *else_b),
            _ => return None,
        };
        // Construction emits `then_b == exit` (taken when iteration is
        // exhausted). loop_info classifies the back-edge-reaching branch
        // as body_b — so for our shape, info.body_b == else_b and
        // info.exit_b == then_b.
        if info.body_b != else_b || info.exit_b != then_b {
            return None;
        }
        // Header's cond must be a single inst in the header.
        let cv = match cond_op {
            Operand::Value(v) => v,
            _ => return None,
        };
        let cv_inst = match self.func.lowered().values[cv.0 as usize].kind {
            ValueDefKind::Inst(iid) => iid,
            _ => return None,
        };
        if header_blk.instructions.len() != 1 || header_blk.instructions[0] != cv_inst {
            return None;
        }
        let iter_op = match &self.func.lowered().instructions[cv_inst.0 as usize].kind {
            InstKind::ForInDone { iter } => *iter,
            _ => return None,
        };
        // Body's first inst must be a matching `ForInNext(iter)`.
        let body_blk = &self.func.lowered().blocks[info.body_b.0 as usize];
        let next_iid = *body_blk.instructions.first()?;
        let next_iter = match &self.func.lowered().instructions[next_iid.0 as usize].kind {
            InstKind::ForInNext { iter } => *iter,
            _ => return None,
        };
        // Iter operand match: ForInDone and ForInNext must use the
        // same iter Value (Operand doesn't derive PartialEq;
        // construction always emits Value-typed iters).
        match (iter_op, next_iter) {
            (Operand::Value(a), Operand::Value(b)) if a.0 == b.0 => {}
            _ => return None,
        }
        // Pre-header (header's non-back-edge predecessor) must end with
        // a `ForInInit(obj)` whose result is `iter`. The pre-header is
        // the unique predecessor of `header` that is NOT dominated by
        // `header` (the back-edge predecessor IS).
        let pre_header_b = header_blk
            .predecessors
            .iter()
            .copied()
            .find(|p| !self.is_back_edge_pred(*p, header))?;
        let pre_header_blk = &self.func.lowered().blocks[pre_header_b.0 as usize];
        let init_iid = *pre_header_blk.instructions.last()?;
        let obj_op = match &self.func.lowered().instructions[init_iid.0 as usize].kind {
            InstKind::ForInInit { obj } => *obj,
            _ => return None,
        };
        // Iter operand must match the ForInInit's result.
        let iter_value = match iter_op {
            Operand::Value(v) => v,
            _ => return None,
        };
        let init_result_inst = match self.func.lowered().values[iter_value.0 as usize].kind {
            ValueDefKind::Inst(i) => i,
            _ => return None,
        };
        if init_result_inst != init_iid {
            return None;
        }
        Some(ForInPattern {
            obj: obj_op,
            iter_op,
            init_iid,
            done_iid: cv_inst,
            next_iid,
            pre_header_b,
        })
    }

    /// Cheap "is `pred` reached only via a back-edge from `header`'s
    /// dominated subgraph" check — a back-edge predecessor IS reached
    /// from header (since header dominates the body), whereas the
    /// pre-header is not.
    fn is_back_edge_pred(&mut self, pred: BlockId, header: BlockId) -> bool {
        self.dominates(header, pred)
    }

    /// Emit `Stmt::ForIn { kind: "var", binding, obj, body }` for a
    /// detected ForIn loop. Body is walked from `info.body_b`'s
    /// instruction-after-ForInNext up to the back-edge.
    fn emit_for_in(
        &mut self,
        header: BlockId,
        info: LoopInfo,
        pat: ForInPattern,
        out: &mut Vec<Stmt>,
    ) {
        // The user's binding name is the source-tagged name on the
        // `ForInNext` result Value. Construction calls `set_source_name`
        // with the original `binding` string from the tree-IR's
        // `Stmt::ForIn { binding, .. }`.
        let next_value = find_value_for_inst(self.func, pat.next_iid);
        let binding_name = self.name_for(next_value).to_string();
        let obj_expr = self.operand_to_expr(&pat.obj);

        // Body: walk body_b until reaching `header` (back-edge stop).
        // The leading `ForInNext` inst is suppressed by emit_inst's
        // ForInInit/Done/Next match (set in commit 1), so the standard
        // `walk` machinery handles bodies with internal control flow
        // (nested if / loop / switch / etc.) naturally.
        let mut body_stmts = Vec::new();
        self.loop_headers_open.push(header);
        self.walk(info.body_b, Some(header), &mut body_stmts);
        let popped = self.loop_headers_open.pop();
        debug_assert_eq!(popped, Some(header));

        // The pre-header's `ForInInit` and the header's `ForInDone` are
        // implicit in the emitted `for (binding in obj)` — emit_block_insts
        // for those blocks must NOT emit them as bare `let __vN = for_in_init(obj)`
        // statements. Construction places these instructions at known
        // positions; reconstruction's emit_inst suppresses them by
        // checking the InstKind. (See emit_inst's CatchValue precedent.)

        out.push(Stmt::ForIn {
            kind: "var".to_string(),
            binding: binding_name,
            obj: Box::new(obj_expr),
            body: body_stmts,
        });
    }

    /// If the header's cond is the result of a single instruction in the
    /// header, return the inlined expression and `true`; else return the
    /// fallback (use the cond Value's name) and `false`.
    fn try_inline_cond(&self, header: BlockId, cond_op: &Operand) -> (Expr, bool) {
        let blk = &self.func.lowered().blocks[header.0 as usize];
        if let Operand::Value(cv) = cond_op
            && let ValueDefKind::Inst(cv_inst) = self.func.lowered().values[cv.0 as usize].kind
            && blk.instructions.len() == 1
            && blk.instructions[0] == cv_inst
        {
            let kind = &self.func.lowered().instructions[cv_inst.0 as usize].kind;
            // Inlining a cond expression cannot itself produce
            // pre-stmts (cond inlining is only for a single
            // single-block-instruction; AllocClosure is never the
            // cond of a CondBranch). A discarded sink suffices.
            let mut sink = Vec::new();
            return (self.inst_kind_to_expr(kind, &mut sink), true);
        }
        // Fallback. If header has zero instructions, this still works — we
        // just read the cond name (or constant). If it has more than one
        // instruction, the caller emits all of them before the loop and at
        // body tail.
        (self.operand_to_expr(cond_op), blk.instructions.is_empty())
    }

    // -----------------------------------------------------------------
    // CFG analyses (cached)
    // -----------------------------------------------------------------

    fn loop_info(&mut self, b: BlockId) -> Option<LoopInfo> {
        if let Some(c) = self.loop_info_cache.get(&b) {
            return *c;
        }
        let result = self.compute_loop_info(b);
        self.loop_info_cache.insert(b, result);
        result
    }

    fn compute_loop_info(&mut self, b: BlockId) -> Option<LoopInfo> {
        let block = &self.func.lowered().blocks[b.0 as usize];
        let (then_b, else_b) = match &block.terminator {
            Terminator::CondBranch { then_b, else_b, .. } => (*then_b, *else_b),
            _ => return None,
        };
        // Back-edge test: is there a predecessor dominated by `b`? Only then
        // is `b` a loop header in the standard sense. Without this guard, a
        // cbr inside a loop body whose successors transitively reach the
        // OUTER header would be misidentified as a header.
        let preds = block.predecessors.clone();
        let has_back_edge = preds.iter().any(|&p| self.dominates(b, p));
        if !has_back_edge {
            return None;
        }

        // Reachability for loop-shape detection must stay inside the
        // header's dominated subgraph. A naïve forward-reach on
        // `else_b` (the loop's exit branch) can transitively re-enter
        // the header through some OUTER loop iterating around the
        // current one — e.g., `case 2: while (inner) { if (c) return;
        // else x; }; continue;` inside `while (true) { switch (...)
        // {...} break; }`. The inner-while's exit edges to the outer-
        // while's header, which then re-enters the switch and the
        // case body, transitively reaching the inner header. That
        // path is not a back-edge of the inner loop — it leaves the
        // inner loop's dominated subgraph and re-enters from outside.
        // Bounding the BFS by `dominates(b, succ)` confines the
        // analysis to the loop being classified.
        let then_reaches_b = self.reaches_within_dom(then_b, b, b);
        let else_reaches_b = self.reaches_within_dom(else_b, b, b);
        match (then_reaches_b, else_reaches_b) {
            (true, false) => Some(LoopInfo {
                body_b: then_b,
                exit_b: else_b,
            }),
            (false, true) => Some(LoopInfo {
                body_b: else_b,
                exit_b: then_b,
            }),
            (true, true) => panic!(
                "loop header bb{} has both cbr branches reaching the header — \
                 unsupported irreducible CFG; needs a real relooper",
                b.0
            ),
            (false, false) => panic!(
                "block bb{} has a back-edge predecessor but neither cbr branch \
                 reaches the header — CFG analysis disagrees with itself",
                b.0
            ),
        }
    }

    /// `dominator` dominates `dominated` iff every path from entry to
    /// `dominated` passes through `dominator`. Equivalent: removing
    /// `dominator` leaves `dominated` unreachable from entry.
    ///
    /// Dominance is a reachability-conditional property: if `dominated`
    /// is unreachable from entry to begin with, dominance is undefined
    /// and we return `false`. Without that guard, the
    /// reachability-by-blocking BFS below would conclude
    /// `reaches_target=false` for any unreachable target — regardless
    /// of `dominator` — and report `dominates=true`, falsely flagging
    /// every block as a dominator of every unreachable block. That
    /// misclassification surfaced on `lodash_cloneDeep.high.js`: a
    /// stale predecessor entry pointed at an unreachable block, which
    /// then registered as a phantom back-edge in
    /// `compute_loop_info` (the dominated-predecessor check), causing
    /// `loop_info` to attempt loop-shape analysis on a non-loop and
    /// panic when neither cbr branch reached the alleged header.
    fn dominates(&mut self, dominator: BlockId, dominated: BlockId) -> bool {
        if dominator == dominated {
            return true;
        }
        if dominator == self.func.lowered().entry {
            return true;
        }
        let key = (dominator, dominated);
        if let Some(v) = self.dom_cache.get(&key) {
            return *v;
        }
        // Reachability precondition: an unreachable `dominated` is not
        // dominated by any block (textbook definition). The cache is
        // shared with the post-block walk below; this short-circuit
        // does not pollute it because we only insert under `key` after
        // the full computation.
        let from_entry = self.forward_reachable(self.func.lowered().entry);
        if !from_entry.contains(&dominated) {
            self.dom_cache.insert(key, false);
            return false;
        }
        let mut visited: HashSet<BlockId> = HashSet::new();
        let mut stack = vec![self.func.lowered().entry];
        let mut reaches_target = false;
        while let Some(b) = stack.pop() {
            if b == dominator {
                continue;
            }
            if !visited.insert(b) {
                continue;
            }
            if b == dominated {
                reaches_target = true;
                break;
            }
            for (succ, _) in self.func.lowered().blocks[b.0 as usize]
                .terminator
                .successors()
            {
                stack.push(succ);
            }
        }
        let dominates = !reaches_target;
        self.dom_cache.insert(key, dominates);
        dominates
    }

    fn reaches(&mut self, start: BlockId, target: BlockId) -> bool {
        // Convenience: reachability through forward edges.
        let r = self.forward_reachable(start);
        r.contains(&target)
    }

    /// Reachability bounded to the dominated subgraph of `dominator`.
    /// Walks forward from `start`, only enqueuing successors that
    /// `dominator` dominates. Used by loop-shape detection so paths
    /// that exit the loop's region and re-enter through an outer
    /// loop's iteration don't masquerade as back-edges. Not cached —
    /// the dominator parameter would expand the cache key beyond the
    /// hot path.
    fn reaches_within_dom(
        &mut self,
        start: BlockId,
        target: BlockId,
        dominator: BlockId,
    ) -> bool {
        if start == target {
            return true;
        }
        let mut visited: HashSet<BlockId> = HashSet::new();
        let mut stack = vec![start];
        while let Some(b) = stack.pop() {
            if !visited.insert(b) {
                continue;
            }
            if b == target {
                return true;
            }
            for (succ, _) in self.func.lowered().blocks[b.0 as usize]
                .terminator
                .successors()
            {
                // Stay inside the loop's dominated subgraph. The
                // header itself is self-dominated, so the back-edge
                // succ == target is allowed and triggers the next-
                // iteration `b == target` check.
                if self.dominates(dominator, succ) {
                    stack.push(succ);
                }
            }
        }
        false
    }

    fn forward_reachable(&mut self, start: BlockId) -> HashSet<BlockId> {
        if let Some(s) = self.reachable_cache.get(&start) {
            return s.clone();
        }
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(b) = stack.pop() {
            if !visited.insert(b) {
                continue;
            }
            for (succ, _) in self.func.lowered().blocks[b.0 as usize]
                .terminator
                .successors()
            {
                stack.push(succ);
            }
        }
        self.reachable_cache.insert(start, visited.clone());
        visited
    }

    /// Given an if/else cbr with branches `b1` and `b2`, find the diamond's
    /// post-dominator (the join). Returns None if the branches have no
    /// common forward-reachable block, or if the only common block is the
    /// parent's `stop` (in which case the parent will continue from there
    /// after the if statement returns control).
    fn find_diamond_join(
        &mut self,
        peak: BlockId,
        b1: BlockId,
        b2: BlockId,
        parent_stop: Option<BlockId>,
    ) -> Option<BlockId> {
        let r1 = self.forward_reachable(b1);
        let r2 = self.forward_reachable(b2);
        let common: Vec<BlockId> = r1.intersection(&r2).copied().collect();
        if common.is_empty() {
            return None;
        }
        // Filter out the parent's stop — if both branches reach it, that's
        // the parent's responsibility, not an if-internal join. Also
        // exclude `b1` and `b2` themselves: a block reaches itself
        // trivially (distance=0), which would make a branch block win
        // over the actual join.
        //
        // Also exclude `peak` (the cbr's own source block): when the
        // cbr sits inside a loop body, both branches transitively
        // reach `peak` via the loop's back-edge. Without this
        // exclusion `peak` ends up in `common` with the same score
        // as the legitimate join. Tie-breaking ("first wins" over
        // hashset iteration order) is non-deterministic, so the
        // walker sometimes picks `peak` itself, sets `cur = peak`,
        // and infinitely revisits the cbr.
        let candidates: Vec<BlockId> = common
            .into_iter()
            .filter(|b| Some(*b) != parent_stop && *b != b1 && *b != b2 && *b != peak)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        // Pick the candidate that is a post-dominator: a block whose
        // forward-reachable set is a SUBSET of every other candidate's
        // forward-reachable set (i.e., everyone passes through it before
        // diverging again). For the shapes our converter produces this is
        // unambiguous and equal to the explicit join_b in lower_if.
        //
        // Score by (distance(b1, c) + distance(b2, c)). The true join
        // is the closest block both branches BOTH reach; a block on
        // only one branch's path scores worse because the other
        // branch's distance includes a long loop detour. `min` would
        // tie when both arms are "1 hop from one branch, far from the
        // other" (e.g. inside a loop, `c` is bb_then-tail's successor
        // and `c'` is bb_else-tail's successor — both score min=1 but
        // only one is the true post-dominator). Sum disambiguates.
        // Naive O(n²) is fine — n is the diamond interior, small in
        // practice.
        let saturating = |a: usize, b: usize| a.checked_add(b).unwrap_or(usize::MAX);
        let mut best = candidates[0];
        let mut best_score = usize::MAX;
        for &c in &candidates {
            let score = saturating(self.distance(b1, c), self.distance(b2, c));
            if score < best_score {
                best_score = score;
                best = c;
            }
        }
        Some(best)
    }

    /// Forward distance (BFS hop count) from `start` to `target`. Returns
    /// `usize::MAX` if unreachable.
    fn distance(&self, start: BlockId, target: BlockId) -> usize {
        if start == target {
            return 0;
        }
        let mut visited: HashSet<BlockId> = HashSet::new();
        let mut frontier = vec![start];
        let mut depth = 0usize;
        while !frontier.is_empty() {
            depth += 1;
            let mut next: Vec<BlockId> = Vec::new();
            for b in frontier {
                if !visited.insert(b) {
                    continue;
                }
                for (succ, _) in self.func.lowered().blocks[b.0 as usize]
                    .terminator
                    .successors()
                {
                    if succ == target {
                        return depth;
                    }
                    next.push(succ);
                }
            }
            frontier = next;
        }
        usize::MAX
    }
}

// ============================================================================
// Free helpers
// ============================================================================

/// Two-pass renamer. Returns a `(value_index → final_name)` map and the
/// list of `_tN`-style hoisted-temporary names whose `let` declarations
/// must be emitted at the function body's top.
///
/// Determinism: the COLLECT pass walks Values in index order (which is
/// the order `FunctionBuilder` allocates them — left-to-right by
/// instruction emission and parameter declaration). The ASSIGN pass
/// honors the COLLECT order so two equivalent SSA modules produce
/// byte-identical names.
///
/// Scope: collisions are detected at function granularity, not per JS
/// block. This is a deliberate simplification documented in the ticket
/// — two source-level `i`s in unrelated for-loops emit as `i` and
/// `i_1` rather than both as `i` (which would be JS-legal under block
/// scoping). Output is slightly uglier in pathological cases, but the
/// renamer is dramatically simpler. Revisit if quality complaints
/// surface.
fn assign_names(
    sm: &ssa::Module,
    func: &Function,
    cfg: &EmitConfig,
) -> (HashMap<u32, String>, Vec<String>) {
    let mut final_names: HashMap<u32, String> = HashMap::new();
    let mut committed: HashMap<String, u32> = HashMap::new();
    let mut hoisted: Vec<String> = Vec::new();
    let mut t_counter: u32 = 0;

    // Pre-claim every pinned name to the pinned Value. Caller
    // staging-conflict (two distinct Values pinned to the same name)
    // panics here.
    let pinned_names: std::collections::HashSet<&str> =
        cfg.immovable_names.values().map(|s| s.as_str()).collect();
    for (&vi, name) in &cfg.immovable_names {
        if let Some(&prior) = committed.get(name)
            && prior != vi
        {
            panic!(
                "EmitConfig.immovable_names pins both %{prior} and %{vi} to `{name}`. \
                 Caller bug — two distinct Values cannot share an immovable name."
            );
        }
        committed.insert(name.clone(), vi);
        final_names.insert(vi, name.clone());
    }

    for (i, def) in func.lowered().values.iter().enumerate() {
        let vi = i as u32;
        if final_names.contains_key(&vi) {
            // Already assigned via pinning; if it's an Inst or
            // BlockParam its name still needs to surface in `hoisted`
            // so the function-top let-block declares it.
            if matches!(
                def.kind,
                ValueDefKind::Inst(_) | ValueDefKind::BlockParam { .. }
            ) {
                hoisted.push(final_names[&vi].clone());
            }
            continue;
        }
        match &def.kind {
            ValueDefKind::Removed => continue,
            ValueDefKind::Arg(_) => {
                let preferred = def.source_name.map(|s| sm.strings.resolve(&s).to_string());
                let chosen = pick_name(
                    preferred,
                    &mut committed,
                    vi,
                    &pinned_names,
                    &mut t_counter,
                    false,
                );
                final_names.insert(vi, chosen);
            }
            ValueDefKind::Inst(_) | ValueDefKind::BlockParam { .. } => {
                let preferred = def.source_name.map(|s| sm.strings.resolve(&s).to_string());
                let chosen = pick_name(
                    preferred,
                    &mut committed,
                    vi,
                    &pinned_names,
                    &mut t_counter,
                    true,
                );
                hoisted.push(chosen.clone());
                final_names.insert(vi, chosen);
            }
        }
    }

    (final_names, hoisted)
}

/// Resolve the final name for a single Value. `for_let` controls only
/// whether the synthesized fallback uses the `_tN` (true) or `_aN`
/// (false) prefix; both prefixes share the committed-name space because
/// emitted JS scopes them together.
///
/// `pinned` is the set of names already locked by EmitConfig. The
/// suffix-collision path skips any candidate that lands on a pinned
/// name; bare-name preference for a pinned name fails through to the
/// suffix path (the pin's owner already claimed the bare slot).
fn pick_name(
    preferred: Option<String>,
    committed: &mut HashMap<String, u32>,
    vi: u32,
    pinned: &std::collections::HashSet<&str>,
    t_counter: &mut u32,
    for_let: bool,
) -> String {
    if let Some(want) = preferred {
        if !committed.contains_key(&want) && !pinned.contains(want.as_str()) {
            committed.insert(want.clone(), vi);
            return want;
        }
        let mut k: u32 = 1;
        loop {
            let candidate = format!("{want}_{k}");
            if !committed.contains_key(&candidate) && !pinned.contains(candidate.as_str()) {
                committed.insert(candidate.clone(), vi);
                return candidate;
            }
            k += 1;
        }
    }

    loop {
        let prefix = if for_let { "_t" } else { "_a" };
        let candidate = format!("{prefix}{}", *t_counter);
        *t_counter += 1;
        if !committed.contains_key(&candidate) && !pinned.contains(candidate.as_str()) {
            committed.insert(candidate.clone(), vi);
            return candidate;
        }
    }
}

/// Best-effort serialization of a tree-IR `Expr` to a JS-shaped string.
/// Used only for non-identifier capture operands rendered into an inner
/// closure body's `Operand::Capture` resolution. This is a last-resort
/// fallback; a more rigorous approach would extract the capture into a
/// `let` before the FuncLiteral.
fn render_expr_to_string(e: &Expr) -> String {
    use ir::tree::{binop_str, unop_str};
    match e {
        Expr::Number(n) => format!("{n}"),
        Expr::String(s) => format!("{s:?}"),
        Expr::Bool(b) => format!("{b}"),
        Expr::Null => "null".to_string(),
        Expr::Undefined => "undefined".to_string(),
        Expr::Ident(n) => n.clone(),
        Expr::This => "this".to_string(),
        Expr::RegExp(s) => s.clone(),
        Expr::Binary { op, lhs, rhs } => format!(
            "({}{}{})",
            render_expr_to_string(lhs),
            binop_str(*op),
            render_expr_to_string(rhs)
        ),
        Expr::Unary { op, operand } => {
            format!("({}{})", unop_str(*op), render_expr_to_string(operand))
        }
        Expr::Member { obj, prop } => format!("{}.{}", render_expr_to_string(obj), prop),
        Expr::Index { obj, index } => {
            format!(
                "{}[{}]",
                render_expr_to_string(obj),
                render_expr_to_string(index)
            )
        }
        Expr::Call { callee, args } => format!(
            "{}({})",
            render_expr_to_string(callee),
            args.iter()
                .map(render_expr_to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expr::Func(_) => "/*func*/".to_string(),
        Expr::Array { elements } => format!(
            "[{}]",
            elements
                .iter()
                .map(render_expr_to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expr::Object { props } => {
            let parts: Vec<String> = props
                .iter()
                .map(|p| {
                    match &p.key {
                        ir::tree::ObjectKey::Spread => {
                            format!("...{}", render_expr_to_string(&p.value))
                        }
                        ir::tree::ObjectKey::Static(s) => {
                            format!("\"{s}\":{}", render_expr_to_string(&p.value))
                        }
                        ir::tree::ObjectKey::Computed(e) => {
                            format!("[{}]:{}", render_expr_to_string(e), render_expr_to_string(&p.value))
                        }
                    }
                })
                .collect();
            format!("{{{}}}", parts.join(","))
        }
        Expr::New { callee, args } => format!(
            "new {}({})",
            render_expr_to_string(callee),
            args.iter()
                .map(render_expr_to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => format!(
            "({}?{}:{})",
            render_expr_to_string(cond),
            render_expr_to_string(then_branch),
            render_expr_to_string(else_branch)
        ),
        Expr::Update { op, target, prefix } => {
            let s = match op {
                ir::tree::UpdateOp::Inc => "++",
                ir::tree::UpdateOp::Dec => "--",
            };
            if *prefix {
                format!("({}{})", s, render_expr_to_string(target))
            } else {
                format!("({}{})", render_expr_to_string(target), s)
            }
        }
        Expr::Spread(inner) => format!("...{}", render_expr_to_string(inner)),
        Expr::Sequence { exprs } => format!(
            "({})",
            exprs
                .iter()
                .map(render_expr_to_string)
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expr::Assign { target, value } => format!(
            "({}={})",
            render_expr_to_string(target),
            render_expr_to_string(value)
        ),
        Expr::CompoundAssign { op, target, value } => format!(
            "({}{}={})",
            render_expr_to_string(target),
            binop_str(*op),
            render_expr_to_string(value)
        ),
        Expr::Template { quasis, exprs } => {
            let mut out = String::from("`");
            let mut q_iter = quasis.iter();
            let mut e_iter = exprs.iter();
            if let Some(q) = q_iter.next() {
                out.push_str(q);
            }
            while let Some(expr) = e_iter.next() {
                out.push_str("${");
                out.push_str(&render_expr_to_string(expr));
                out.push('}');
                if let Some(q) = q_iter.next() {
                    out.push_str(q);
                }
            }
            out.push('`');
            out
        }
    }
}

/// Conservative check: true iff `s` is a syntactically valid JS
/// identifier (ASCII identifier-start followed by identifier-continue,
/// not a reserved word). Used at emit time to decide between
/// `obj.key` and `obj["key"]` for arbitrary string keys interned
/// through StoreProp/LoadProp — keys whose source form is a numeric
/// literal or string literal containing non-ident characters would
/// otherwise round-trip as syntactically invalid JS.
fn is_valid_js_ident(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first == '_' || first == '$' || first.is_ascii_alphabetic()) {
        return false;
    }
    for c in chars {
        if !(c == '_' || c == '$' || c.is_ascii_alphanumeric()) {
            return false;
        }
    }
    // Conservative: don't bother filtering reserved words — using
    // `obj["if"]` for a key named "if" is always safe even though
    // `obj.if` is also legal in modern JS.
    !matches!(
        s,
        "true" | "false" | "null" | "undefined" | "this" | "function" | "return"
    )
}

fn find_value_for_inst(func: &Function, iid: InstId) -> Value {
    for (i, def) in func.lowered().values.iter().enumerate() {
        if let ValueDefKind::Inst(j) = &def.kind
            && *j == iid
        {
            return Value(i as u32);
        }
    }
    panic!("instruction {iid:?} has no value-def");
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use emit::emit;
    use crate::lower_ssa;
    use crate::parse_and_lower;

    /// Full pipeline: source → tree-IR → SSA-IR → tree-IR → emit. Returns
    /// the final emitted JS string. Asserts that the resulting JS re-parses
    /// (a minimum well-formedness check).
    fn full_roundtrip(src: &str) -> String {
        let tm = parse_and_lower(src).expect("frontend lowering must succeed");
        let sm = lower_ssa::lower_module(&tm);
        sm.validate()
            .unwrap_or_else(|errs| panic!("SSA module failed validation: {errs:#?}"));
        let tm2 = lower_module(&sm);
        let out = emit(&tm2);
        // Re-parse the emitted JS. This is a structural sanity check; it
        // doesn't prove behavior preservation but catches gross syntax bugs.
        parse_and_lower(&out).unwrap_or_else(|e| {
            panic!("emitted JS failed to re-parse: {e}\n--- emitted ---\n{out}\n--- end ---")
        });
        out
    }

    #[test]
    fn fibonacci_roundtrip() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let out = full_roundtrip(src);
        // Spot-check: function name preserved (top-level decl), recursive
        // call appears as identifier reference.
        assert!(out.contains("function fibonacci"));
        assert!(
            out.contains("fibonacci("),
            "expected at least one call to fibonacci, got:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn sum_to_loop_roundtrip() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("function sum_to"));
        assert!(out.contains("while"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn if_else_mutation_roundtrip() {
        let src = r#"
            function f(c) {
                let x = 1;
                if (c) { x = 2; } else { x = 3; }
                return x;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("function f"));
        assert!(out.contains("if"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn lexical_shadowing_roundtrip() {
        let src = r#"
            function f() {
                let x = 1;
                if (true) { let x = 2; }
                return x;
            }
        "#;
        let out = full_roundtrip(src);
        // The outer x = 1 is unchanged; SSA's trivial-param removal
        // collapses the join into `ret 1`. Out-of-SSA must preserve that —
        // the function should return 1.
        assert!(
            out.contains("return 1"),
            "expected literal 'return 1' in output:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn early_return_in_branch_roundtrip() {
        let src = r#"
            function f(c) {
                if (c) return 1;
                return 2;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("return 1"));
        assert!(out.contains("return 2"));
        insta::assert_snapshot!(out);
    }

    /// Idempotence (weaker than behavior preservation, doesn't need node):
    /// running the pipeline twice should produce the same output as once.
    /// SSA conversion is deterministic given the tree-IR, and out-of-SSA
    /// is deterministic given the SSA. So once the pipeline reaches a
    /// fixed point (no more renames happen), it stays there.
    #[test]
    fn pipeline_idempotent_on_fibonacci() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let once = full_roundtrip(src);
        let twice = full_roundtrip(&once);
        assert_eq!(once, twice, "pipeline should be idempotent after one pass");
    }

    #[test]
    fn pipeline_idempotent_on_sum_to() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let once = full_roundtrip(src);
        let twice = full_roundtrip(&once);
        assert_eq!(once, twice, "pipeline should be idempotent after one pass");
    }

    /// SSA validator must accept the pre-out-of-SSA module for every test
    /// input. This is implicit in `full_roundtrip`'s `sm.validate()` call,
    /// but a dedicated test makes the contract explicit.
    #[test]
    fn ssa_module_validates_for_all_test_inputs() {
        let inputs = [
            "function f(n) { if (n <= 1) return n; return f(n-1) + f(n-2); }",
            "function f(n) { let i=0; while (i<n) { i=i+1; } return i; }",
            "function f(c) { let x=1; if (c) { x=2; } else { x=3; } return x; }",
        ];
        for src in inputs {
            let tm = parse_and_lower(src).unwrap();
            let sm = lower_ssa::lower_module(&tm);
            sm.validate()
                .unwrap_or_else(|errs| panic!("SSA validation failed for {src}: {errs:#?}"));
        }
    }

    // ====================================================================
    // Globals model — round-trips. Each test confirms the unbound-ident
    // lowering survives the full pipeline and re-emits as JS that re-parses.
    // ====================================================================

    /// The current out-of-SSA pass emits each SSA value as its own
    /// `_tN = ...;` statement; expression chaining like `console.log("hi")`
    /// only re-fuses once an inliner runs (a future pass). We therefore
    /// assert on the parts — `console` and `.log` and the string arg —
    /// that the IR reliably preserves. The snapshot pins the full shape.
    #[test]
    fn console_log_roundtrip() {
        let src = r#"function f() { console.log("hi"); }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("console"), "missing `console`:\n{out}");
        assert!(out.contains(".log"), "missing `.log`:\n{out}");
        assert!(out.contains("\"hi\""), "missing `\"hi\"`:\n{out}");
        assert!(
            !out.contains("globalThis"),
            "GlobalThis indirection should be invisible in emit:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn math_floor_roundtrip() {
        let src = r#"function f(x) { return Math.floor(x); }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("Math"), "missing `Math`:\n{out}");
        assert!(out.contains(".floor"), "missing `.floor`:\n{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn unbound_write_roundtrip() {
        let src = r#"function f() { x = 1; }"#;
        let out = full_roundtrip(src);
        // The store re-emits as a bare assignment (`x = 1;`), not
        // `globalThis.x = 1;`.
        assert!(out.contains("x = 1"), "expected `x = 1` in output:\n{out}");
        assert!(
            !out.contains("globalThis"),
            "GlobalThis indirection should be invisible in emit:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    /// `globalThis.foo` collapses to bare `foo` after the round-trip — the
    /// SSA already merges them (verified in `lower_ssa::tests::
    /// globalthis_dot_foo_normalizes_to_bare_foo`); this test confirms the
    /// out-of-SSA pass picks the bare-identifier surface form. The literal
    /// `globalThis` substring must NOT survive — that's the whole point of
    /// the lossy normalization.
    #[test]
    fn globalthis_dot_foo_emits_as_bare_foo() {
        let src = r#"function f() { return globalThis.foo; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("foo"), "expected `foo` in output:\n{out}");
        assert!(
            !out.contains("globalThis"),
            "explicit `globalThis` indirection should be erased:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn lexical_console_shadowing_roundtrip() {
        let src = r#"function f() { let console = 1; return console; }"#;
        let out = full_roundtrip(src);
        // The local binding shadows the global, so the return collapses to
        // `return 1` after trivial-param removal — no `console` ident,
        // no global lookup.
        assert!(
            out.contains("return 1"),
            "shadowed `console` should fold to `return 1`:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    // ====================================================================
    // Behavior-preservation tests (feature-gated on `node-tests`).
    //
    // These shell out to `node` to evaluate both the original source and
    // the round-tripped source on a battery of inputs, asserting equal
    // outputs. They're off by default so CI runs without a Node runtime;
    // enable with `cargo test --features node-tests`.
    // ====================================================================

    #[cfg(feature = "node-tests")]
    fn node_eval(src: &str, harness: &str) -> String {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let program = format!("{src}\n{harness}\n");
        let mut child = Command::new("node")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn node — install Node or disable the node-tests feature");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(program.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        if !out.status.success() {
            panic!(
                "node failed:\nstderr: {}\nprogram:\n{}",
                String::from_utf8_lossy(&out.stderr),
                program
            );
        }
        String::from_utf8(out.stdout).unwrap()
    }

    #[cfg(feature = "node-tests")]
    fn assert_behavior_equivalent(src: &str, harness: &str) {
        let original = node_eval(src, harness);
        let roundtripped_src = full_roundtrip(src);
        let roundtripped = node_eval(&roundtripped_src, harness);
        assert_eq!(
            original, roundtripped,
            "behavior diverged after round-trip\n\
             ===== original src =====\n{src}\n\
             ===== roundtripped src =====\n{roundtripped_src}\n\
             ===== original output =====\n{original}\
             ===== roundtripped output =====\n{roundtripped}"
        );
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn fibonacci_behavior_preserved() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let harness = "for (let i = 0; i <= 12; i++) console.log(fibonacci(i));";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn sum_to_behavior_preserved() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let harness = "for (let i = 0; i <= 20; i++) console.log(sum_to(i));";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn if_else_behavior_preserved() {
        let src = r#"
            function f(c) {
                let x = 1;
                if (c) { x = 2; } else { x = 3; }
                return x;
            }
        "#;
        let harness =
            "console.log(f(true)); console.log(f(false)); console.log(f(0)); console.log(f(1));";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn lexical_shadowing_behavior_preserved() {
        let src = r#"
            function f() {
                let x = 1;
                if (true) { let x = 2; }
                return x;
            }
        "#;
        let harness = "console.log(f());";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn early_return_behavior_preserved() {
        let src = r#"
            function f(c) {
                if (c) return 1;
                return 2;
            }
        "#;
        let harness = "console.log(f(true)); console.log(f(false)); console.log(f(0)); console.log(f(\"x\"));";
        assert_behavior_equivalent(src, harness);
    }

    // =====================================================================
    // T1: name-preservation acceptance tests
    // =====================================================================

    /// Two distinct named lets in the same scope retain their original
    /// names. Non-trivial inits used so the SSA actually creates a Value
    /// for each binding (constant-init lets fold away in lowering — the
    /// name has nothing to attach to).
    #[test]
    fn distinct_named_lets_preserve_names() {
        let src = r#"function f(a) { let x = a + 1; let y = a + 2; return x + y; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("let x"), "expected `let x`:\n{out}");
        assert!(out.contains("let y"), "expected `let y`:\n{out}");
    }

    /// Function parameters emit by their original source names.
    #[test]
    fn parameter_names_preserved() {
        let src = r#"function f(a, b) { return a + b; }"#;
        let out = full_roundtrip(src);
        assert!(
            out.contains("function f(a, b)"),
            "expected `function f(a, b)`:\n{out}"
        );
        assert!(!out.contains("_a0"));
        assert!(!out.contains("_a1"));
    }

    /// Reassignment becomes a fresh `let` with a numeric suffix. This is
    /// the documented T1 gap: the SSA shape produces two distinct values
    /// both annotated with the source name, and the renamer suffix-
    /// disambiguates the second. A future ticket will recover the
    /// original mutation pattern.
    ///
    /// Non-trivial RHS so the SSA produces an Inst for each assignment
    /// (constants fold to `Operand::Const` and have nothing to tag).
    #[test]
    fn reassignment_becomes_fresh_let() {
        let src = r#"function f(a) { let x = a + 1; x = a + 2; return x; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("let x"), "expected `let x`:\n{out}");
        assert!(
            out.contains("let x_1"),
            "expected `let x_1` for the reassignment:\n{out}"
        );
        assert!(
            out.contains("return x_1"),
            "expected `return x_1` (the latest value):\n{out}"
        );
    }

    /// Lexical shadowing: outer x and inner x get x and x_1 under the
    /// conservative function-flat scope. JS would allow both to be `x`
    /// under block scoping, but the renamer treats the entire function
    /// body as one namespace for collision detection. See `assign_names`
    /// for the rationale.
    ///
    /// Non-trivial inits, plus a use of the inner x in the same branch,
    /// so SSA actually materializes both bindings as Values.
    #[test]
    fn lexical_shadow_inner_gets_suffix() {
        let src = r#"
            function f(a, c) {
                let x = a + 1;
                let r = 0;
                if (c) {
                    let x = a + 2;
                    r = x;
                }
                return x + r;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("let x"), "outer x missing:\n{out}");
        // The inner let gets x_1 — even though JS scoping would allow
        // both to be `x` in distinct blocks. Conservative simplification.
        assert!(
            out.contains("let x_1"),
            "inner shadowing x should emit as x_1:\n{out}"
        );
    }

    /// Synthetic intermediates remain `_tN`. We do not invent names for
    /// expression sub-results.
    #[test]
    fn complex_expr_intermediates_emit_as_t() {
        let src = r#"function f(a, b, c, d) { let r = (a + b) * (c - d); return r; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("let r"), "expected `let r`:\n{out}");
        // The (a+b) and (c-d) sub-expressions have no source names.
        assert!(
            out.contains("_t0") || out.contains("_t1"),
            "expected synthesized intermediates `_tN`:\n{out}"
        );
    }

    // =====================================================================
    // EmitConfig — pinned-name behavior. Direct unit tests against the
    // renamer rather than via the full pipeline.
    // =====================================================================

    /// A pinned Value gets exactly its required name. A second Value
    /// preferring the same source name is suffix-disambiguated, even
    /// though it would otherwise win the bare slot by walk order.
    #[test]
    fn emit_config_pin_routes_other_value_around() {
        use ir::ssa::{
            Constant, FunctionBuilder, InstKind, Module as SsaModule, Operand, Terminator,
        };
        let mut module = SsaModule::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v_a = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(1)),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        let v_b = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(3)),
                rhs: Operand::Const(Constant::Int32(4)),
            })
            .unwrap();
        let y_id = b.intern("y");
        b.set_source_name(v_a, y_id);
        b.set_source_name(v_b, y_id);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(v_b)),
        });
        b.finish();

        // Pin v_b to "y". The earlier-walked v_a — which prefers "y"
        // via source_name — must be suffix-disambiguated to "y_1".
        let mut cfg = EmitConfig::default();
        cfg.immovable_names.insert(v_b.0, "y".to_string());
        let tm_out = lower_module_with_config(&module, &cfg);
        let out = emit::emit(&tm_out);
        assert!(
            out.contains("let y_1"),
            "v_a should be routed to y_1 because v_b is pinned:\n{out}"
        );
        assert!(
            out.contains("let y"),
            "v_b should still get exactly `y`:\n{out}"
        );
    }

    /// Pinning two distinct Values to the same name is a caller bug.
    #[test]
    #[should_panic(expected = "two distinct Values cannot share an immovable name")]
    fn emit_config_pin_conflict_panics() {
        use ir::ssa::{
            Constant, FunctionBuilder, InstKind, Module as SsaModule, Operand, Terminator,
        };
        let mut module = SsaModule::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v_a = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(1)),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        let v_b = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(3)),
                rhs: Operand::Const(Constant::Int32(4)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(v_b)),
        });
        b.finish();

        let mut cfg = EmitConfig::default();
        cfg.immovable_names.insert(v_a.0, "y".to_string());
        cfg.immovable_names.insert(v_b.0, "y".to_string());
        let _ = lower_module_with_config(&module, &cfg);
    }

    /// Regression for the `loop_info` / `dominates` phantom back-edge
    /// bug. Original failure: a function with `this` whose CFG ended up
    /// with a stale predecessor pointing at an unreachable block.
    /// `dominates(b, p)` returned `true` for unreachable `p` (the BFS
    /// from entry never reached `p` regardless of blocking through
    /// `b`), which then registered as a phantom back-edge in
    /// `compute_loop_info`, panicking when neither of the alleged
    /// header's cbr branches reached the header. Fixed by adding a
    /// reachability precondition to `dominates`.
    ///
    /// The exact CFG shape that triggered this lives in
    /// `corpus/realworld/tier3/lodash_cloneDeep.high.js`; the
    /// `debug_clonedeep_high` test in `realworld_stability.rs` is the
    /// integration-level regression. This unit test is a small,
    /// focused source-level check that `this`-using functions go
    /// through the full pipeline without panicking — the smallest
    /// correctness contract the fix establishes.
    #[test]
    fn this_in_loop_does_not_panic_loop_info() {
        // Method-style body: `this` reference, conditional branch,
        // loop back-edge from an inner `if/continue`. Roughly the
        // shape that exercises the dominance check in
        // `compute_loop_info`.
        let src = r#"
            function f() {
                let i = 0;
                while (i < 10) {
                    if (this.skip(i)) {
                        i = i + 1;
                        continue;
                    }
                    this.handle(i);
                    i = i + 1;
                }
                return this.result;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("this"), "this must survive: {out}");
    }

    /// Regression for `build_top_level_layout`: when a top-level
    /// function contains an inner function literal, the second
    /// top-level function gets dropped from the output.
    ///
    /// Root cause: `discover_module` assigns FuncIds via
    /// `pending.len()`, which counts top-level + inner literals
    /// interleaved (top-level first, then inner literals discovered
    /// inside its body, then the next top-level). So the second
    /// top-level function's FuncId is `1 + (count of inner literals in
    /// the first)`, not `1`. But `build_top_level_layout` uses a
    /// sequential counter `0, 1, 2, ...` that only counts top-level
    /// items. The `TopLevelEntry::Function(FuncId(1))` it records for
    /// the second top-level function actually points at an inner
    /// literal — which `lower_module_inner` recognizes via
    /// `inner_targets` and silently skips, dropping the top-level
    /// item entirely.
    ///
    /// This bug doesn't bite the corpus because most corpus files
    /// have a single top-level function (the IIFE wrapper). Surfaces
    /// only with 2+ top-level functions where the first has nested
    /// literals.
    #[test]
    fn second_top_level_function_after_inner_literal_is_not_dropped() {
        let src = r#"
            function first(x) {
                let inner = function (y) { return y + 1; };
                return inner(x);
            }
            function second(z) {
                return z * 2;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(
            out.contains("function first"),
            "first must emit:\n{out}"
        );
        assert!(
            out.contains("function second"),
            "second was silently dropped (build_top_level_layout bug):\n{out}"
        );
    }

    /// `-0` must survive the roundtrip as negative zero, not be silently
    /// flipped to `+0`.  The decomposition `(-0.0) - x` only preserves the
    /// sign if the lhs constant is -0.0 (IEEE 754: -0.0 - 0.0 == -0.0, but
    /// 0.0 - 0.0 == +0.0).
    #[test]
    fn neg_zero_roundtrip() {
        let src = r#"function f() { return -0; }"#;
        let out = full_roundtrip(src);
        // The emitted JS must contain the -0 sentinel so that it evaluates to
        // negative zero (Object.is(-0, result) would hold at runtime).
        assert!(
            out.contains("(-0)"),
            "negative zero must survive roundtrip as (-0), got:\n{out}"
        );
        assert!(
            !out.contains("return 0"),
            "plain `return 0` would silently produce +0:\n{out}"
        );
    }
}
