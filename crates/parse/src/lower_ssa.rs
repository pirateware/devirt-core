//! Tree-IR → SSA-IR conversion using the Braun et al. 2013 algorithm
//! ("Simple and Efficient Construction of Static Single Assignment Form").
//!
//! The converter walks the tree-IR top-down, threading a per-function
//! `Converter` that owns the Braun bookkeeping (`current_def`,
//! `incomplete_params`, `sealed`) and a lexical scope stack mapping JS
//! identifiers to `SymbolId`s. All Braun state is keyed by `SymbolId`, never
//! by name, so lexical shadowing falls out for free.
//!
//! Trivial-param removal is mandatory (per the paper). Without it, every
//! join block accumulates redundant parameters that downstream passes have
//! to see through.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use ir::free_vars;
use ir::ssa::{
    self, BasicBlock, BlockId, Constant, FuncId, FunctionBuilder, InstKind, Operand, Purity, StrId,
    Terminator, TopLevelEntry, Value, ValueDef, ValueDefKind,
};
use ir::tree::{self, BinOp, Expr, FuncBody, FuncLiteral, Stmt, UnOp, UpdateOp};

// ============================================================================
// Public entry points
// ============================================================================

pub fn lower_module(tm: &tree::Module) -> ssa::Module {
    // Pass 1: discover every function-shaped node — top-level
    // `Item::Function` / `Item::PassThroughFunction`, plus inner
    // `Stmt::FuncDecl` / `Expr::Func` reachable from a Lowered body.
    // Each gets a FuncId assigned in DFS pre-order; nested literals
    // also have their FuncId stamped on the `Cell` of their
    // `FuncLiteral` so pass 2's outer build can look them up cheaply
    // when emitting `AllocClosure`.
    //
    // Captures are computed in post-order (innermost first) so the
    // free-var analysis can absorb nested literals' captures into the
    // enclosing function's own list — `A` must capture every name
    // captured by a nested `B` that `A` itself does not bind.
    let pending = discover_module(tm);

    // `func_names` maps top-level function names to their FuncId so the
    // converter can resolve direct references (recursion, sibling
    // calls) to `Constant::Func(@name)`. Inner names are NOT included
    // — a nested function's identifier resolves via the lexical
    // binding the outer's lowering sets up at the `Stmt::FuncDecl`
    // textual position.
    let mut func_names: HashMap<String, FuncId> = HashMap::new();
    for p in &pending {
        if let PendingKind::TopLevel(f) = &p.kind {
            func_names.insert(f.name.clone(), p.fid);
        } else if let PendingKind::TopLevelPassThrough(lit) = &p.kind
            && let Some(name) = &lit.name
        {
            func_names.insert(name.clone(), p.fid);
        }
    }

    // `inner_captures` is the by-FuncId capture list, used by both the
    // outer's `AllocClosure` emission (to know how many slots and in
    // what order) and the inner's converter (to seed `Operand::Capture(i)`
    // bindings for those names).
    let inner_captures: HashMap<FuncId, Vec<String>> = pending
        .iter()
        .map(|p| (p.fid, p.captures.clone()))
        .collect();

    // FuncIds whose body is PassThrough. Used at AllocClosure emission
    // to detect "capture flowing into a PassThrough body" so the
    // builder can mark the source Value (or capture slot) for pin
    // wiring at SSA→tree time.
    let passthrough_fids: HashSet<FuncId> = pending
        .iter()
        .filter(|p| match &p.kind {
            PendingKind::TopLevelPassThrough(_) => true,
            PendingKind::InnerLiteral(lit) => {
                matches!(lit.body, FuncBody::PassThrough(_)) || lit.force_passthrough.get()
            }
            PendingKind::TopLevel(_) => false,
        })
        .map(|p| p.fid)
        .collect();

    // For each function, the union of capture-name lists of its
    // direct child literals — i.e. "names a nested function (or its
    // PassThrough descendant via free-var bubble-up) needs from this
    // function's scope." Reads of these names are unaffected;
    // *writes* (Stmt::Assign target=Ident) need to materialize as a
    // textual reassignment in the emitted output, even when the
    // value is a constant. Const-to-const assignments dissolve into
    // Braun bookkeeping with no Inst, so a PassThrough body that
    // lexically references the name would otherwise see only the
    // initial value at runtime — the architectural rule from
    // `pt_free_vars`'s module doc: PassThrough is opaque to passes
    // but not to scope analysis, and source-level mutations of
    // PT-referenced names must survive. The fix: at Stmt::Assign,
    // synthesize a `StoreProp(GlobalThis, name, val)` which lower_tree
    // re-emits as `name = val` (lexical resolution then hits the
    // local let-binding).
    let pt_referenced_for_fid: HashMap<FuncId, HashSet<String>> = {
        let mut m: HashMap<FuncId, HashSet<String>> = HashMap::new();
        for p in &pending {
            m.entry(p.fid).or_default();
        }
        for p in &pending {
            if let Some(parent) = p.parent {
                let caps = inner_captures.get(&p.fid).cloned().unwrap_or_default();
                let entry = m.entry(parent).or_default();
                for n in caps {
                    entry.insert(n);
                }
            }
        }
        m
    };

    // Pass 2: build each pending function in FuncId order. Each
    // `FunctionBuilder::finish()` pushes one slot to `module.functions`,
    // so build-order == FuncId-order is mandatory for the indices to
    // line up.
    let mut sm = ssa::Module::new();
    for p in &pending {
        match &p.kind {
            PendingKind::TopLevel(f) => {
                let mut pt_refs = pt_referenced_for_fid
                    .get(&p.fid)
                    .cloned()
                    .unwrap_or_default();
                // Names targeted by an `Expr::Assign` in expression
                // position must materialize as textual stores, not
                // dissolve into Braun bookkeeping — see
                // `collect_assign_expr_target_names` doc.
                pt_refs.extend(collect_assign_expr_target_names(&f.body));
                lower_top_level_function(
                    &mut sm,
                    f,
                    &func_names,
                    &inner_captures,
                    &passthrough_fids,
                    pt_refs,
                );
            }
            PendingKind::TopLevelPassThrough(lit) => {
                push_passthrough_function(&mut sm, lit);
            }
            PendingKind::InnerLiteral(lit) => {
                let mut pt_refs = pt_referenced_for_fid
                    .get(&p.fid)
                    .cloned()
                    .unwrap_or_default();
                if let FuncBody::Stmts(ss) = &lit.body {
                    pt_refs.extend(collect_assign_expr_target_names(ss));
                }
                lower_inner_literal(
                    &mut sm,
                    lit,
                    &p.captures,
                    &func_names,
                    &inner_captures,
                    &passthrough_fids,
                    pt_refs,
                );
            }
        }
    }

    // Top-level layout: walk `tm.items` in source order, assigning
    // FuncId references to top-level function items in the same order
    // `discover_module` did, and copying PassThrough source slices for
    // non-function statements. Mirrors source order so emit reproduces
    // the original interleaving.
    sm.top_level_layout = build_top_level_layout(tm);

    // Fixed-point propagation: for every Lowered function whose
    // `captures_pass_through_onward` set is non-empty, walk every
    // AllocClosure in any other function targeting this function and
    // propagate the flagged capture slots back into the calling
    // function's captured-by-passthrough or onward set. Iterate until
    // no change.
    propagate_passthrough_captures(&mut sm);

    sm
}

fn propagate_passthrough_captures(sm: &mut ssa::Module) {
    loop {
        let mut changed = false;
        // Snapshot onward sets so we can read while writing into other
        // functions.
        let onward: Vec<Vec<usize>> = sm
            .functions
            .iter()
            .map(|f| f.captures_pass_through_onward.clone())
            .collect();

        for caller_idx in 0..sm.functions.len() {
            // Collect (target_fid, captures-clone) pairs from this
            // caller's AllocClosure instructions. Have to clone the
            // vector to avoid borrowing the function while we mutate
            // it below.
            let allocs: Vec<(FuncId, Vec<Operand>)> = match &sm.functions[caller_idx].body {
                ssa::FunctionBody::Lowered(b) => b
                    .instructions
                    .iter()
                    .filter_map(|inst| match &inst.kind {
                        InstKind::AllocClosure { func, captures } => {
                            Some((*func, captures.clone()))
                        }
                        _ => None,
                    })
                    .collect(),
                ssa::FunctionBody::PassThrough(_) => continue,
            };

            for (target_fid, captures) in allocs {
                let target_onward = &onward[target_fid.0 as usize];
                if target_onward.is_empty() {
                    continue;
                }
                for &slot in target_onward {
                    if slot >= captures.len() {
                        continue;
                    }
                    match captures[slot] {
                        Operand::Value(v) => {
                            let cap = &mut sm.functions[caller_idx].captured_by_passthrough;
                            if !cap.contains(&v.0) {
                                cap.push(v.0);
                                changed = true;
                            }
                        }
                        Operand::Capture(idx) => {
                            let onw = &mut sm.functions[caller_idx]
                                .captures_pass_through_onward;
                            if !onw.contains(&idx) {
                                onw.push(idx);
                                changed = true;
                            }
                        }
                        Operand::Const(_) => {}
                    }
                }
            }
        }

        if !changed {
            break;
        }
    }

    // Post-pass constraint check (mirrors the one in
    // FunctionBuilder::finish, but propagation can introduce new
    // captured-by-passthrough Value indices after the builder has
    // already finished).
    for func in &sm.functions {
        if let ssa::FunctionBody::Lowered(body) = &func.body {
            for &vi in &func.captured_by_passthrough {
                if body.values[vi as usize].source_name.is_none() {
                    panic!(
                        "Value %{vi} in function `{}` is captured into a PassThrough body \
                         but has no source_name. A synthesized intermediate cannot be \
                         captured by a PassThrough body — the lowering must capture only \
                         named source-level bindings.",
                        func.name
                    );
                }
            }
        }
    }
}

// ----------------------------------------------------------------------
// Pass 1: discovery
// ----------------------------------------------------------------------

/// One per function-shaped node in the source. Indexed by FuncId
/// (i.e. `pending[fid.0 as usize].fid == fid`).
struct PendingFunction<'a> {
    fid: FuncId,
    kind: PendingKind<'a>,
    /// Free-var names this function captures from its enclosing scope,
    /// alphabetically sorted. Top-level functions always have an empty
    /// list (they have no enclosing JS scope).
    captures: Vec<String>,
    /// FuncId of the lexically-enclosing function. `None` for top-level
    /// items (which have no JS-level outer). Used to compute
    /// `pt_referenced_for_fid` — the union of children's captures
    /// per parent.
    parent: Option<FuncId>,
}

enum PendingKind<'a> {
    TopLevel(&'a tree::IrFunction),
    TopLevelPassThrough(&'a tree::FuncLiteral),
    InnerLiteral(&'a tree::FuncLiteral),
}

/// Walk `tm.items` in source order and produce the layout that the
/// out-of-SSA emitter will use to interleave functions with passthrough
/// statements. FuncId assignment must match `discover_module`: top-level
/// functions get FuncId(0), FuncId(1), ... in source order — the same
/// order they were pushed onto `pending`.
fn build_top_level_layout(tm: &tree::Module) -> Vec<TopLevelEntry> {
    let mut out = Vec::with_capacity(tm.items.len());
    let mut next_top_fid: u32 = 0;
    for it in &tm.items {
        match it {
            tree::Item::Function(_) | tree::Item::PassThroughFunction(_) => {
                out.push(TopLevelEntry::Function(FuncId(next_top_fid)));
                next_top_fid += 1;
            }
            tree::Item::PassThroughStmt(src) => {
                out.push(TopLevelEntry::PassThroughStmt(src.clone()));
            }
            tree::Item::Stmt(_) => {
                // Hand-built tests may construct Item::Stmt directly;
                // the OXC frontend never does. We have no source slice
                // for these, so skip — the caller is expected to set
                // `top_level_layout` themselves if they need a custom
                // layout. Functions-only emission (the historical
                // behavior) handles this case naturally.
            }
        }
    }
    out
}

fn discover_module<'a>(tm: &'a tree::Module) -> Vec<PendingFunction<'a>> {
    let mut pending: Vec<PendingFunction<'a>> = Vec::new();
    let mut captures_by_fid: HashMap<FuncId, Vec<String>> = HashMap::new();
    for it in &tm.items {
        match it {
            tree::Item::Function(f) => {
                let fid = FuncId(pending.len() as u32);
                pending.push(PendingFunction {
                    fid,
                    kind: PendingKind::TopLevel(f),
                    captures: Vec::new(),
                    parent: None,
                });
                // If this function is going to take the SSA-fallback
                // path (body contains Try/ForIn/Throw/This), its body
                // is preserved as a verbatim source slice and inner
                // literals inside it stay opaque — just like the
                // already-PassThrough case below. Skip body recursion
                // so we don't allocate FuncIds for inner literals
                // we'll never emit, which would also misalign
                // top-level FuncIds.
                if body_needs_ssa_fallback(&f.body) {
                    captures_by_fid.insert(fid, Vec::new());
                    continue;
                }
                // Build outer-body context: reassigned-name set + an
                // empty bindings-seen-so-far set (params and the
                // function's own name are bound at body entry).
                let reassigned = collect_reassigned_names(&f.body);
                let mut seen: HashSet<String> = HashSet::new();
                seen.insert(f.name.clone());
                for p in &f.params {
                    seen.insert(p.clone());
                }
                // Hoist nested FuncDecl names so a textual position
                // before the decl can still reference the function name
                // without it counting as forward-ref.
                hoist_funcdecl_names(&f.body, &mut seen);
                discover_stmts(
                    &f.body,
                    &mut pending,
                    &mut captures_by_fid,
                    &mut seen,
                    &reassigned,
                    fid,
                );
                // Top-level functions cannot capture (no enclosing JS
                // scope). The empty captures list is the load-bearing
                // statement here, not an oversight.
                captures_by_fid.insert(fid, Vec::new());
            }
            tree::Item::PassThroughFunction(lit) => {
                let fid = FuncId(pending.len() as u32);
                lit.func_id.set(Some(fid));
                pending.push(PendingFunction {
                    fid,
                    kind: PendingKind::TopLevelPassThrough(lit),
                    captures: Vec::new(),
                    parent: None,
                });
                captures_by_fid.insert(fid, Vec::new());
            }
            tree::Item::Stmt(_) | tree::Item::PassThroughStmt(_) => {
                // Top-level non-function statements have no per-function
                // SSA representation. The source-order layout is
                // reconstructed by `build_top_level_layout` after
                // discovery; here we just skip them.
            }
        }
    }
    // Apply captures back into the pending list.
    for p in pending.iter_mut() {
        if let Some(c) = captures_by_fid.get(&p.fid) {
            p.captures = c.clone();
        }
    }
    pending
}

/// Names that appear as the LHS of an `Stmt::Assign` anywhere in
/// `stmts`, including in nested control-flow blocks. Walks the body
/// stopping at nested function literals — those have their own scope.
fn collect_reassigned_names(stmts: &[Stmt]) -> HashSet<String> {
    let mut out = HashSet::new();
    walk_reassigned_stmts(stmts, &mut out);
    out
}

fn walk_reassigned_stmts(stmts: &[Stmt], out: &mut HashSet<String>) {
    for s in stmts {
        match s {
            Stmt::Assign {
                target: Expr::Ident(n),
                ..
            } => {
                out.insert(n.clone());
            }
            Stmt::Assign { .. } => {}
            Stmt::Let { .. } | Stmt::Const { .. } | Stmt::ExprStmt(_) | Stmt::Return(_) => {}
            Stmt::Throw(_) => {}
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                walk_reassigned_stmts(then_body, out);
                if let Some(eb) = else_body {
                    walk_reassigned_stmts(eb, out);
                }
            }
            Stmt::While { body, .. } => walk_reassigned_stmts(body, out),
            Stmt::For { init, body, step, .. } => {
                if let Some(init) = init {
                    walk_reassigned_stmts(std::slice::from_ref(init.as_ref()), out);
                }
                walk_reassigned_stmts(body, out);
                // Step is an Expr (typically `i++` / `i = i+1`); update
                // expressions to identifier targets are reassignments.
                if let Some(step) = step {
                    walk_reassigned_in_expr(step, out);
                }
            }
            Stmt::ForOf { body, .. } => walk_reassigned_stmts(body, out),
            Stmt::ForIn { body, obj, .. } => {
                walk_reassigned_in_expr(obj, out);
                walk_reassigned_stmts(body, out);
            }
            Stmt::Continue => {}
            Stmt::Block(ss) => walk_reassigned_stmts(ss, out),
            Stmt::FuncDecl(_) => {} // nested literal: own scope
            Stmt::Try { body, catch, finally } => {
                walk_reassigned_stmts(body, out);
                if let Some((_, catch_body)) = catch {
                    walk_reassigned_stmts(catch_body, out);
                }
                if let Some(finally_body) = finally {
                    walk_reassigned_stmts(finally_body, out);
                }
            }
        }
    }
}

/// Identify identifier targets reassigned by an expression's
/// `Expr::Update`. We only track ident updates here (matching the
/// `Stmt::Assign` rule in `walk_reassigned_stmts`); update on
/// `Expr::Member` / `Expr::Index` mutates a property, not a binding.
fn walk_reassigned_in_expr(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Update {
            target, ..
        } => {
            if let Expr::Ident(n) = target.as_ref() {
                out.insert(n.clone());
            }
            walk_reassigned_in_expr(target, out);
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_reassigned_in_expr(lhs, out);
            walk_reassigned_in_expr(rhs, out);
        }
        Expr::Unary { operand, .. } => walk_reassigned_in_expr(operand, out),
        Expr::Call { callee, args } => {
            walk_reassigned_in_expr(callee, out);
            for a in args {
                walk_reassigned_in_expr(a, out);
            }
        }
        Expr::Member { obj, .. } => walk_reassigned_in_expr(obj, out),
        Expr::Index { obj, index } => {
            walk_reassigned_in_expr(obj, out);
            walk_reassigned_in_expr(index, out);
        }
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_reassigned_in_expr(cond, out);
            walk_reassigned_in_expr(then_branch, out);
            walk_reassigned_in_expr(else_branch, out);
        }
        Expr::Array { elements } => {
            for el in elements {
                walk_reassigned_in_expr(el, out);
            }
        }
        Expr::Object { props } => {
            for p in props {
                if let ir::tree::ObjectKey::Computed(e) = &p.key {
                    walk_reassigned_in_expr(e, out);
                }
                walk_reassigned_in_expr(&p.value, out);
            }
        }
        Expr::New { callee, args } => {
            walk_reassigned_in_expr(callee, out);
            for a in args {
                walk_reassigned_in_expr(a, out);
            }
        }
        // Stop at function literals — nested scope.
        Expr::Func(_) => {}
        Expr::Spread(inner) => walk_reassigned_in_expr(inner, out),
        Expr::Template { exprs, .. } => {
            for e in exprs {
                walk_reassigned_in_expr(e, out);
            }
        }
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Null
        | Expr::Undefined
        | Expr::Ident(_)
        | Expr::This => {}
        Expr::Sequence { exprs } => {
            for e in exprs {
                walk_reassigned_in_expr(e, out);
            }
        }
        Expr::Assign { target, value } => {
            // Plain `=` to an identifier is a reassignment; track it
            // the same way `Stmt::Assign { target: Expr::Ident, .. }`
            // does in `walk_reassigned_stmts`.
            if let Expr::Ident(n) = target.as_ref() {
                out.insert(n.clone());
            }
            walk_reassigned_in_expr(target, out);
            walk_reassigned_in_expr(value, out);
        }
        Expr::CompoundAssign { target, value, .. } => {
            // `x op= rhs` writes to `x` (when target is an identifier).
            if let Expr::Ident(n) = target.as_ref() {
                out.insert(n.clone());
            }
            walk_reassigned_in_expr(target, out);
            walk_reassigned_in_expr(value, out);
        }
    }
}

/// Hoist nested FuncDecl names into `seen` (top-level only — does not
/// descend into nested control-flow). JS function declarations are
/// hoisted to the enclosing function scope; this matches our tree-IR
/// shape (nested FuncDecls live as Stmt::FuncDecl directly under the
/// function body).
fn hoist_funcdecl_names(stmts: &[Stmt], seen: &mut HashSet<String>) {
    for s in stmts {
        if let Stmt::FuncDecl(lit) = s
            && let Some(n) = &lit.name
        {
            seen.insert(n.clone());
        }
    }
}

fn discover_stmts<'a>(
    stmts: &'a [Stmt],
    pending: &mut Vec<PendingFunction<'a>>,
    captures_by_fid: &mut HashMap<FuncId, Vec<String>>,
    seen: &mut HashSet<String>,
    reassigned: &HashSet<String>,
    parent_fid: FuncId,
) {
    for s in stmts {
        discover_stmt(s, pending, captures_by_fid, seen, reassigned, parent_fid);
    }
}

fn discover_stmt<'a>(
    s: &'a Stmt,
    pending: &mut Vec<PendingFunction<'a>>,
    captures_by_fid: &mut HashMap<FuncId, Vec<String>>,
    seen: &mut HashSet<String>,
    reassigned: &HashSet<String>,
    parent_fid: FuncId,
) {
    match s {
        Stmt::Let { name, init } => {
            if let Some(e) = init {
                discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
            seen.insert(name.clone());
        }
        Stmt::Const { name, init } => {
            discover_expr(init, pending, captures_by_fid, seen, reassigned, parent_fid);
            seen.insert(name.clone());
        }
        Stmt::Assign { target, value } => {
            discover_expr(target, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(value, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Stmt::ExprStmt(e) => discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid),
        Stmt::Return(e) => {
            if let Some(e) = e {
                discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            discover_expr(cond, pending, captures_by_fid, seen, reassigned, parent_fid);
            // Inner-block bindings don't escape; clone seen so they
            // don't pollute the outer state.
            let mut inner_seen = seen.clone();
            discover_stmts(
                then_body,
                pending,
                captures_by_fid,
                &mut inner_seen,
                reassigned,
                parent_fid,
            );
            if let Some(eb) = else_body {
                let mut inner_seen2 = seen.clone();
                discover_stmts(
                    eb,
                    pending,
                    captures_by_fid,
                    &mut inner_seen2,
                    reassigned,
                    parent_fid,
                );
            }
        }
        Stmt::While { cond, body } => {
            discover_expr(cond, pending, captures_by_fid, seen, reassigned, parent_fid);
            let mut inner_seen = seen.clone();
            discover_stmts(
                body,
                pending,
                captures_by_fid,
                &mut inner_seen,
                reassigned,
                parent_fid,
            );
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            // Init lives in a fresh inner scope; bindings introduced by
            // it (e.g. `let i = 0`) don't escape past the loop.
            let mut inner_seen = seen.clone();
            if let Some(init) = init {
                discover_stmt(
                    init,
                    pending,
                    captures_by_fid,
                    &mut inner_seen,
                    reassigned,
                    parent_fid,
                );
            }
            if let Some(cond) = cond {
                discover_expr(
                    cond,
                    pending,
                    captures_by_fid,
                    &mut inner_seen,
                    reassigned,
                    parent_fid,
                );
            }
            discover_stmts(
                body,
                pending,
                captures_by_fid,
                &mut inner_seen,
                reassigned,
                parent_fid,
            );
            if let Some(step) = step {
                discover_expr(
                    step,
                    pending,
                    captures_by_fid,
                    &mut inner_seen,
                    reassigned,
                    parent_fid,
                );
            }
        }
        Stmt::ForOf {
            iterable,
            body,
            ..
        } => {
            discover_expr(iterable, pending, captures_by_fid, seen, reassigned, parent_fid);
            // The binding variable is introduced in a fresh inner scope;
            // it doesn't affect the outer scope's seen set.
            let mut inner_seen = seen.clone();
            discover_stmts(
                body,
                pending,
                captures_by_fid,
                &mut inner_seen,
                reassigned,
                parent_fid,
            );
        }
        Stmt::ForIn {
            obj,
            body,
            ..
        } => {
            discover_expr(obj, pending, captures_by_fid, seen, reassigned, parent_fid);
            let mut inner_seen = seen.clone();
            discover_stmts(
                body,
                pending,
                captures_by_fid,
                &mut inner_seen,
                reassigned,
                parent_fid,
            );
        }
        Stmt::Continue => {}
        Stmt::Block(ss) => {
            let mut inner_seen = seen.clone();
            discover_stmts(
                ss,
                pending,
                captures_by_fid,
                &mut inner_seen,
                reassigned,
                parent_fid,
            );
        }
        Stmt::FuncDecl(lit) => {
            // Function name is hoisted (already inserted in `seen` by
            // hoist_funcdecl_names).
            discover_literal(lit, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Stmt::Try { body, catch, finally } => {
            let mut inner_seen = seen.clone();
            discover_stmts(body, pending, captures_by_fid, &mut inner_seen, reassigned, parent_fid);
            if let Some((param, catch_body)) = catch {
                let mut catch_seen = seen.clone();
                if let Some(p) = param {
                    catch_seen.insert(p.clone());
                }
                discover_stmts(catch_body, pending, captures_by_fid, &mut catch_seen, reassigned, parent_fid);
            }
            if let Some(finally_body) = finally {
                let mut finally_seen = seen.clone();
                discover_stmts(finally_body, pending, captures_by_fid, &mut finally_seen, reassigned, parent_fid);
            }
        }
        Stmt::Throw(e) => {
            discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
    }
}

fn discover_expr<'a>(
    e: &'a Expr,
    pending: &mut Vec<PendingFunction<'a>>,
    captures_by_fid: &mut HashMap<FuncId, Vec<String>>,
    seen: &mut HashSet<String>,
    reassigned: &HashSet<String>,
    parent_fid: FuncId,
) {
    match e {
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Null
        | Expr::Undefined
        | Expr::Ident(_)
        | Expr::This => {}
        Expr::Binary { lhs, rhs, .. } => {
            discover_expr(lhs, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(rhs, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Expr::Unary { operand, .. } => {
            discover_expr(operand, pending, captures_by_fid, seen, reassigned, parent_fid)
        }
        Expr::Call { callee, args } => {
            discover_expr(callee, pending, captures_by_fid, seen, reassigned, parent_fid);
            for a in args {
                discover_expr(a, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Expr::Member { obj, .. } => discover_expr(obj, pending, captures_by_fid, seen, reassigned, parent_fid),
        Expr::Index { obj, index } => {
            discover_expr(obj, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(index, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Expr::Func(lit) => discover_literal(lit, pending, captures_by_fid, seen, reassigned, parent_fid),
        Expr::Array { elements } => {
            for el in elements {
                discover_expr(el, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Expr::Object { props } => {
            for p in props {
                if let ir::tree::ObjectKey::Computed(e) = &p.key {
                    discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid);
                }
                discover_expr(&p.value, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Expr::New { callee, args } => {
            discover_expr(callee, pending, captures_by_fid, seen, reassigned, parent_fid);
            for a in args {
                discover_expr(a, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            discover_expr(cond, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(then_branch, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(else_branch, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Expr::Update { target, .. } => {
            discover_expr(target, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Expr::Spread(inner) => {
            discover_expr(inner, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Expr::Template { exprs, .. } => {
            for e in exprs {
                discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Expr::Sequence { exprs } => {
            for e in exprs {
                discover_expr(e, pending, captures_by_fid, seen, reassigned, parent_fid);
            }
        }
        Expr::Assign { target, value } => {
            discover_expr(target, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(value, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
        Expr::CompoundAssign { target, value, .. } => {
            discover_expr(target, pending, captures_by_fid, seen, reassigned, parent_fid);
            discover_expr(value, pending, captures_by_fid, seen, reassigned, parent_fid);
        }
    }
}

fn discover_literal<'a>(
    lit: &'a FuncLiteral,
    pending: &mut Vec<PendingFunction<'a>>,
    captures_by_fid: &mut HashMap<FuncId, Vec<String>>,
    outer_seen: &HashSet<String>,
    outer_reassigned: &HashSet<String>,
    parent_fid: FuncId,
) {
    // Pre-order: stamp FuncId immediately so any sibling reference
    // discovered later (or parent's later post-order analysis) can read
    // it from the Cell.
    let fid = FuncId(pending.len() as u32);
    lit.func_id.set(Some(fid));
    pending.push(PendingFunction {
        fid,
        kind: PendingKind::InnerLiteral(lit),
        captures: Vec::new(),
        parent: Some(parent_fid),
    });

    // Recurse into the body to discover deeper nested literals (and to
    // populate their captures before we run free-var analysis on this
    // one). The inner body has its OWN scope: parameters, hoisted
    // FuncDecl names, and a fresh reassigned-set.
    let inner_reassigned = match &lit.body {
        FuncBody::Stmts(ss) => collect_reassigned_names(ss),
        FuncBody::Expression(_) | FuncBody::PassThrough(_) => HashSet::new(),
    };
    let mut inner_seen: HashSet<String> = HashSet::new();
    if let Some(n) = &lit.name {
        inner_seen.insert(n.clone());
    }
    for p in &lit.params {
        inner_seen.insert(p.clone());
    }
    match &lit.body {
        FuncBody::Stmts(ss) => {
            hoist_funcdecl_names(ss, &mut inner_seen);
            discover_stmts(
                ss,
                pending,
                captures_by_fid,
                &mut inner_seen,
                &inner_reassigned,
                fid,
            );
        }
        FuncBody::Expression(e) => {
            discover_expr(
                e,
                pending,
                captures_by_fid,
                &mut inner_seen,
                &inner_reassigned,
                fid,
            );
        }
        // PassThrough bodies are walked separately by the
        // `pt_free_vars` analysis (see free_vars.rs); they cannot host
        // nested literals from this lowering's perspective.
        FuncBody::PassThrough(_) => {}
    }

    // Post-order: now compute this literal's captures using free-var
    // analysis. Inner literals' captures are already stamped in
    // `captures_by_fid`, so the analyzer can absorb them.
    let captures = free_vars::analyze(lit, captures_by_fid);

    // Forward-ref / reassigned-capture rule: if any capture is
    // forward-referenced (not yet in `outer_seen` at this textual
    // position) or appears in `outer_reassigned`, re-classify this
    // literal to PassThrough. The check is per project memory
    // `closures-disposition-rule`.
    let should_passthrough = captures
        .iter()
        .any(|name| !outer_seen.contains(name) || outer_reassigned.contains(name));
    if should_passthrough {
        // Source slice is required for re-classification — without it
        // we'd lose the body. The lower.rs front end always stashes one
        // for nested literals; tests that build tree-IR by hand may
        // omit it, in which case we leave the literal as Lowered (and
        // accept the SSA-level panic the rule was trying to avoid).
        if lit.passthrough_source.is_some() {
            lit.force_passthrough.set(true);
        } else {
            debug_assert!(
                false,
                "forward-ref/reassigned violation on inner literal `{}` but no \
                 passthrough_source is stashed; cannot re-classify",
                lit.name.as_deref().unwrap_or("<anon>")
            );
        }
    }

    captures_by_fid.insert(fid, captures);
}

// ----------------------------------------------------------------------
// Pass 2: build
// ----------------------------------------------------------------------

/// Build an SSA `Function` with `FunctionBody::PassThrough` from a
/// tree-IR `FuncLiteral` whose body is `FuncBody::PassThrough`. No
/// `FunctionBuilder` involved — PassThrough functions have no blocks,
/// values, or instructions.
fn push_passthrough_function(sm: &mut ssa::Module, lit: &tree::FuncLiteral) {
    let source = match &lit.body {
        tree::FuncBody::PassThrough(s) => s.clone(),
        _ => panic!("Item::PassThroughFunction must carry a PassThrough body"),
    };
    let name = lit.name.clone().unwrap_or_default();
    let function_name_id = lit.name.as_ref().map(|n| sm.strings.get_or_intern(n));
    let param_names: Vec<ssa::StrId> = lit
        .params
        .iter()
        .map(|p| sm.strings.get_or_intern(p))
        .collect();
    sm.functions.push(ssa::Function {
        name,
        capture_count: 0,
        captured_by_passthrough: Vec::new(),
        captures_pass_through_onward: Vec::new(),
        forced_let_decls: Vec::new(),
        body: ssa::FunctionBody::PassThrough(ssa::PassThroughBody {
            source,
            param_names,
            function_name: function_name_id,
            is_async: lit.is_async,
            is_generator: lit.is_generator,
            is_arrow: lit.is_arrow,
            body_is_expression: false,
        }),
    });
}

/// True if the body contains a `Stmt::ForIn` or `Stmt::Try` at any
/// depth (not descending into nested function literals — those have
/// their own disposition). These constructs cannot be expressed in
/// SSA form, so we route enclosing functions through the PassThrough
/// path using a stashed source slice. The disposition walker has
/// already classified the function as `Lowered` (no blocker), so the
/// coverage metric still counts the unlock.
fn body_needs_ssa_fallback(stmts: &[Stmt]) -> bool {
    fn visit_stmt(s: &Stmt) -> bool {
        match s {
            Stmt::ForIn { .. } | Stmt::Try { .. } | Stmt::Throw(_) => true,
            Stmt::If { cond, then_body, else_body, .. } => {
                visit_expr(cond)
                    || then_body.iter().any(visit_stmt)
                    || else_body.as_ref().is_some_and(|b| b.iter().any(visit_stmt))
            }
            Stmt::While { cond, body } => {
                visit_expr(cond) || body.iter().any(visit_stmt)
            }
            Stmt::For { body, init, cond, step } => {
                init.as_deref().is_some_and(visit_stmt)
                    || cond.as_ref().is_some_and(visit_expr)
                    || step.as_ref().is_some_and(visit_expr)
                    || body.iter().any(visit_stmt)
            }
            Stmt::ForOf { iterable, body, .. } => {
                visit_expr(iterable) || body.iter().any(visit_stmt)
            }
            Stmt::Block(body) => body.iter().any(visit_stmt),
            // Nested function literals have their own disposition.
            Stmt::FuncDecl(_) => false,
            Stmt::Let { init: Some(e), .. } | Stmt::Const { init: e, .. }
            | Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => visit_expr(e),
            Stmt::Assign { target, value } => visit_expr(target) || visit_expr(value),
            _ => false,
        }
    }
    // `Expr::This` flows through SSA as `Constant::This`, which works
    // for simple uses but exposes a latent CFG-reconstruction bug in
    // `lower_tree::loop_info` on certain back-edge shapes (visible on
    // `lodash_cloneDeep.high.js`). Until that's fixed, route any
    // function whose body references `this` to the same SSA-fallback
    // path used by Try/ForIn/Throw — disposition still classifies the
    // function as Lowered, so coverage counts the unlock.
    fn visit_expr(e: &Expr) -> bool {
        match e {
            Expr::This => true,
            Expr::Binary { lhs, rhs, .. } => visit_expr(lhs) || visit_expr(rhs),
            Expr::Unary { operand, .. } => visit_expr(operand),
            Expr::Call { callee, args } | Expr::New { callee, args } => {
                visit_expr(callee) || args.iter().any(visit_expr)
            }
            Expr::Member { obj, .. } => visit_expr(obj),
            Expr::Index { obj, index } => visit_expr(obj) || visit_expr(index),
            Expr::Ternary { cond, then_branch, else_branch } => {
                visit_expr(cond) || visit_expr(then_branch) || visit_expr(else_branch)
            }
            Expr::Array { elements } => elements.iter().any(visit_expr),
            Expr::Object { props } => props.iter().any(|p| visit_expr(&p.value)),
            Expr::Sequence { exprs } | Expr::Template { exprs, .. } => {
                exprs.iter().any(visit_expr)
            }
            Expr::Assign { target, value } => visit_expr(target) || visit_expr(value),
            Expr::CompoundAssign { target, value, .. } => {
                visit_expr(target) || visit_expr(value)
            }
            Expr::Update { target, .. } => visit_expr(target),
            Expr::Spread(inner) => visit_expr(inner),
            // Nested function literals have their own disposition; their
            // body's `this` is independent of the enclosing function.
            Expr::Func(_) => false,
            _ => false,
        }
    }
    stmts.iter().any(visit_stmt)
}

/// Collect every identifier name that appears as the target of an
/// `Expr::Assign` anywhere in `stmts`, NOT descending into nested
/// function literals. These names need a textual reassignment in the
/// emitted output even when the value would otherwise be inlined as
/// a control-flow condition by the tree reconstruction — same
/// mechanism `pt_referenced_names` uses. Without it, code like
/// `while ((x = foo())) { use(x); }` reconstructs as `while (foo())
/// { use(x); }` with `x` reading an unbound declaration, silently
/// breaking semantics.
fn collect_assign_expr_target_names(stmts: &[Stmt]) -> std::collections::HashSet<String> {
    fn visit_stmt(s: &Stmt, out: &mut std::collections::HashSet<String>) {
        match s {
            Stmt::Let { init: Some(e), .. } | Stmt::Const { init: e, .. }
            | Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => visit_expr(e, out),
            Stmt::Assign { value, target } => {
                // The assignment itself is fine (it produces a real
                // Stmt::Assign in tree-IR). Recurse only into the RHS
                // and into the target's sub-expressions for nested
                // Expr::Assigns.
                visit_expr(target, out);
                visit_expr(value, out);
            }
            Stmt::If { cond, then_body, else_body } => {
                visit_expr(cond, out);
                for s in then_body { visit_stmt(s, out); }
                if let Some(eb) = else_body { for s in eb { visit_stmt(s, out); } }
            }
            Stmt::While { cond, body } => {
                visit_expr(cond, out);
                for s in body { visit_stmt(s, out); }
            }
            Stmt::For { init, cond, step, body } => {
                if let Some(init) = init { visit_stmt(init, out); }
                if let Some(cond) = cond { visit_expr(cond, out); }
                if let Some(step) = step { visit_expr(step, out); }
                for s in body { visit_stmt(s, out); }
            }
            Stmt::ForOf { iterable, body, .. } => {
                visit_expr(iterable, out);
                for s in body { visit_stmt(s, out); }
            }
            Stmt::ForIn { obj, body, .. } => {
                visit_expr(obj, out);
                for s in body { visit_stmt(s, out); }
            }
            Stmt::Block(b) => for s in b { visit_stmt(s, out); },
            // Stop at nested function literals.
            Stmt::FuncDecl(_) => {}
            Stmt::Try { body, catch, finally } => {
                for s in body { visit_stmt(s, out); }
                if let Some((_, cb)) = catch { for s in cb { visit_stmt(s, out); } }
                if let Some(fb) = finally { for s in fb { visit_stmt(s, out); } }
            }
            Stmt::Let { init: None, .. } | Stmt::Return(None) | Stmt::Continue => {}
            Stmt::Throw(e) => visit_expr(e, out),
        }
    }
    fn visit_expr(e: &Expr, out: &mut std::collections::HashSet<String>) {
        match e {
            Expr::Assign { target, value } => {
                if let Expr::Ident(n) = target.as_ref() {
                    out.insert(n.clone());
                }
                visit_expr(target, out);
                visit_expr(value, out);
            }
            Expr::CompoundAssign { target, value, .. } => {
                if let Expr::Ident(n) = target.as_ref() {
                    out.insert(n.clone());
                }
                visit_expr(target, out);
                visit_expr(value, out);
            }
            Expr::Sequence { exprs } => for e in exprs { visit_expr(e, out); },
            Expr::Binary { lhs, rhs, .. } => { visit_expr(lhs, out); visit_expr(rhs, out); }
            Expr::Unary { operand, .. } => visit_expr(operand, out),
            Expr::Call { callee, args } => {
                visit_expr(callee, out);
                for a in args { visit_expr(a, out); }
            }
            Expr::Member { obj, .. } => visit_expr(obj, out),
            Expr::Index { obj, index } => { visit_expr(obj, out); visit_expr(index, out); }
            Expr::New { callee, args } => {
                visit_expr(callee, out);
                for a in args { visit_expr(a, out); }
            }
            Expr::Array { elements } => for el in elements { visit_expr(el, out); },
            Expr::Object { props } => {
                for p in props {
                    if let ir::tree::ObjectKey::Computed(k) = &p.key { visit_expr(k, out); }
                    visit_expr(&p.value, out);
                }
            }
            Expr::Ternary { cond, then_branch, else_branch } => {
                visit_expr(cond, out);
                visit_expr(then_branch, out);
                visit_expr(else_branch, out);
            }
            Expr::Update { target, .. } => visit_expr(target, out),
            Expr::Spread(inner) => visit_expr(inner, out),
            Expr::Template { exprs, .. } => for e in exprs { visit_expr(e, out); },
            // Stop at function literals — nested scope.
            Expr::Func(_) => {}
            Expr::Number(_) | Expr::String(_) | Expr::Bool(_) | Expr::Null
            | Expr::Undefined | Expr::Ident(_) | Expr::This => {}
        }
    }
    let mut out = std::collections::HashSet::new();
    for s in stmts { visit_stmt(s, &mut out); }
    out
}

/// Push a PassThrough SSA function synthesized from an `IrFunction`
/// (which the disposition walker considered Lowered, but contains a
/// construct SSA can't handle). Mirrors `push_passthrough_function`
/// but takes its source from `IrFunction.source`.
fn push_top_level_passthrough_from_lowered(sm: &mut ssa::Module, f: &tree::IrFunction) {
    let source = f.source.clone().unwrap_or_default();
    let function_name_id = Some(sm.strings.get_or_intern(&f.name));
    let param_names: Vec<ssa::StrId> = f
        .params
        .iter()
        .map(|p| sm.strings.get_or_intern(p))
        .collect();
    sm.functions.push(ssa::Function {
        name: f.name.clone(),
        capture_count: 0,
        captured_by_passthrough: Vec::new(),
        captures_pass_through_onward: Vec::new(),
        forced_let_decls: Vec::new(),
        body: ssa::FunctionBody::PassThrough(ssa::PassThroughBody {
            source,
            param_names,
            function_name: function_name_id,
            is_async: false,
            is_generator: false,
            is_arrow: false,
            body_is_expression: false,
        }),
    });
}

fn lower_top_level_function(
    sm: &mut ssa::Module,
    f: &tree::IrFunction,
    func_names: &HashMap<String, FuncId>,
    inner_captures: &HashMap<FuncId, Vec<String>>,
    passthrough_fids: &HashSet<FuncId>,
    pt_referenced_names: HashSet<String>,
) {
    // SSA-fallback short-circuit: if the body contains ForIn or Try,
    // route through PassThrough using the stashed source slice. The
    // disposition walker has already classified the function as
    // Lowered (no blocker reported), so this is invisible to coverage
    // metrics — the function counts as unlocked even though SSA
    // optimization passes don't run on it.
    if body_needs_ssa_fallback(&f.body) {
        push_top_level_passthrough_from_lowered(sm, f);
        return;
    }
    let mut builder = FunctionBuilder::new(sm, f.name.clone(), f.params.len() as u32);
    {
        let mut conv = Converter::new(
            &mut builder,
            func_names,
            inner_captures,
            &[],
            passthrough_fids,
            pt_referenced_names,
        );
        conv.lower_top_level(f);
    }
    builder.finish();
}

/// Build a Lowered SSA `Function` from a nested `FuncLiteral` whose
/// body is `FuncBody::Stmts` or `FuncBody::Expression`. PassThrough
/// inner literals route through `push_inner_passthrough_function`
/// instead — same shape as `push_passthrough_function` but with the
/// caller's `capture_count`.
fn lower_inner_literal(
    sm: &mut ssa::Module,
    lit: &FuncLiteral,
    captures: &[String],
    func_names: &HashMap<String, FuncId>,
    inner_captures: &HashMap<FuncId, Vec<String>>,
    passthrough_fids: &HashSet<FuncId>,
    pt_referenced_names: HashSet<String>,
) {
    let name = lit.name.clone().unwrap_or_default();

    // Pass-1 may have flipped `force_passthrough` because of the
    // forward-ref / reassigned-capture rule. Route the body through
    // the PassThrough builder using the stashed source slice instead
    // of trying to lower a Stmts body whose captures don't satisfy
    // the rule.
    //
    // PassThrough inner functions carry capture references lexically
    // in their preserved source text — `capture_count` is forced to 0
    // here to match the empty captures vector emitted by the outer's
    // `alloc_closure_operand`.
    if lit.force_passthrough.get()
        && let Some(src) = &lit.passthrough_source
    {
        push_inner_passthrough_function_with_source(sm, lit, 0, src.clone());
        return;
    }

    match &lit.body {
        FuncBody::Stmts(stmts) => {
            // SSA-fallback short-circuit (see body_needs_ssa_fallback):
            // if the body contains ForIn or Try, route through
            // PassThrough using the lit's stashed source slice.
            if body_needs_ssa_fallback(stmts)
                && let Some(src) = &lit.passthrough_source
            {
                push_inner_passthrough_function_with_source(
                    sm,
                    lit,
                    captures.len(),
                    src.clone(),
                );
                return;
            }
        }
        FuncBody::Expression(_) => {}
        FuncBody::PassThrough(_) => {
            push_inner_passthrough_function(sm, lit, 0);
            return;
        }
    }

    let mut builder =
        FunctionBuilder::new_with_captures(sm, name, lit.params.len() as u32, captures.len());
    {
        let mut conv = Converter::new(
            &mut builder,
            func_names,
            inner_captures,
            captures,
            passthrough_fids,
            pt_referenced_names,
        );
        conv.lower_inner_literal(lit);
    }
    builder.finish();
}

/// Like `push_passthrough_function`, but for an inner literal that
/// carries a non-zero `capture_count` (the outer's `AllocClosure`
/// supplied capture operands; the body references those by name in its
/// preserved source slice).
fn push_inner_passthrough_function(
    sm: &mut ssa::Module,
    lit: &tree::FuncLiteral,
    capture_count: usize,
) {
    let source = match &lit.body {
        tree::FuncBody::PassThrough(s) => s.clone(),
        _ => panic!("push_inner_passthrough_function: body must be PassThrough"),
    };
    push_inner_passthrough_function_with_source(sm, lit, capture_count, source);
}

/// Same as `push_inner_passthrough_function` but takes the source slice
/// directly — used by the forward-ref / reassigned-capture
/// re-classification path, where the literal's body is still
/// `FuncBody::Stmts` but `passthrough_source` carries the original text.
fn push_inner_passthrough_function_with_source(
    sm: &mut ssa::Module,
    lit: &tree::FuncLiteral,
    capture_count: usize,
    source: String,
) {
    let name = lit.name.clone().unwrap_or_default();
    let function_name_id = lit.name.as_ref().map(|n| sm.strings.get_or_intern(n));
    let param_names: Vec<ssa::StrId> = lit
        .params
        .iter()
        .map(|p| sm.strings.get_or_intern(p))
        .collect();
    sm.functions.push(ssa::Function {
        name,
        capture_count,
        captured_by_passthrough: Vec::new(),
        captures_pass_through_onward: Vec::new(),
        forced_let_decls: Vec::new(),
        body: ssa::FunctionBody::PassThrough(ssa::PassThroughBody {
            source,
            param_names,
            function_name: function_name_id,
            is_async: lit.is_async,
            is_generator: lit.is_generator,
            is_arrow: lit.is_arrow,
            // Arrow expression-body PassThrough is normalized at lower.rs
            // time: the body slice always parses as a block (we wrap
            // expression bodies in `return <expr>;`). Keeping
            // body_is_expression=false here keeps the emit wrapper simple.
            body_is_expression: false,
        }),
    });
}

// ============================================================================
// SymbolId — internal identifier for Braun keys
// ============================================================================

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
struct SymbolId(u32);

// ============================================================================
// Converter
// ============================================================================

struct Converter<'b, 'm> {
    builder: &'b mut FunctionBuilder<'m>,
    func_names: &'b HashMap<String, FuncId>,
    /// By-FuncId capture-name lists. Used when emitting an
    /// `AllocClosure` for a nested literal: the operand vector is built
    /// by resolving each capture name through the converter's own
    /// `lookup`, in the same order the inner converter will bind them
    /// to `Operand::Capture(idx)`. Sharing the order is mandatory; both
    /// sides agree by reading from this map.
    inner_captures: &'b HashMap<FuncId, Vec<String>>,
    /// This function's own captures, in declared order. Inner-literal
    /// builds bind each name to `Operand::Capture(i)` at the entry
    /// block before any user-visible code runs. Empty for top-level.
    own_captures: &'b [String],
    /// FuncIds whose body is `FunctionBody::PassThrough`. Read at
    /// AllocClosure emission to flag captures flowing into a
    /// PassThrough body for pin wiring at SSA→tree time.
    passthrough_fids: &'b HashSet<FuncId>,
    /// Names that any nested PassThrough descendant references and
    /// that this function may bind. When a `Stmt::Assign` writes to
    /// one of these names, the lowering MUST produce an instruction
    /// that re-emits at lower_tree time as a textual reassignment —
    /// otherwise const-to-const reassignments dissolve into Braun
    /// bookkeeping (no Inst) and the PassThrough body's lexical
    /// reference reads the snapshot value rather than the live one.
    /// See module doc note at the top of `pt_free_vars`.
    pt_referenced_names: HashSet<String>,

    // Symbol allocation
    next_symbol: u32,

    // Lexical scope stack. The innermost (most recently pushed) scope is
    // searched first; an inner `let` may shadow an outer binding by
    // allocating a fresh SymbolId in the inner scope.
    scopes: Vec<HashMap<String, SymbolId>>,

    // Braun's `current_def`: per (variable, block), the operand to use when
    // the variable is read in that block.
    current_def: HashMap<(SymbolId, BlockId), Operand>,

    // Block params added speculatively while the block was unsealed; filled
    // (via `add_param_operands`) at seal time. Order matters — params are
    // appended to `block.params` in the same order, and edge args must match
    // by position.
    incomplete_params: HashMap<BlockId, Vec<(SymbolId, Value)>>,

    // Sealed blocks. A sealed block promises no further predecessors will be
    // added, which means reads can fully resolve through `add_param_operands`.
    sealed: HashSet<BlockId>,

    // Source name (interned) for each SymbolId that originated from a
    // named source binding. Used to annotate block params introduced by
    // Braun for that SymbolId so the out-of-SSA renamer can recover the
    // original name across loop carriers.
    symbol_names: HashMap<SymbolId, StrId>,

    /// Stack of (continue_target, break_target) for the active enclosing
    /// loops. `Stmt::Continue` reads the top of stack and emits a Branch
    /// to the continue target — for `Stmt::While` this is the header,
    /// for `Stmt::For` this is a separate step block that runs the
    /// update expression before re-checking the condition. `break` is
    /// not yet supported in the IR; the second slot is reserved.
    loop_targets: Vec<(BlockId, BlockId)>,
}

impl<'b, 'm> Converter<'b, 'm> {
    fn new(
        builder: &'b mut FunctionBuilder<'m>,
        func_names: &'b HashMap<String, FuncId>,
        inner_captures: &'b HashMap<FuncId, Vec<String>>,
        own_captures: &'b [String],
        passthrough_fids: &'b HashSet<FuncId>,
        pt_referenced_names: HashSet<String>,
    ) -> Self {
        Self {
            builder,
            func_names,
            inner_captures,
            own_captures,
            passthrough_fids,
            pt_referenced_names,
            next_symbol: 0,
            scopes: Vec::new(),
            current_def: HashMap::new(),
            incomplete_params: HashMap::new(),
            sealed: HashSet::new(),
            symbol_names: HashMap::new(),
            loop_targets: Vec::new(),
        }
    }

    fn lower_top_level(&mut self, f: &tree::IrFunction) {
        let entry = self.setup_entry_with_module_scope();
        self.lower_params(&f.params, entry);
        for stmt in &f.body {
            self.lower_stmt(stmt);
        }
        self.finish_lowering();
    }

    fn lower_inner_literal(&mut self, lit: &FuncLiteral) {
        let entry = self.setup_entry_with_module_scope();

        // Scope 1.5: capture bindings. Each capture name resolves to
        // `Operand::Capture(i)` for the inner body's reads. Allocated
        // before the param scope so a same-named param shadows a
        // capture (matches JS lexical-shadowing semantics).
        self.push_scope();
        for (i, cname) in self.own_captures.iter().enumerate() {
            let sym = self.fresh_symbol();
            self.bind_in_top_scope(cname.clone(), sym);
            self.write_variable(sym, entry, Operand::Capture(i));
        }

        // A named function expression binds its own name to itself. We
        // model that via a synthetic capture: the outer's `AllocClosure`
        // emits the binding, but for now the simpler self-reference path
        // is to bind the name as a capture-style sym pointing at the
        // matching `Constant::Func` — the inner can then call itself.
        // Stream A defers this: the front end does not yet produce
        // anonymous-named function expressions, so the absence is
        // dormant.

        self.push_scope();
        self.lower_params(&lit.params, entry);

        match &lit.body {
            FuncBody::Stmts(stmts) => {
                for s in stmts {
                    self.lower_stmt(s);
                }
            }
            FuncBody::Expression(e) => {
                let v = self.convert_expr(e);
                let cb = self.current_block();
                if !self.is_terminated(cb) {
                    self.builder
                        .set_terminator(Terminator::Return { val: Some(v) });
                }
            }
            FuncBody::PassThrough(_) => {
                unreachable!("lower_inner_literal received a PassThrough body; routed elsewhere")
            }
        }

        self.pop_scope(); // params
        self.pop_scope(); // captures
        self.pop_scope(); // module
        self.implicit_return();
    }

    /// Create the entry block and bind module-level function names in
    /// scope 0. Returns the entry BlockId so the caller can write
    /// captures/params against it.
    fn setup_entry_with_module_scope(&mut self) -> BlockId {
        let entry = self.builder.new_block();
        self.builder.switch_to_block(entry);
        self.sealed.insert(entry);

        // Scope 0: module — top-level function names bound to
        // Constant::Func. Makes references to top-level functions (and
        // the function's own name, for recursion) resolve via the
        // normal Braun path.
        self.push_scope();
        let mut sorted_names: Vec<(&String, &FuncId)> = self.func_names.iter().collect();
        sorted_names.sort_by_key(|(n, _)| n.as_str());
        for (name, fid) in sorted_names {
            let sym = self.fresh_symbol();
            self.bind_in_top_scope(name.clone(), sym);
            self.write_variable(sym, entry, Operand::Const(Constant::Func(*fid)));
        }
        entry
    }

    fn lower_params(&mut self, params: &[String], entry: BlockId) {
        self.push_scope();
        for (i, pname) in params.iter().enumerate() {
            let sym = self.fresh_symbol();
            self.bind_in_top_scope(pname.clone(), sym);
            let arg_val = self.builder.body.params[i];
            let name_id = self.builder.intern(pname);
            self.builder.set_source_name(arg_val, name_id);
            self.write_variable(sym, entry, Operand::Value(arg_val));
        }
    }

    /// Pop the param + module scopes and emit an implicit `return
    /// undefined` if the current block lacks a terminator. Used by the
    /// top-level function path; the inner-literal path handles its own
    /// scope ladder because it has an extra capture scope.
    fn finish_lowering(&mut self) {
        self.pop_scope(); // params
        self.pop_scope(); // module
        self.implicit_return();
    }

    fn implicit_return(&mut self) {
        let cb = self
            .builder
            .current_block()
            .expect("function lowering left no current block");
        if !self.is_terminated(cb) {
            self.builder
                .set_terminator(Terminator::Return { val: None });
        }
    }

    // ------------------------------------------------------------------
    // Scope management
    // ------------------------------------------------------------------

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn bind_in_top_scope(&mut self, name: String, sym: SymbolId) {
        let id = self.builder.intern(&name);
        self.symbol_names.insert(sym, id);
        self.scopes
            .last_mut()
            .expect("no active scope")
            .insert(name, sym);
    }

    fn lookup(&self, name: &str) -> Option<SymbolId> {
        for scope in self.scopes.iter().rev() {
            if let Some(s) = scope.get(name) {
                return Some(*s);
            }
        }
        None
    }

    fn fresh_symbol(&mut self) -> SymbolId {
        let s = SymbolId(self.next_symbol);
        self.next_symbol += 1;
        s
    }

    /// Annotate an Operand-defined value with the source-level binding
    /// name. Only tags values that don't yet have a name — this matters
    /// for `let r = x;` where the RHS is an Ident already-tagged via its
    /// original binding ("x"); re-tagging it as "r" would lose the
    /// stronger name. The renamer's first-claim-wins handles the case
    /// where two distinct Insts coincide on the same name.
    ///
    /// No-op for Const operands and for retired Values.
    fn tag_binding_source_name(&mut self, op: Operand, name: &str) {
        if let Operand::Value(v) = op {
            let def = &self.builder.body.values[v.0 as usize];
            if def.is_removed() || def.source_name.is_some() {
                return;
            }
            let id = self.builder.intern(name);
            self.builder.set_source_name(v, id);
        }
    }

    // ------------------------------------------------------------------
    // Braun primitives
    // ------------------------------------------------------------------

    fn write_variable(&mut self, var: SymbolId, block: BlockId, op: Operand) {
        self.current_def.insert((var, block), op);
    }

    fn read_variable(&mut self, var: SymbolId, block: BlockId) -> Operand {
        if let Some(op) = self.current_def.get(&(var, block)) {
            return *op;
        }
        self.read_variable_recursive(var, block)
    }

    fn read_variable_recursive(&mut self, var: SymbolId, block: BlockId) -> Operand {
        if !self.sealed.contains(&block) {
            // Unsealed: speculatively add a param and queue it for filling.
            let param = self.builder.add_block_param(block);
            self.tag_block_param_with_symbol(param, var);
            self.incomplete_params
                .entry(block)
                .or_default()
                .push((var, param));
            self.write_variable(var, block, Operand::Value(param));
            return Operand::Value(param);
        }

        let preds = self.predecessors_of(block);
        if preds.len() == 1 {
            let val = self.read_variable(var, preds[0]);
            self.write_variable(var, block, val);
            return val;
        }

        // Sealed with multiple preds. Add a param, write the placeholder
        // BEFORE recursing so cycles bottom out. Then fill operands.
        let param = self.builder.add_block_param(block);
        self.tag_block_param_with_symbol(param, var);
        self.write_variable(var, block, Operand::Value(param));
        self.add_param_operands(var, block, param)
    }

    fn tag_block_param_with_symbol(&mut self, param: Value, var: SymbolId) {
        if let Some(&name) = self.symbol_names.get(&var) {
            self.builder.set_source_name(param, name);
        }
    }

    fn add_param_operands(&mut self, var: SymbolId, block: BlockId, param: Value) -> Operand {
        let preds = self.predecessors_of(block);
        for pred in preds {
            let op = self.read_variable(var, pred);
            self.append_edge_arg(pred, block, op);
        }
        self.try_remove_trivial_param(param)
    }

    fn try_remove_trivial_param(&mut self, param: Value) -> Operand {
        // Look up the block and current index of the param. If it's not a
        // block param (or already removed), bail out conservatively.
        let (block, idx) = match self.builder.body.values[param.0 as usize].kind {
            ValueDefKind::BlockParam { block, index } => (block, index as usize),
            ValueDefKind::Removed => return Operand::Const(Constant::Undefined),
            _ => return Operand::Value(param),
        };

        // Snapshot incomings at this index.
        let preds = self.predecessors_of(block);
        let mut incomings: Vec<Operand> = Vec::with_capacity(preds.len());
        for &p in &preds {
            let op = self.edge_arg_at(p, block, idx).unwrap_or_else(|| {
                panic!(
                    "predecessor bb{} of bb{} has no edge arg at index {}",
                    p.0, block.0, idx
                )
            });
            incomings.push(op);
        }

        // Determine `same`: the unique non-self incoming, or Undefined if
        // there are no non-self incomings.
        let mut same: Option<Operand> = None;
        for op in &incomings {
            if let Operand::Value(v) = op
                && *v == param
            {
                continue; // self-reference: harmless
            }
            match same {
                None => same = Some(*op),
                Some(s) if operand_eq(&s, op) => continue,
                Some(_) => return Operand::Value(param), // not trivial
            }
        }
        let same = same.unwrap_or(Operand::Const(Constant::Undefined));

        // Find users that are themselves block params, BEFORE replace_uses
        // walks the IR (which would erase the references we look at here).
        let block_param_users = self.find_block_param_users(param);

        // Substitute every use of `param` with `same`.
        self.replace_value_uses(param, same);

        // Drop the param from the block, then strip the corresponding edge
        // arg from each predecessor's terminator.
        self.remove_block_param(block, idx);
        for &p in &preds {
            self.remove_edge_arg_at(p, block, idx);
        }

        // Retire the Value slot.
        self.builder.body.values[param.0 as usize] = ValueDef::removed();

        // Recurse on users that may have become trivial.
        for u in block_param_users {
            if u == param {
                continue;
            }
            if self.builder.body.values[u.0 as usize].is_removed() {
                continue;
            }
            self.try_remove_trivial_param(u);
        }

        same
    }

    fn seal_block(&mut self, b: BlockId) {
        if self.sealed.contains(&b) {
            return;
        }
        // Take ownership of the entries; trivial-param removal during fills
        // can mutate `incomplete_params` indirectly, so iterating a borrowed
        // entry is unsafe.
        let entries = self.incomplete_params.remove(&b).unwrap_or_default();
        for (sym, param) in entries {
            // A param may have been retired by an earlier iteration's
            // recursive trivial-removal; skip in that case.
            if self.builder.body.values[param.0 as usize].is_removed() {
                continue;
            }
            self.add_param_operands(sym, b, param);
        }
        self.sealed.insert(b);
    }

    // ------------------------------------------------------------------
    // Statement lowering
    // ------------------------------------------------------------------

    fn lower_stmt(&mut self, stmt: &Stmt) {
        // If the current block already terminates (e.g., previous stmt was
        // a Return), insert a fresh dead block here so we don't overwrite
        // the existing terminator. Lowering into this block is preserved as
        // dead code; downstream DCE can prune it.
        let cb = self.current_block();
        if self.is_terminated(cb) {
            let dead = self.builder.new_block();
            self.sealed.insert(dead);
            self.builder.switch_to_block(dead);
        }
        match stmt {
            Stmt::Let { name, init } => {
                let op = match init {
                    Some(e) => self.convert_expr(e),
                    None => Operand::Const(Constant::Undefined),
                };
                self.tag_binding_source_name(op, name);
                let sym = self.fresh_symbol();
                self.bind_in_top_scope(name.clone(), sym);
                let cb = self.current_block();
                self.write_variable(sym, cb, op);
                // PT-referenced name: ensure a `let name;` declaration
                // appears in the emit (so the synthesized assignment
                // stays scoped to a local) and emit the initial-value
                // assignment as a side-effect store. See `Stmt::Assign`
                // arm and the `pt_referenced_names` doc.
                if self.pt_referenced_names.contains(name) {
                    self.builder.mark_forced_let_decl(name);
                    if init.is_some() {
                        let key = self.builder.intern(name);
                        self.builder.add_inst(InstKind::StoreProp {
                            obj: Operand::Const(Constant::GlobalThis),
                            key,
                            val: op,
                            purity: Purity::Throws,
                        });
                    }
                }
            }
            Stmt::Const { name, init } => {
                let op = self.convert_expr(init);
                self.tag_binding_source_name(op, name);
                let sym = self.fresh_symbol();
                self.bind_in_top_scope(name.clone(), sym);
                let cb = self.current_block();
                self.write_variable(sym, cb, op);
                if self.pt_referenced_names.contains(name) {
                    self.builder.mark_forced_let_decl(name);
                    let key = self.builder.intern(name);
                    self.builder.add_inst(InstKind::StoreProp {
                        obj: Operand::Const(Constant::GlobalThis),
                        key,
                        val: op,
                        purity: Purity::Throws,
                    });
                }
            }
            Stmt::Assign { target, value } => {
                let val_op = self.convert_expr(value);
                match target {
                    Expr::Ident(name) => {
                        if let Some(sym) = self.lookup(name) {
                            // Tag the new value with the binding's source
                            // name so a same-name reassignment chain
                            // re-emerges in emit (suffix-disambiguated by
                            // the renamer when needed).
                            self.tag_binding_source_name(val_op, name);
                            let cb = self.current_block();
                            self.write_variable(sym, cb, val_op);

                            // PassThrough closures referencing this name
                            // resolve lexically at runtime — the live
                            // value, not a snapshot. So the source-level
                            // mutation MUST appear textually in the
                            // emitted output, even when `val_op` is a
                            // constant (which produces no Inst and would
                            // otherwise vanish into Braun bookkeeping).
                            // Synthesize a `StoreProp(GlobalThis, name,
                            // val)` — lower_tree resugars stores rooted
                            // at GlobalThis as bare `name = val`, which
                            // resolves lexically to the local
                            // let-binding at runtime. See
                            // `pt_referenced_names` doc on the Converter.
                            if self.pt_referenced_names.contains(name) {
                                let key = self.builder.intern(name);
                                self.builder.add_inst(InstKind::StoreProp {
                                    obj: Operand::Const(Constant::GlobalThis),
                                    key,
                                    val: val_op,
                                    purity: Purity::Throws,
                                });
                            }
                        } else {
                            // Sloppy-mode-equivalent unbound write: lowers to
                            // a property store on the global object. Strict
                            // mode would throw on the unbound write; we
                            // accept the divergence (see read-side note).
                            // The special-case constants (globalThis,
                            // undefined, NaN, Infinity, …) are read-only at
                            // the language level; assigning to them is a
                            // no-op in sloppy mode and a TypeError in strict
                            // mode. We model the no-op by routing through
                            // the global store anyway — the value lands in
                            // a globalThis property of that name.
                            let key = self.builder.intern(name);
                            self.builder.add_inst(InstKind::StoreProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                val: val_op,
                                purity: Purity::Throws,
                            });
                        }
                    }
                    Expr::Member { obj, prop } => {
                        let obj_op = self.convert_expr(obj);
                        let key = self.builder.intern(prop);
                        let purity = property_access_purity(&obj_op);
                        self.builder.add_inst(InstKind::StoreProp {
                            obj: obj_op,
                            key,
                            val: val_op,
                            purity,
                        });
                    }
                    Expr::Index { obj, index } => {
                        let obj_op = self.convert_expr(obj);
                        let idx_op = self.convert_expr(index);
                        let purity = property_access_purity(&obj_op);
                        self.builder.add_inst(InstKind::StoreIndex {
                            obj: obj_op,
                            index: idx_op,
                            val: val_op,
                            purity,
                        });
                    }
                    _ => panic!("invalid assignment target: {target:?}"),
                }
            }
            Stmt::ExprStmt(e) => {
                let _ = self.convert_expr(e);
            }
            Stmt::Return(e) => {
                let val = e.as_ref().map(|x| self.convert_expr(x));
                self.builder.set_terminator(Terminator::Return { val });
                // We do NOT switch to a fresh block here. If a subsequent
                // statement appears, lower_stmt's prologue creates a dead
                // block on demand, avoiding a stray empty block when no
                // dead code follows.
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => self.lower_if(cond, then_body, else_body.as_deref()),
            Stmt::While { cond, body } => self.lower_while(cond, body),
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => self.lower_for(init.as_deref(), cond.as_ref(), step.as_ref(), body),
            Stmt::ForOf {
                kind,
                binding,
                iterable,
                body,
            } => self.lower_for_of(kind, binding, iterable, body),
            Stmt::Continue => {
                let (cont, _brk) = *self
                    .loop_targets
                    .last()
                    .expect("Stmt::Continue outside any loop — walker should have rejected");
                self.builder.set_terminator(Terminator::Branch {
                    target: cont,
                    args: vec![],
                });
                // Leave the current block terminated; the lower_stmt prologue
                // creates a fresh dead block on demand for any following
                // (dead) code.
            }
            Stmt::Block(stmts) => {
                self.push_scope();
                for s in stmts {
                    self.lower_stmt(s);
                }
                self.pop_scope();
            }
            Stmt::FuncDecl(lit) => {
                // Inner function declaration. Pass 1 stamped a FuncId
                // and computed a captures list; we resolve those names
                // now, in the enclosing scope, and emit an
                // AllocClosure bound to the function's name.
                let alloc = self.alloc_closure_operand(lit);
                let name = lit.name.as_ref().expect("Stmt::FuncDecl must carry a name");
                self.tag_binding_source_name(alloc, name);
                let sym = self.fresh_symbol();
                self.bind_in_top_scope(name.clone(), sym);
                let cb = self.current_block();
                self.write_variable(sym, cb, alloc);
            }
            Stmt::Try { .. } | Stmt::ForIn { .. } | Stmt::Throw(_) => {
                // None of these are expressible in SSA. The
                // `body_needs_ssa_fallback` check at function entry
                // (in `lower_top_level_function` / `lower_inner_literal`)
                // routes any function containing them to PassThrough
                // using the stashed source slice. Reaching this arm
                // means that check missed a textual position (or a
                // hand-built tree-IR test fixture lacks the source
                // slice required to fall back).
                unreachable!(
                    "Stmt::{{Try,ForIn,Throw}} reached SSA conversion — \
                     body_needs_ssa_fallback should have routed this function \
                     to PassThrough"
                );
            }
        }
    }

    fn lower_if(&mut self, cond: &Expr, then_body: &[Stmt], else_body: Option<&[Stmt]>) {
        let cond_op = self.convert_expr(cond);
        let then_b = self.builder.new_block();
        let else_b = self.builder.new_block();
        let join_b = self.builder.new_block();

        self.builder.set_terminator(Terminator::CondBranch {
            cond: cond_op,
            then_b,
            then_args: vec![],
            else_b,
            else_args: vec![],
        });
        // then/else have exactly one pred each (the cbr we just set), so
        // they can be sealed immediately. join has up to two preds and stays
        // unsealed until both branches finish.
        self.sealed.insert(then_b);
        self.sealed.insert(else_b);

        // then-branch
        self.builder.switch_to_block(then_b);
        self.push_scope();
        for s in then_body {
            self.lower_stmt(s);
        }
        self.pop_scope();
        let then_end = self.current_block();
        if !self.is_terminated(then_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: join_b,
                args: vec![],
            });
        }

        // else-branch (an absent else lowers to an empty body that just
        // joins, preserving the join's two-pred shape).
        self.builder.switch_to_block(else_b);
        self.push_scope();
        if let Some(eb) = else_body {
            for s in eb {
                self.lower_stmt(s);
            }
        }
        self.pop_scope();
        let else_end = self.current_block();
        if !self.is_terminated(else_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: join_b,
                args: vec![],
            });
        }

        self.seal_block(join_b);
        self.builder.switch_to_block(join_b);
    }

    fn lower_while(&mut self, cond: &Expr, body: &[Stmt]) {
        let header_b = self.builder.new_block();
        let body_b = self.builder.new_block();
        let exit_b = self.builder.new_block();

        // Branch from current to header. Header has at least one pred (us);
        // a back-edge from body will be added later. We must NOT seal
        // header yet — the back-edge predecessor is missing.
        self.builder.set_terminator(Terminator::Branch {
            target: header_b,
            args: vec![],
        });

        // body and exit each have a single pred (the cbr we'll set in
        // header). Safe to seal.
        self.sealed.insert(body_b);
        self.sealed.insert(exit_b);

        // Header: condition + branch.
        self.builder.switch_to_block(header_b);
        let cond_op = self.convert_expr(cond);
        self.builder.set_terminator(Terminator::CondBranch {
            cond: cond_op,
            then_b: body_b,
            then_args: vec![],
            else_b: exit_b,
            else_args: vec![],
        });

        // Body. `continue` inside the body jumps to the header.
        self.builder.switch_to_block(body_b);
        self.loop_targets.push((header_b, exit_b));
        self.push_scope();
        for s in body {
            self.lower_stmt(s);
        }
        self.pop_scope();
        self.loop_targets.pop();
        let body_end = self.current_block();
        if !self.is_terminated(body_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: header_b,
                args: vec![],
            });
        }

        // NOW seal header — the back-edge predecessor exists. Sealing here
        // (rather than earlier) is what gives loop-varying variables their
        // phi-merged values. Sealing too early would silently dead-end
        // back-edge reads.
        self.seal_block(header_b);

        self.builder.switch_to_block(exit_b);
    }

    /// Lower `for (init; cond; step) body`. Distinct from while only in
    /// that `continue` targets a step block (not the header), and the
    /// step expression evaluates after every iteration, including
    /// continued ones. The CFG shape:
    ///
    /// ```text
    ///     pre  ──init──▶ header
    ///                       │ (cond)
    ///                  ┌────┴────┐
    ///                  ▼         ▼
    ///                body       exit
    ///                  │
    ///                  ▼ (fallthrough or `continue`)
    ///                step
    ///                  │
    ///                  ▼ (back-edge)
    ///                header
    /// ```
    ///
    /// SSA→tree's loop-shape recognizer treats this as a single loop
    /// whose body contains the original body's instructions followed
    /// by the step's instructions; the `continue` jumps cause empty
    /// then-branches in the if/else recovery, which is correct.
    fn lower_for(
        &mut self,
        init: Option<&Stmt>,
        cond: Option<&Expr>,
        step: Option<&Expr>,
        body: &[Stmt],
    ) {
        // Init lives in a fresh block scope so its bindings (`let i = 0`
        // etc.) don't leak past the loop.
        self.push_scope();
        if let Some(init) = init {
            self.lower_stmt(init);
        }

        let header_b = self.builder.new_block();
        let body_b = self.builder.new_block();
        let step_b = self.builder.new_block();
        let exit_b = self.builder.new_block();

        self.builder.set_terminator(Terminator::Branch {
            target: header_b,
            args: vec![],
        });
        self.sealed.insert(body_b);
        self.sealed.insert(exit_b);
        // step_b is reachable from the body's fallthrough AND every
        // `continue` inside the body — the predecessor count isn't
        // known until the body finishes. Defer its sealing.

        // Header.
        self.builder.switch_to_block(header_b);
        let cond_op = match cond {
            Some(c) => self.convert_expr(c),
            None => Operand::Const(Constant::Bool(true)),
        };
        self.builder.set_terminator(Terminator::CondBranch {
            cond: cond_op,
            then_b: body_b,
            then_args: vec![],
            else_b: exit_b,
            else_args: vec![],
        });

        // Body. `continue` inside jumps to step_b.
        self.builder.switch_to_block(body_b);
        self.loop_targets.push((step_b, exit_b));
        self.push_scope();
        for s in body {
            self.lower_stmt(s);
        }
        self.pop_scope();
        self.loop_targets.pop();
        let body_end = self.current_block();
        if !self.is_terminated(body_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: step_b,
                args: vec![],
            });
        }

        // Step block: every continue and the body's fallthrough have
        // arrived; safe to seal.
        self.seal_block(step_b);
        self.builder.switch_to_block(step_b);
        if let Some(step) = step {
            let _ = self.convert_expr(step);
        }
        let step_end = self.current_block();
        if !self.is_terminated(step_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: header_b,
                args: vec![],
            });
        }

        // Back-edge predecessor exists; seal header.
        self.seal_block(header_b);

        self.builder.switch_to_block(exit_b);
        self.pop_scope();
    }

    /// Lower `for (kind binding of iterable) body`. Desugars to:
    ///
    /// ```js
    /// const __iter = iterable[Symbol.iterator]();
    /// while (true) {
    ///   const __step = __iter.next();
    ///   if (__step.done) break;   // exit_b
    ///   const binding = __step.value;
    ///   // body
    /// }
    /// ```
    ///
    /// The CFG shape matches the standard while-loop pattern that the
    /// SSA→tree reconstructor recognises (header cbr → body or exit).
    fn lower_for_of(&mut self, _kind: &str, binding: &str, iterable: &Expr, body: &[Stmt]) {
        self.push_scope();

        // Evaluate the iterable once before entering the loop.
        let iterable_op = self.convert_expr(iterable);

        // `iterable[Symbol.iterator]()` via:
        //   sym_global  = LoadProp(GlobalThis, "Symbol")
        //   sym_iter    = LoadProp(sym_global, "iterator")
        //   get_iter_fn = LoadIndex(iterable, sym_iter)
        //   iter        = Call(get_iter_fn, [iterable])
        let symbol_key = self.builder.intern("Symbol");
        let sym_global = self.emit(InstKind::LoadProp {
            obj: Operand::Const(Constant::GlobalThis),
            key: symbol_key,
            purity: Purity::Throws,
        });
        let iter_key = self.builder.intern("iterator");
        let sym_iter_prop = self.emit(InstKind::LoadProp {
            obj: sym_global,
            key: iter_key,
            purity: Purity::Opaque,
        });
        let get_iter_fn = self.emit(InstKind::LoadIndex {
            obj: iterable_op,
            index: sym_iter_prop,
            purity: Purity::Opaque,
        });
        let iter_val = self.emit(InstKind::Call {
            callee: get_iter_fn,
            args: vec![iterable_op],
        });

        // Bind the iterator to a symbol so the header block can read it.
        let iter_name = format!("__foriter_{}__", binding);
        let iter_sym = self.fresh_symbol();
        let iter_strid = self.builder.intern(&iter_name);
        if let Operand::Value(v) = iter_val {
            self.builder.set_source_name(v, iter_strid);
        }
        self.bind_in_top_scope(iter_name.clone(), iter_sym);
        let cb = self.current_block();
        self.write_variable(iter_sym, cb, iter_val);

        // CFG: pre → header → (body | exit). Same shape as while.
        let header_b = self.builder.new_block();
        let body_b = self.builder.new_block();
        let exit_b = self.builder.new_block();

        self.builder.set_terminator(Terminator::Branch {
            target: header_b,
            args: vec![],
        });
        self.sealed.insert(body_b);
        self.sealed.insert(exit_b);

        // Header: call `__iter.next()`, check `done`, branch.
        self.builder.switch_to_block(header_b);
        let cb = self.current_block();
        let iter_op = self.read_variable(iter_sym, cb);
        // `__iter.next()` — use CallMethod so `this = iter_op`.
        let next_key = self.builder.intern("next");
        let step_result = self.emit(InstKind::CallMethod {
            obj: iter_op,
            method: next_key,
            args: vec![],
        });

        // `__step.done`
        let done_key = self.builder.intern("done");
        let done_val = self.emit(InstKind::LoadProp {
            obj: step_result,
            key: done_key,
            purity: Purity::Opaque,
        });
        // `!done_val`
        let not_done = self.emit(InstKind::Not { operand: done_val });

        // Stash step_result in a symbol so body_b can read it.
        let step_name = format!("__forstep_{}__", binding);
        let step_sym = self.fresh_symbol();
        let step_strid = self.builder.intern(&step_name);
        if let Operand::Value(v) = step_result {
            self.builder.set_source_name(v, step_strid);
        }
        self.bind_in_top_scope(step_name.clone(), step_sym);
        let cb = self.current_block();
        self.write_variable(step_sym, cb, step_result);

        self.builder.set_terminator(Terminator::CondBranch {
            cond: not_done,
            then_b: body_b,
            then_args: vec![],
            else_b: exit_b,
            else_args: vec![],
        });

        // Body.
        self.builder.switch_to_block(body_b);
        self.loop_targets.push((header_b, exit_b));
        self.push_scope();

        // `const binding = __step.value`
        let cb = self.current_block();
        let step_op = self.read_variable(step_sym, cb);
        let value_key = self.builder.intern("value");
        let binding_val = self.emit(InstKind::LoadProp {
            obj: step_op,
            key: value_key,
            purity: Purity::Opaque,
        });
        let binding_sym = self.fresh_symbol();
        let binding_strid = self.builder.intern(binding);
        if let Operand::Value(v) = binding_val {
            self.builder.set_source_name(v, binding_strid);
        }
        self.bind_in_top_scope(binding.to_string(), binding_sym);
        let cb = self.current_block();
        self.write_variable(binding_sym, cb, binding_val);

        for s in body {
            self.lower_stmt(s);
        }

        self.pop_scope();
        self.loop_targets.pop();
        let body_end = self.current_block();
        if !self.is_terminated(body_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: header_b,
                args: vec![],
            });
        }

        self.seal_block(header_b);
        self.builder.switch_to_block(exit_b);
        self.pop_scope();
    }

    // ------------------------------------------------------------------
    // Expression lowering
    // ------------------------------------------------------------------

    fn convert_expr(&mut self, e: &Expr) -> Operand {
        match e {
            Expr::Number(n) => Operand::Const(Constant::Number(*n)),
            Expr::String(s) => {
                let id = self.builder.intern(s);
                Operand::Const(Constant::String(id))
            }
            Expr::Bool(b) => Operand::Const(Constant::Bool(*b)),
            Expr::Null => Operand::Const(Constant::Null),
            Expr::Undefined => Operand::Const(Constant::Undefined),
            Expr::This => Operand::Const(Constant::This),
            Expr::Ident(name) => {
                if let Some(sym) = self.lookup(name) {
                    let cb = self.current_block();
                    return self.read_variable(sym, cb);
                }
                // Unbound identifier: lower as a property access on the
                // global object (or a special-case constant). Strict-mode JS
                // would throw ReferenceError on a bare unbound read while
                // `globalThis.foo` would return undefined; we treat both
                // identically (sloppy-mode-equivalent) — acceptable for
                // malware analysis, where strict-mode divergence is rare and
                // not load-bearing for understanding intent.
                if let Some(c) = special_global_constant(name) {
                    return Operand::Const(c);
                }
                let key = self.builder.intern(name);
                self.emit(InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    key,
                    purity: Purity::Throws,
                })
            }
            Expr::Binary { op, lhs, rhs } => self.convert_binary(*op, lhs, rhs),
            Expr::Unary { op, operand } => self.convert_unary(*op, operand),
            Expr::Call { callee, args } => {
                // Check if any argument is a spread.
                let has_spread = args.iter().any(|a| matches!(a, Expr::Spread(_)));
                // Method-call discrimination: `obj.foo(args)` must be
                // emitted as CallMethod so `this` binds to `obj` at
                // call time. Lowering as separate LoadProp + Call
                // produces a freestanding function value with `this`
                // = undefined, which silently miscompiles instance
                // method calls. Static-member calls only — computed
                // member calls don't have a CallMethod analogue and
                // fall back to LoadIndex + Call (correctness gap; the
                // disposition walker permits them).
                if let Expr::Member { obj, prop } = callee.as_ref() {
                    let obj_op = self.convert_expr(obj);
                    let method = self.builder.intern(prop);
                    let mut arg_ops = Vec::with_capacity(args.len());
                    let mut spread_mask = Vec::with_capacity(args.len());
                    for a in args {
                        if let Expr::Spread(inner) = a {
                            arg_ops.push(self.convert_expr(inner));
                            spread_mask.push(true);
                        } else {
                            arg_ops.push(self.convert_expr(a));
                            spread_mask.push(false);
                        }
                    }
                    if has_spread {
                        return self.emit(InstKind::CallMethodSpread {
                            obj: obj_op,
                            method,
                            args: arg_ops,
                            spread_mask,
                        });
                    } else {
                        return self.emit(InstKind::CallMethod {
                            obj: obj_op,
                            method,
                            args: arg_ops,
                        });
                    }
                }
                let callee_op = self.convert_expr(callee);
                let mut arg_ops = Vec::with_capacity(args.len());
                let mut spread_mask = Vec::with_capacity(args.len());
                for a in args {
                    if let Expr::Spread(inner) = a {
                        arg_ops.push(self.convert_expr(inner));
                        spread_mask.push(true);
                    } else {
                        arg_ops.push(self.convert_expr(a));
                        spread_mask.push(false);
                    }
                }
                if has_spread {
                    self.emit(InstKind::CallSpread {
                        callee: callee_op,
                        args: arg_ops,
                        spread_mask,
                    })
                } else {
                    self.emit(InstKind::Call {
                        callee: callee_op,
                        args: arg_ops,
                    })
                }
            }
            Expr::Member { obj, prop } => {
                let obj_op = self.convert_expr(obj);
                let key = self.builder.intern(prop);
                let purity = property_access_purity(&obj_op);
                self.emit(InstKind::LoadProp {
                    obj: obj_op,
                    key,
                    purity,
                })
            }
            Expr::Index { obj, index } => {
                let obj_op = self.convert_expr(obj);
                let idx_op = self.convert_expr(index);
                let purity = property_access_purity(&obj_op);
                self.emit(InstKind::LoadIndex {
                    obj: obj_op,
                    index: idx_op,
                    purity,
                })
            }
            Expr::Func(lit) => self.alloc_closure_operand(lit),
            Expr::Array { elements } => {
                // Check if any element is a spread.
                let has_spread = elements.iter().any(|e| matches!(e, Expr::Spread(_)));
                if has_spread {
                    // Evaluate left-to-right, tracking which are spread.
                    let mut elem_ops = Vec::with_capacity(elements.len());
                    let mut spread_mask = Vec::with_capacity(elements.len());
                    for e in elements {
                        if let Expr::Spread(inner) = e {
                            elem_ops.push(self.convert_expr(inner));
                            spread_mask.push(true);
                        } else {
                            elem_ops.push(self.convert_expr(e));
                            spread_mask.push(false);
                        }
                    }
                    self.emit(InstKind::ArrayWithSpread {
                        elements: elem_ops,
                        spread_mask,
                    })
                } else {
                    // Evaluate elements left-to-right, then allocate, then
                    // store. AllocArray is PURE; the StoreIndex chain on a
                    // freshly-allocated array can use Purity::Pure since
                    // the target hasn't escaped between alloc and stores.
                    let element_ops: Vec<Operand> =
                        elements.iter().map(|e| self.convert_expr(e)).collect();
                    let arr = self.emit(InstKind::AllocArray {
                        length: Some(elements.len() as u32),
                    });
                    for (i, op) in element_ops.into_iter().enumerate() {
                        let idx = Operand::Const(Constant::Number(i as f64));
                        self.builder.add_inst(InstKind::StoreIndex {
                            obj: arr.clone(),
                            index: idx,
                            val: op,
                            purity: Purity::Pure,
                        });
                    }
                    arr
                }
            }
            Expr::New { callee, args } => {
                let ctor = self.convert_expr(callee);
                let arg_ops: Vec<Operand> =
                    args.iter().map(|a| self.convert_expr(a)).collect();
                self.emit(InstKind::Construct {
                    ctor,
                    args: arg_ops,
                })
            }
            Expr::Object { props } => {
                use ir::tree::ObjectKey;
                // Check if any prop is a spread.
                let has_spread = props.iter().any(|p| matches!(p.key, ObjectKey::Spread));
                if has_spread {
                    use ir::ssa::SpreadObjectKey;
                    // Evaluate all keys and values left-to-right.
                    let mut keys: Vec<Option<SpreadObjectKey>> = Vec::with_capacity(props.len());
                    let mut vals: Vec<Operand> = Vec::with_capacity(props.len());
                    for p in props {
                        match &p.key {
                            ObjectKey::Spread => {
                                keys.push(None);
                                vals.push(self.convert_expr(&p.value));
                            }
                            ObjectKey::Static(s) => {
                                let sid = self.builder.intern(s);
                                keys.push(Some(SpreadObjectKey::Static(sid)));
                                vals.push(self.convert_expr(&p.value));
                            }
                            ObjectKey::Computed(e) => {
                                let key_op = self.convert_expr(e);
                                keys.push(Some(SpreadObjectKey::Computed(key_op)));
                                vals.push(self.convert_expr(&p.value));
                            }
                        }
                    }
                    self.emit(InstKind::ObjectWithSpread { keys, vals })
                } else {
                    // Per JS spec: for each prop, key is evaluated before
                    // value (computed keys); for static keys the order is
                    // unobservable. We pre-evaluate everything so element
                    // effects are sequenced before the alloc, then alloc,
                    // then store. Pure stores: target has not escaped.
                    enum EvalKey {
                        Static(StrId),
                        Computed(Operand),
                    }
                    let evaluated: Vec<(EvalKey, Operand)> = props
                        .iter()
                        .map(|p| {
                            let k = match &p.key {
                                ObjectKey::Static(s) => EvalKey::Static(self.builder.intern(s)),
                                ObjectKey::Computed(e) => EvalKey::Computed(self.convert_expr(e)),
                                ObjectKey::Spread => unreachable!("no spread in this branch"),
                            };
                            let v = self.convert_expr(&p.value);
                            (k, v)
                        })
                        .collect();
                    let obj = self.emit(InstKind::AllocObject);
                    for (k, v) in evaluated {
                        match k {
                            EvalKey::Static(key) => {
                                self.builder.add_inst(InstKind::StoreProp {
                                    obj: obj.clone(),
                                    key,
                                    val: v,
                                    purity: Purity::Pure,
                                });
                            }
                            EvalKey::Computed(idx) => {
                                self.builder.add_inst(InstKind::StoreIndex {
                                    obj: obj.clone(),
                                    index: idx,
                                    val: v,
                                    purity: Purity::Pure,
                                });
                            }
                        }
                    }
                    obj
                }
            }
            Expr::Ternary {
                cond,
                then_branch,
                else_branch,
            } => self.convert_ternary(cond, then_branch, else_branch),
            Expr::Update { op, target, prefix } => self.convert_update(*op, target, *prefix),
            Expr::Spread(_) => {
                // Spread is handled by the enclosing Call/Array/Object arm; a
                // bare Spread at the expression level is a lowering bug.
                panic!("Expr::Spread encountered outside of a Call/Array/Object context — lowering bug");
            }
            Expr::Sequence { exprs } => {
                // Comma operator: walk each operand for side effects;
                // the value of the expression is the last operand. The
                // bundler-interop pattern `(0, obj.method)(args)` is
                // preserved correctly because `Expr::Call` only
                // specializes a literal `Expr::Member` callee into
                // `CallMethod`; a Sequence callee falls through to the
                // plain `Call` path with `this === undefined`.
                debug_assert!(!exprs.is_empty(), "Expr::Sequence must be non-empty");
                let mut last = None;
                for e in exprs {
                    last = Some(self.convert_expr(e));
                }
                last.expect("Sequence non-empty by debug_assert")
            }
            Expr::Assign { target, value } => {
                // Plain `=` in expression position: emit the same
                // side-effecting store as `Stmt::Assign` and yield the
                // RHS as the expression's value (per ECMA-262 13.15
                // for plain bindings and properties without setters).
                //
                // Ident targets here always synthesize a
                // `StoreProp(GlobalThis, name, val)` regardless of
                // whether `name` is in `pt_referenced_names`. The
                // tree reconstruction inlines branch-condition values,
                // which would otherwise drop the binding's write
                // (silently producing a body that reads an unbound
                // `name`). The synthesized store resugars textually to
                // `name = val` and resolves lexically to the local
                // let-binding at runtime — equivalent to writing the
                // tagged-name through a forced let-decl. See
                // `collect_assign_expr_target_names` for the related
                // `pt_referenced_names` extension.
                let val_op = self.convert_expr(value);
                match target.as_ref() {
                    Expr::Ident(name) => {
                        if let Some(sym) = self.lookup(name) {
                            self.tag_binding_source_name(val_op.clone(), name);
                            let cb = self.current_block();
                            self.write_variable(sym, cb, val_op.clone());
                            self.builder.mark_forced_let_decl(name);
                            let key = self.builder.intern(name);
                            self.builder.add_inst(InstKind::StoreProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                val: val_op.clone(),
                                purity: Purity::Throws,
                            });
                        } else {
                            // Unbound assignment-as-expression: store on globalThis.
                            let key = self.builder.intern(name);
                            self.builder.add_inst(InstKind::StoreProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                val: val_op.clone(),
                                purity: Purity::Throws,
                            });
                        }
                    }
                    Expr::Member { obj, prop } => {
                        let obj_op = self.convert_expr(obj);
                        let key = self.builder.intern(prop);
                        let purity = property_access_purity(&obj_op);
                        self.builder.add_inst(InstKind::StoreProp {
                            obj: obj_op,
                            key,
                            val: val_op.clone(),
                            purity,
                        });
                    }
                    Expr::Index { obj, index } => {
                        let obj_op = self.convert_expr(obj);
                        let idx_op = self.convert_expr(index);
                        let purity = property_access_purity(&obj_op);
                        self.builder.add_inst(InstKind::StoreIndex {
                            obj: obj_op,
                            index: idx_op,
                            val: val_op.clone(),
                            purity,
                        });
                    }
                    _ => panic!("Expr::Assign with unsupported target: {target:?}"),
                }
                val_op
            }
            Expr::CompoundAssign { op, target, value } => {
                // `lhs op= rhs` in expression position. Decompose into
                // a single LHS evaluation, a load, the binary op, a
                // store, and yield the computed value. Mirrors
                // `convert_update` but uses the requested binary op.
                // Spec ordering: evaluate receiver, evaluate key,
                // load, evaluate rhs, op, store.
                match target.as_ref() {
                    Expr::Ident(name) => {
                        if let Some(sym) = self.lookup(name) {
                            let cb0 = self.current_block();
                            let old = self.read_variable(sym, cb0);
                            let rhs_op = self.convert_expr(value);
                            let new = self.emit_binop_for_compound(*op, old, rhs_op);
                            self.tag_binding_source_name(new.clone(), name);
                            let cb = self.current_block();
                            self.write_variable(sym, cb, new.clone());
                            self.builder.mark_forced_let_decl(name);
                            let key = self.builder.intern(name);
                            self.builder.add_inst(InstKind::StoreProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                val: new.clone(),
                                purity: Purity::Throws,
                            });
                            new
                        } else {
                            // Unbound: read+write through globalThis.
                            let key = self.builder.intern(name);
                            let old = self.emit(InstKind::LoadProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                purity: Purity::Throws,
                            });
                            let rhs_op = self.convert_expr(value);
                            let new = self.emit_binop_for_compound(*op, old, rhs_op);
                            self.builder.add_inst(InstKind::StoreProp {
                                obj: Operand::Const(Constant::GlobalThis),
                                key,
                                val: new.clone(),
                                purity: Purity::Throws,
                            });
                            new
                        }
                    }
                    Expr::Member { obj, prop } => {
                        let obj_op = self.convert_expr(obj);
                        let key = self.builder.intern(prop);
                        let purity = property_access_purity(&obj_op);
                        let old = self.emit(InstKind::LoadProp {
                            obj: obj_op.clone(),
                            key,
                            purity,
                        });
                        let rhs_op = self.convert_expr(value);
                        let new = self.emit_binop_for_compound(*op, old, rhs_op);
                        self.builder.add_inst(InstKind::StoreProp {
                            obj: obj_op,
                            key,
                            val: new.clone(),
                            purity,
                        });
                        new
                    }
                    Expr::Index { obj, index } => {
                        let obj_op = self.convert_expr(obj);
                        let idx_op = self.convert_expr(index);
                        let purity = property_access_purity(&obj_op);
                        let old = self.emit(InstKind::LoadIndex {
                            obj: obj_op.clone(),
                            index: idx_op.clone(),
                            purity,
                        });
                        let rhs_op = self.convert_expr(value);
                        let new = self.emit_binop_for_compound(*op, old, rhs_op);
                        self.builder.add_inst(InstKind::StoreIndex {
                            obj: obj_op,
                            index: idx_op,
                            val: new.clone(),
                            purity,
                        });
                        new
                    }
                    _ => panic!(
                        "Expr::CompoundAssign with unsupported target: {target:?}"
                    ),
                }
            }
            Expr::Template { quasis, exprs } => {
                // Desugar `` `a${x}b${y}c` `` to a chain of AddGeneric:
                //   "a" + x -> ("a" + x) + "b" -> (("a" + x) + "b") + y -> ...
                // An empty template `` `hello` `` (no expressions) becomes
                // Const(String("hello")) directly.
                debug_assert_eq!(
                    quasis.len(),
                    exprs.len() + 1,
                    "template literal invariant: quasis.len() == exprs.len() + 1"
                );
                // Start with the first quasi.
                let first_id = self.builder.intern(&quasis[0]);
                let mut acc = Operand::Const(Constant::String(first_id));
                for (i, expr) in exprs.iter().enumerate() {
                    // Concatenate the expression.
                    let expr_op = self.convert_expr(expr);
                    acc = self.emit(InstKind::AddGeneric {
                        lhs: acc,
                        rhs: expr_op,
                    });
                    // Concatenate the following quasi (may be empty string).
                    let next_id = self.builder.intern(&quasis[i + 1]);
                    let next_op = Operand::Const(Constant::String(next_id));
                    // Skip adding empty trailing quasis to avoid redundant
                    // `+ ""` at the end — but only for the last position
                    // and only if the quasi is truly empty, because
                    // `something + ""` has the same value but adds an
                    // instruction. Always include non-empty quasis and the
                    // intermediate ones (they affect string splitting).
                    if !quasis[i + 1].is_empty() || i + 1 < exprs.len() {
                        acc = self.emit(InstKind::AddGeneric {
                            lhs: acc,
                            rhs: next_op,
                        });
                    }
                }
                acc
            }
        }
    }

    /// Lower `cond ? a : b`. Same control-flow shape as `if (cond) { t
    /// = a } else { t = b }; t`: write the value into a fresh symbol
    /// in each branch, then read it after the join — Braun inserts
    /// the phi automatically.
    fn convert_ternary(&mut self, cond: &Expr, then_e: &Expr, else_e: &Expr) -> Operand {
        let cond_op = self.convert_expr(cond);
        let then_b = self.builder.new_block();
        let else_b = self.builder.new_block();
        let join_b = self.builder.new_block();

        self.builder.set_terminator(Terminator::CondBranch {
            cond: cond_op,
            then_b,
            then_args: vec![],
            else_b,
            else_args: vec![],
        });
        self.sealed.insert(then_b);
        self.sealed.insert(else_b);

        // Synthesized symbol for the ternary's value. No source name —
        // the renamer will give it a `_tN`-style hoisted-temporary name.
        let sym = self.fresh_symbol();

        self.builder.switch_to_block(then_b);
        let v_then = self.convert_expr(then_e);
        let then_end = self.current_block();
        self.write_variable(sym, then_end, v_then);
        if !self.is_terminated(then_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: join_b,
                args: vec![],
            });
        }

        self.builder.switch_to_block(else_b);
        let v_else = self.convert_expr(else_e);
        let else_end = self.current_block();
        self.write_variable(sym, else_end, v_else);
        if !self.is_terminated(else_end) {
            self.builder.set_terminator(Terminator::Branch {
                target: join_b,
                args: vec![],
            });
        }

        self.seal_block(join_b);
        self.builder.switch_to_block(join_b);
        self.read_variable(sym, join_b)
    }

    /// Lower `++x`, `x++`, `--x`, `x--`. The receiver and key (if any)
    /// of the target are evaluated exactly once before the read+write,
    /// then the operator emits a binary `+ 1` / `- 1` and writes back.
    /// Returns the old value for postfix, the new value for prefix —
    /// matching JS semantics for numeric inputs (non-numeric inputs
    /// diverge on `ToNumber` coercion; see docs/DECISIONS.md).
    /// Emit the binary instruction backing a compound assignment's
    /// op (e.g. `BinOp::Add` for `+=`). Centralised so the SSA op
    /// mapping stays in one place; `convert_binary` already handles
    /// the same set for plain binary expressions, but compound never
    /// wants the comparison/logical/short-circuit branches and a
    /// few of those would be ill-formed (`=== `, `&&` etc. are not
    /// valid compound ops at parse time, but enumerating only the
    /// arithmetic/bitwise subset keeps the panic helpful).
    fn emit_binop_for_compound(
        &mut self,
        op: BinOp,
        lhs: Operand,
        rhs: Operand,
    ) -> Operand {
        let kind = match op {
            BinOp::Add => InstKind::AddGeneric { lhs, rhs },
            BinOp::Sub => InstKind::SubGeneric { lhs, rhs },
            BinOp::Mul => InstKind::MulGeneric { lhs, rhs },
            BinOp::Div => InstKind::DivGeneric { lhs, rhs },
            BinOp::Mod => InstKind::ModGeneric { lhs, rhs },
            BinOp::Pow => InstKind::PowGeneric { lhs, rhs },
            BinOp::BitAnd => InstKind::BitAnd { lhs, rhs },
            BinOp::BitOr => InstKind::BitOr { lhs, rhs },
            BinOp::BitXor => InstKind::BitXor { lhs, rhs },
            BinOp::Shl => InstKind::Shl { lhs, rhs },
            BinOp::Shr => InstKind::Shr { lhs, rhs },
            BinOp::UShr => InstKind::UShr { lhs, rhs },
            other => panic!("invalid compound-assignment operator: {other:?}"),
        };
        self.emit(kind)
    }

    fn convert_update(&mut self, op: UpdateOp, target: &Expr, prefix: bool) -> Operand {
        match target {
            Expr::Ident(name) => {
                if let Some(sym) = self.lookup(name) {
                    let cb = self.current_block();
                    let old = self.read_variable(sym, cb);
                    let new = self.emit(make_inc_dec(op, old.clone()));
                    self.tag_binding_source_name(new.clone(), name);
                    let cb2 = self.current_block();
                    self.write_variable(sym, cb2, new.clone());
                    if self.pt_referenced_names.contains(name) {
                        let key = self.builder.intern(name);
                        self.builder.add_inst(InstKind::StoreProp {
                            obj: Operand::Const(Constant::GlobalThis),
                            key,
                            val: new.clone(),
                            purity: Purity::Throws,
                        });
                    }
                    if prefix { new } else { old }
                } else {
                    // Unbound update: lower as global property load + store.
                    let key = self.builder.intern(name);
                    let old = self.emit(InstKind::LoadProp {
                        obj: Operand::Const(Constant::GlobalThis),
                        key,
                        purity: Purity::Throws,
                    });
                    let new = self.emit(make_inc_dec(op, old.clone()));
                    self.builder.add_inst(InstKind::StoreProp {
                        obj: Operand::Const(Constant::GlobalThis),
                        key,
                        val: new.clone(),
                        purity: Purity::Throws,
                    });
                    if prefix { new } else { old }
                }
            }
            Expr::Member { obj, prop } => {
                let obj_op = self.convert_expr(obj);
                let key = self.builder.intern(prop);
                let purity = property_access_purity(&obj_op);
                let old = self.emit(InstKind::LoadProp {
                    obj: obj_op.clone(),
                    key,
                    purity,
                });
                let new = self.emit(make_inc_dec(op, old.clone()));
                self.builder.add_inst(InstKind::StoreProp {
                    obj: obj_op,
                    key,
                    val: new.clone(),
                    purity,
                });
                if prefix { new } else { old }
            }
            Expr::Index { obj, index } => {
                let obj_op = self.convert_expr(obj);
                let idx_op = self.convert_expr(index);
                let purity = property_access_purity(&obj_op);
                let old = self.emit(InstKind::LoadIndex {
                    obj: obj_op.clone(),
                    index: idx_op.clone(),
                    purity,
                });
                let new = self.emit(make_inc_dec(op, old.clone()));
                self.builder.add_inst(InstKind::StoreIndex {
                    obj: obj_op,
                    index: idx_op,
                    val: new.clone(),
                    purity,
                });
                if prefix { new } else { old }
            }
            other => panic!("invalid update target: {other:?}"),
        }
    }

    /// Resolve a nested literal's pre-allocated FuncId and its captures
    /// list, then emit an `AllocClosure` whose capture operands are
    /// each name's value in the enclosing scope (this converter's
    /// `lookup` + Braun read). The returned `Operand::Value` is the
    /// freshly allocated closure, ready to bind or pass on.
    fn alloc_closure_operand(&mut self, lit: &FuncLiteral) -> Operand {
        let fid = lit
            .func_id
            .get()
            .expect("inner FuncLiteral missing FuncId — pass 1 must have stamped it");

        // Capture-name list is read from the shared `inner_captures`
        // map rather than re-derived here, so the inner converter and
        // this side agree on order.
        let names: Vec<String> = self.inner_captures.get(&fid).cloned().unwrap_or_default();
        let target_is_passthrough = self.passthrough_fids.contains(&fid);
        let mut captures: Vec<Operand> = Vec::with_capacity(names.len());
        for name in &names {
            match self.lookup(name) {
                Some(sym) => {
                    let cb = self.current_block();
                    captures.push(self.read_variable(sym, cb));
                }
                None if target_is_passthrough => {
                    // Forward reference into a PassThrough body —
                    // the name will be bound later in the outer
                    // (the closures-disposition-rule routes such
                    // captures to PT precisely so the body re-emits
                    // as text and resolves lexically at runtime).
                    // Use an undefined placeholder for the SSA
                    // capture slot; a `forced_let_decls` entry will
                    // ensure the binding declaration exists, and
                    // the synthesized StoreProp at the eventual
                    // `Stmt::Let` site supplies the real value.
                    self.builder.mark_forced_let_decl(name);
                    captures.push(Operand::Const(Constant::Undefined));
                }
                None => {
                    panic!(
                        "AllocClosure capture `{}` for f{} is not in scope at allocation site",
                        name, fid.0
                    );
                }
            }
        }
        // If the AllocClosure target has a PassThrough body, every
        // non-Const capture must be pinned at emit time so the
        // PassThrough body's lexical references survive renaming. The
        // transitive case (target is Lowered but its own body
        // recursively flows captures into a PassThrough) is handled by
        // the `propagate_passthrough_captures` fixed-point pass after
        // all functions are built. Note: with B's design, inner
        // PassThrough literals short-circuit above with empty captures
        // (their lexical names are pinned via the outer's source_name
        // chain rather than via SSA-level captures). The branch below
        // therefore mostly fires for the sibling case: a Lowered
        // function AllocClosure-ing a top-level PassThrough.
        if self.passthrough_fids.contains(&fid) {
            for op in captures.iter() {
                match op {
                    Operand::Value(v) => {
                        self.builder.mark_value_captured_by_passthrough(*v);
                    }
                    Operand::Capture(idx) => {
                        self.builder.mark_capture_pass_through_onward(*idx);
                    }
                    Operand::Const(_) => {}
                }
            }
        }
        self.emit(InstKind::AllocClosure {
            func: fid,
            captures,
        })
    }

    /// Emit an instruction whose result is needed as an `Operand`. Panics if
    /// the InstKind is void (a store) — call sites use this only for
    /// value-producing kinds.
    fn emit(&mut self, kind: InstKind) -> Operand {
        let v = self
            .builder
            .add_inst(kind)
            .expect("emit() called on a void instruction");
        Operand::Value(v)
    }

    fn convert_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        // Logical operators short-circuit, so they can't be lowered through
        // the value-producing LogAnd/LogOr instructions (which are pure,
        // both-sides-evaluated). Branch instead.
        if matches!(op, BinOp::LogAnd | BinOp::LogOr) {
            return self.convert_short_circuit(op, lhs, rhs);
        }

        let lhs_op = self.convert_expr(lhs);
        let rhs_op = self.convert_expr(rhs);
        let kind = match op {
            BinOp::Add => InstKind::AddGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Sub => InstKind::SubGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Mul => InstKind::MulGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Div => InstKind::DivGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Mod => InstKind::ModGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Eq => InstKind::Eq {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::StrictEq => InstKind::StrictEq {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Ne => {
                let inner = self.emit(InstKind::Eq {
                    lhs: lhs_op,
                    rhs: rhs_op,
                });
                return self.emit(InstKind::Not { operand: inner });
            }
            BinOp::StrictNe => {
                let inner = self.emit(InstKind::StrictEq {
                    lhs: lhs_op,
                    rhs: rhs_op,
                });
                return self.emit(InstKind::Not { operand: inner });
            }
            BinOp::Lt => InstKind::Lt {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Le => InstKind::Le {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            // a > b  ≡  b < a; a >= b  ≡  b <= a. Reordering is sound only
            // because both operands are already evaluated above (left-to-right
            // observable order is preserved).
            BinOp::Gt => InstKind::Lt {
                lhs: rhs_op,
                rhs: lhs_op,
            },
            BinOp::Ge => InstKind::Le {
                lhs: rhs_op,
                rhs: lhs_op,
            },
            BinOp::LogAnd | BinOp::LogOr => unreachable!("handled above"),
            BinOp::Pow => InstKind::PowGeneric {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::BitAnd => InstKind::BitAnd {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::BitOr => InstKind::BitOr {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::BitXor => InstKind::BitXor {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Shl => InstKind::Shl {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::Shr => InstKind::Shr {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::UShr => InstKind::UShr {
                lhs: lhs_op,
                rhs: rhs_op,
            },
            BinOp::In => InstKind::In {
                key: lhs_op,
                obj: rhs_op,
            },
            BinOp::Instanceof => InstKind::Instanceof {
                obj: lhs_op,
                ctor: rhs_op,
            },
        };
        self.emit(kind)
    }

    fn convert_short_circuit(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Operand {
        // && returns lhs if lhs is falsy, else rhs.
        // || returns lhs if lhs is truthy, else rhs.
        // Lowering: branch on lhs into either an "evaluate rhs" block or a
        // "skip" block that just forwards lhs. The join takes a single block
        // param holding the result.
        let lhs_op = self.convert_expr(lhs);

        let rhs_b = self.builder.new_block();
        let skip_b = self.builder.new_block();
        let join_b = self.builder.new_block();

        let (then_b, else_b) = match op {
            BinOp::LogAnd => (rhs_b, skip_b),
            BinOp::LogOr => (skip_b, rhs_b),
            _ => unreachable!(),
        };

        self.builder.set_terminator(Terminator::CondBranch {
            cond: lhs_op,
            then_b,
            then_args: vec![],
            else_b,
            else_args: vec![],
        });
        self.sealed.insert(rhs_b);
        self.sealed.insert(skip_b);

        // rhs evaluation block
        self.builder.switch_to_block(rhs_b);
        let rhs_op = self.convert_expr(rhs);
        let rhs_end = self.current_block();
        self.builder.body.blocks[rhs_end.0 as usize].terminator = Terminator::Branch {
            target: join_b,
            args: vec![rhs_op],
        };

        // skip block — forwards lhs unchanged
        self.builder.switch_to_block(skip_b);
        self.builder.set_terminator(Terminator::Branch {
            target: join_b,
            args: vec![lhs_op],
        });

        // join: result is the param. Manually add it (we know the arity)
        // so we don't go through Braun for a synthetic temporary.
        let result = self.builder.add_block_param(join_b);
        self.sealed.insert(join_b);
        self.builder.switch_to_block(join_b);
        Operand::Value(result)
    }

    fn convert_unary(&mut self, op: UnOp, operand: &Expr) -> Operand {
        let v = self.convert_expr(operand);
        let kind = match op {
            UnOp::Not => InstKind::Not { operand: v },
            UnOp::Pos => InstKind::ToNumber { val: v },
            UnOp::Typeof => InstKind::Typeof { val: v },
            UnOp::Void => {
                // `void e` evaluates e for effects, then yields undefined.
                // The convert_expr above already evaluated e.
                return Operand::Const(Constant::Undefined);
            }
            UnOp::Neg => {
                // Decompose `-x` as `(-0.0) - x`.  The lhs MUST be -0.0, not
                // 0.0: IEEE 754 says -(0.0) == -0.0, but 0.0 - 0.0 == +0.0.
                // JS follows IEEE 754, so a roundtrip through 0.0 - x silently
                // flips the sign of zero.  See also: LLVM's fneg history.
                return self.emit(InstKind::SubGeneric {
                    lhs: Operand::Const(Constant::Number(-0.0_f64)),
                    rhs: v,
                });
            }
            UnOp::BitNot => InstKind::BitNot { operand: v },
        };
        self.emit(kind)
    }

    // ------------------------------------------------------------------
    // Helpers over the under-construction function
    // ------------------------------------------------------------------

    fn current_block(&self) -> BlockId {
        self.builder.current_block().expect("no current block")
    }

    fn is_terminated(&self, b: BlockId) -> bool {
        !matches!(
            self.builder.body.blocks[b.0 as usize].terminator,
            Terminator::Unreachable
        )
    }

    /// Predecessors of `target`, derived from terminator successors. Returns
    /// each source block exactly once even if multiple edges exist (shouldn't
    /// happen in our converter, but guards against degenerate IRs).
    fn predecessors_of(&self, target: BlockId) -> Vec<BlockId> {
        let mut out = Vec::new();
        for (i, blk) in self.builder.body.blocks.iter().enumerate() {
            for (succ, _) in blk.terminator.successors() {
                if succ == target {
                    let src = BlockId(i as u32);
                    if !out.contains(&src) {
                        out.push(src);
                    }
                    break;
                }
            }
        }
        out
    }

    /// Operand at `index` in `src`'s edge args targeting `dst`.
    fn edge_arg_at(&self, src: BlockId, dst: BlockId, index: usize) -> Option<Operand> {
        let blk = &self.builder.body.blocks[src.0 as usize];
        edge_args(&blk.terminator, dst).and_then(|args| args.get(index).copied())
    }

    /// Append `op` to `src`'s edge args targeting `dst`.
    fn append_edge_arg(&mut self, src: BlockId, dst: BlockId, op: Operand) {
        let blk = &mut self.builder.body.blocks[src.0 as usize];
        push_edge_arg(&mut blk.terminator, dst, op);
    }

    /// Strip the operand at `index` from `src`'s edge args targeting `dst`.
    fn remove_edge_arg_at(&mut self, src: BlockId, dst: BlockId, index: usize) {
        let blk = &mut self.builder.body.blocks[src.0 as usize];
        remove_edge_arg(&mut blk.terminator, dst, index);
    }

    /// Drop `block.params[idx]`, then shift the trailing params' indices
    /// down by one in their `ValueDef`s so block-param value lookups stay
    /// consistent.
    fn remove_block_param(&mut self, block: BlockId, idx: usize) {
        let blk = &mut self.builder.body.blocks[block.0 as usize];
        blk.params.remove(idx);
        // Snapshot indices to avoid borrowing both `blocks` and `values` at
        // once.
        let trailing: Vec<Value> = blk.params[idx..].to_vec();
        for (offset, v) in trailing.iter().enumerate() {
            if let ValueDefKind::BlockParam { index, .. } =
                &mut self.builder.body.values[v.0 as usize].kind
            {
                *index = (idx + offset) as u32;
            }
        }
    }

    /// Find every block-param Value whose incoming arg references `target`.
    /// Used to drive the recursive trivial-removal cascade after a removal:
    /// any block param that had `Operand::Value(target)` as one of its
    /// incomings may have just become trivial.
    fn find_block_param_users(&self, target: Value) -> Vec<Value> {
        let mut out = Vec::new();
        for blk in &self.builder.body.blocks {
            for (succ, args) in blk.terminator.successors() {
                let succ_blk = &self.builder.body.blocks[succ.0 as usize];
                for (i, arg) in args.iter().enumerate() {
                    if let Operand::Value(v) = arg
                        && *v == target
                        && i < succ_blk.params.len()
                    {
                        let user = succ_blk.params[i];
                        if !out.contains(&user) {
                            out.push(user);
                        }
                    }
                }
            }
        }
        out
    }

    /// Substitute every `Operand::Value(old)` with `new` across all
    /// instructions, all terminators, and the converter's own `current_def`.
    fn replace_value_uses(&mut self, old: Value, new: Operand) {
        for inst in &mut self.builder.body.instructions {
            walk_inst_operands_mut(&mut inst.kind, &mut |op| {
                if let Operand::Value(v) = op
                    && *v == old
                {
                    *op = new;
                }
            });
        }
        for blk in &mut self.builder.body.blocks {
            walk_term_operands_mut(&mut blk.terminator, &mut |op| {
                if let Operand::Value(v) = op
                    && *v == old
                {
                    *op = new;
                }
            });
        }
        for op in self.current_def.values_mut() {
            if let Operand::Value(v) = op
                && *v == old
            {
                *op = new;
            }
        }
    }
}

// ============================================================================
// Free helpers (don't need Converter state)
// ============================================================================

/// Recognize identifiers whose JS semantics is "always evaluate to a fixed
/// constant" — applied only when the name is not lexically bound (the caller
/// checks lookup() first, so a local `let undefined = 1` shadows correctly).
///
/// `NaN` and `Infinity` use the existing `Constant::Number` variants rather
/// than dedicated variants: a future constant-folding pass that produces NaN
/// from `0/0` (or Infinity from `1/0`) will yield the same canonical form,
/// avoiding two ways to spell the same value.
fn special_global_constant(name: &str) -> Option<Constant> {
    match name {
        "globalThis" | "window" | "self" | "global" => Some(Constant::GlobalThis),
        "undefined" => Some(Constant::Undefined),
        "NaN" => Some(Constant::Number(f64::NAN)),
        "Infinity" => Some(Constant::Number(f64::INFINITY)),
        _ => None,
    }
}

/// Pick the `Purity` tier for a property access whose object operand is
/// `obj`. Accesses rooted at `Const(GlobalThis)` get `Throws` (they read or
/// write the global record and may throw, but don't trigger user-defined
/// getters/setters under our semantic model); everything else stays
/// `Opaque`. Without this, `globalThis.foo` would lower to `Opaque` while
/// the bare unbound read of `foo` would lower to `Throws` — the same
/// observable JS denoted by two different SSA shapes, defeating CSE on
/// equivalent rewrites.
/// Build the increment/decrement instruction for an `Expr::Update`'s
/// read-modify-write step. Always uses the generic numeric-with-coercion
/// variant — the IR has no dedicated "always-number" variant and adding
/// one for `++`/`--` would be premature.
fn make_inc_dec(op: UpdateOp, old: Operand) -> InstKind {
    let one = Operand::Const(Constant::Number(1.0));
    match op {
        UpdateOp::Inc => InstKind::AddGeneric { lhs: old, rhs: one },
        UpdateOp::Dec => InstKind::SubGeneric { lhs: old, rhs: one },
    }
}

fn property_access_purity(obj: &Operand) -> Purity {
    match obj {
        Operand::Const(Constant::GlobalThis) => Purity::Throws,
        _ => Purity::Opaque,
    }
}

fn operand_eq(a: &Operand, b: &Operand) -> bool {
    match (a, b) {
        (Operand::Value(x), Operand::Value(y)) => x == y,
        (Operand::Const(x), Operand::Const(y)) => const_bit_eq(x, y),
        _ => false,
    }
}

fn const_bit_eq(a: &Constant, b: &Constant) -> bool {
    use Constant::*;
    match (a, b) {
        (Number(x), Number(y)) => x.to_bits() == y.to_bits(),
        (Int32(x), Int32(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Null, Null) => true,
        (Undefined, Undefined) => true,
        (Func(x), Func(y)) => x.0 == y.0,
        (GlobalThis, GlobalThis) => true,
        _ => false,
    }
}

/// Edge args from terminator `t` targeting `dst`. Returns the *first*
/// arg-list found that targets `dst`. Multiple-edge cases (switch with
/// duplicate targets, or a degenerate `cbr cond, L, L`) are not generated by
/// this converter.
fn edge_args(t: &Terminator, dst: BlockId) -> Option<&[Operand]> {
    match t {
        Terminator::Branch { target, args } if *target == dst => Some(args.as_slice()),
        Terminator::CondBranch {
            then_b, then_args, ..
        } if *then_b == dst => Some(then_args.as_slice()),
        Terminator::CondBranch {
            else_b, else_args, ..
        } if *else_b == dst => Some(else_args.as_slice()),
        Terminator::Switch {
            cases,
            default,
            default_args,
            ..
        } => {
            for c in cases {
                if c.target == dst {
                    return Some(c.args.as_slice());
                }
            }
            if *default == dst {
                Some(default_args.as_slice())
            } else {
                None
            }
        }
        _ => None,
    }
}

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
        _ => panic!("push_edge_arg: terminator does not target bb{}", dst.0),
    }
}

fn remove_edge_arg(t: &mut Terminator, dst: BlockId, index: usize) {
    match t {
        Terminator::Branch { target, args } if *target == dst => {
            args.remove(index);
        }
        Terminator::CondBranch {
            then_b,
            then_args,
            else_b,
            else_args,
            ..
        } => {
            if *then_b == dst {
                then_args.remove(index);
            }
            if *else_b == dst {
                else_args.remove(index);
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
                    c.args.remove(index);
                }
            }
            if *default == dst {
                default_args.remove(index);
            }
        }
        _ => panic!("remove_edge_arg: terminator does not target bb{}", dst.0),
    }
}

// ----------------------------------------------------------------------
// Mutable operand walks. Mirrors the immutable walks in `ssa.rs` but
// threads `&mut Operand` so trivial-removal can substitute in place.
// ----------------------------------------------------------------------

fn walk_inst_operands_mut(kind: &mut InstKind, f: &mut impl FnMut(&mut Operand)) {
    use InstKind::*;
    match kind {
        AddGeneric { lhs, rhs }
        | SubGeneric { lhs, rhs }
        | MulGeneric { lhs, rhs }
        | DivGeneric { lhs, rhs }
        | ModGeneric { lhs, rhs }
        | PowGeneric { lhs, rhs }
        | BitAnd { lhs, rhs }
        | BitOr { lhs, rhs }
        | BitXor { lhs, rhs }
        | Shl { lhs, rhs }
        | Shr { lhs, rhs }
        | UShr { lhs, rhs }
        | AddNumber { lhs, rhs }
        | SubNumber { lhs, rhs }
        | MulNumber { lhs, rhs }
        | AddInt32 { lhs, rhs }
        | SubInt32 { lhs, rhs }
        | MulInt32 { lhs, rhs }
        | ConcatStr { lhs, rhs }
        | LogAnd { lhs, rhs }
        | LogOr { lhs, rhs }
        | Eq { lhs, rhs }
        | StrictEq { lhs, rhs }
        | Lt { lhs, rhs }
        | Le { lhs, rhs } => {
            f(lhs);
            f(rhs);
        }
        Not { operand } | BitNot { operand } => f(operand),
        LoadProp { obj, .. } => f(obj),
        StoreProp { obj, val, .. } => {
            f(obj);
            f(val);
        }
        LoadIndex { obj, index, .. } => {
            f(obj);
            f(index);
        }
        StoreIndex {
            obj, index, val, ..
        } => {
            f(obj);
            f(index);
            f(val);
        }
        AllocObject | AllocArray { .. } => {}
        AllocClosure { captures, .. } => {
            for c in captures {
                f(c);
            }
        }
        Call { callee, args } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallSpread { callee, args, .. } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallMethod { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        CallMethodSpread { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        ArrayWithSpread { elements, .. } => {
            for e in elements {
                f(e);
            }
        }
        ObjectWithSpread { keys, vals } => {
            for k in keys {
                if let Some(ir::ssa::SpreadObjectKey::Computed(op)) = k {
                    f(op);
                }
            }
            for v in vals {
                f(v);
            }
        }
        Construct { ctor, args } => {
            f(ctor);
            for a in args {
                f(a);
            }
        }
        ToNumber { val }
        | ToString { val }
        | ToBool { val }
        | ToInt32 { val }
        | Typeof { val }
        | OpaqueEval { source: val } => f(val),
        Instanceof { obj, ctor } => {
            f(obj);
            f(ctor);
        }
        In { key, obj } => {
            f(key);
            f(obj);
        }
    }
}

fn walk_term_operands_mut(t: &mut Terminator, f: &mut impl FnMut(&mut Operand)) {
    match t {
        Terminator::Branch { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Terminator::CondBranch {
            cond,
            then_args,
            else_args,
            ..
        } => {
            f(cond);
            for a in then_args {
                f(a);
            }
            for a in else_args {
                f(a);
            }
        }
        Terminator::Switch {
            val,
            cases,
            default_args,
            ..
        } => {
            f(val);
            for c in cases {
                for a in &mut c.args {
                    f(a);
                }
            }
            for a in default_args {
                f(a);
            }
        }
        Terminator::Return { val: Some(v) } => f(v),
        Terminator::Return { val: None } => {}
        Terminator::Throw { val } => f(val),
        Terminator::Unreachable => {}
    }
}

// Silence dead-code lint on `BasicBlock` import — currently re-exported only
// because `walk_*_mut` and helpers may grow to need direct access in future
// changes. Kept here so the dependency is explicit.
#[allow(dead_code)]
fn _unused_basic_block(_: &BasicBlock) {}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ir::ssa::{Terminator, pretty_module};
    use crate::parse_and_lower;

    fn lower_js(src: &str) -> ssa::Module {
        let tm = parse_and_lower(src).expect("tree-IR lowering must succeed");
        let sm = lower_module(&tm);
        sm.validate().unwrap_or_else(|errs| {
            panic!(
                "SSA module failed validation:\n{:#?}\n--- dump ---\n{}",
                errs,
                pretty_module(&sm)
            );
        });
        sm
    }

    /// Count live block params (skipping any `Removed` slots, which would
    /// indicate a bug — block.params is shifted to compact on removal).
    fn live_param_count(func: &ssa::Function, b: BlockId) -> usize {
        func.lowered().blocks[b.0 as usize].params.len()
    }

    #[test]
    fn fibonacci_recursion() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let m = lower_js(src);
        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn sum_to_loop() {
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
        let m = lower_js(src);

        // The header must carry block params for both i and s (both vary
        // across the back-edge). Header is bb1 (entry=bb0, then header=bb1
        // by construction order).
        let header = BlockId(1);
        assert_eq!(
            live_param_count(&m.functions[0], header),
            2,
            "header should carry params for i and s"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn if_else_shared_mutation() {
        let src = r#"
            function f(c) {
                let x = 1;
                if (c) { x = 2; } else { x = 3; }
                return x;
            }
        "#;
        let m = lower_js(src);

        // The lowering is: entry=bb0, then_b=bb1, else_b=bb2, join=bb3.
        // join must carry exactly one param (for x).
        let join = BlockId(3);
        assert_eq!(
            live_param_count(&m.functions[0], join),
            1,
            "join block should carry exactly one param (for x)"
        );

        // The branches' edge args should be Const(2) and Const(3)
        // respectively — no instructions emitted in the branch bodies.
        let then_b = &m.functions[0].lowered().blocks[1];
        let else_b = &m.functions[0].lowered().blocks[2];
        match &then_b.terminator {
            Terminator::Branch { target, args } => {
                assert_eq!(*target, join);
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Operand::Const(Constant::Number(n)) if n == 2.0));
            }
            t => panic!("then branch should jump to join; got {t:?}"),
        }
        match &else_b.terminator {
            Terminator::Branch { target, args } => {
                assert_eq!(*target, join);
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Operand::Const(Constant::Number(n)) if n == 3.0));
            }
            t => panic!("else branch should jump to join; got {t:?}"),
        }

        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn lexical_shadowing() {
        let src = r#"
            function f() {
                let x = 1;
                if (true) { let x = 2; }
                return x;
            }
        "#;
        let m = lower_js(src);

        // The return reads the OUTER x, which was assigned 1 and is
        // unchanged (the inner x = 2 binds a different SymbolId). After
        // trivial-param removal, the return should literally be `ret 1`.
        let last_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .rev()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }));
        let ret_block = last_block.expect("function should have a Return terminator");
        match &ret_block.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => {
                assert_eq!(*n, 1.0, "return should yield outer x = 1");
            }
            t => panic!("return should be a literal Number(1); got {t:?}"),
        }

        insta::assert_snapshot!(pretty_module(&m));
    }

    #[test]
    fn trivial_param_elimination() {
        // The while loop modifies `i` (loop-varying) but not `x`
        // (loop-invariant). Braun would speculatively add a header param
        // for `x` on demand, but trivial-removal must collapse it because
        // both incomings (entry=Const(1), back-edge=same param) reduce to
        // Const(1).
        let src = r#"
            function f(n) {
                let x = 1;
                let i = 0;
                while (i < n) {
                    i = i + 1;
                }
                return x;
            }
        "#;
        let m = lower_js(src);

        // header is bb1 (entry=bb0, then header). It should carry exactly
        // one param — for `i`. If `x` survived the trivial-removal pass,
        // the count would be ≥2.
        let header = BlockId(1);
        assert_eq!(
            live_param_count(&m.functions[0], header),
            1,
            "header should carry exactly one param (for i); x must be removed"
        );

        // The exit block's return should read the constant 1, since x is
        // never modified in the loop.
        let exit_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .rev()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }));
        let exit = exit_block.expect("function should have a Return terminator");
        match &exit.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => {
                assert_eq!(*n, 1.0, "x is loop-invariant; return should fold to 1");
            }
            t => panic!("return should be a literal Number(1); got {t:?}"),
        }

        // No `Removed` ValueDef should appear under any *referenced* slot;
        // the validator already enforces this, but assert it explicitly to
        // catch regressions in trivial-removal bookkeeping.
        let removed_count = m.functions[0]
            .lowered()
            .values
            .iter()
            .filter(|v| v.is_removed())
            .count();
        // We expect at least one Removed entry (the eliminated param).
        assert!(
            removed_count >= 1,
            "expected at least one Removed value-def from trivial-param elimination"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    // ====================================================================
    // Globals model
    // ====================================================================

    /// `console.log("hi")` survives the SSA pass without panicking and
    /// produces a `LoadProp` rooted at `Const(GlobalThis)` for the global
    /// read of `console`. The `Throws` purity is the key signal: it's the
    /// proof that we're using the new precision tier rather than collapsing
    /// to Opaque.
    #[test]
    fn unbound_console_call_lowers_to_global_load() {
        let src = r#"function f() { console.log("hi"); }"#;
        let m = lower_js(src);

        let mut saw_global_load = false;
        for inst in &m.functions[0].lowered().instructions {
            if let InstKind::LoadProp {
                obj: Operand::Const(Constant::GlobalThis),
                key,
                purity,
            } = inst.kind
            {
                assert_eq!(purity, Purity::Throws, "global read should use Throws");
                assert_eq!(m.strings.resolve(&key), "console");
                saw_global_load = true;
            }
        }
        assert!(
            saw_global_load,
            "expected a LoadProp(GlobalThis, \"console\") instruction"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    /// `Math.floor(x)` exercises the same path as `console.log` plus a
    /// member-call rather than a method-call (the front end currently
    /// produces `Call(LoadProp(...), args)`, not `CallMethod`).
    #[test]
    fn unbound_math_floor_lowers_to_global_load() {
        let src = r#"function f(x) { return Math.floor(x); }"#;
        let m = lower_js(src);

        let saw_math = m.functions[0].lowered().instructions.iter().any(|i| {
            matches!(
                i.kind,
                InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    purity: Purity::Throws,
                    ..
                }
            )
        });
        assert!(saw_math, "expected a LoadProp(GlobalThis, \"Math\")");

        insta::assert_snapshot!(pretty_module(&m));
    }

    /// `x = 1` with `x` unbound becomes a global-store rooted at
    /// `Const(GlobalThis)`. Mirror of the read-side test.
    #[test]
    fn unbound_write_lowers_to_global_store() {
        let src = r#"function f() { x = 1; }"#;
        let m = lower_js(src);

        let mut saw_global_store = false;
        for inst in &m.functions[0].lowered().instructions {
            if let InstKind::StoreProp {
                obj: Operand::Const(Constant::GlobalThis),
                key,
                purity,
                ..
            } = inst.kind
            {
                assert_eq!(purity, Purity::Throws, "global write should use Throws");
                assert_eq!(m.strings.resolve(&key), "x");
                saw_global_store = true;
            }
        }
        assert!(
            saw_global_store,
            "expected a StoreProp(GlobalThis, \"x\", _) instruction"
        );

        insta::assert_snapshot!(pretty_module(&m));
    }

    /// `let y = undefined; return y;` — `undefined` is a special-case
    /// constant lookup, not a global read. Validates that `Constant::
    /// Undefined` (not a global LoadProp) reaches the return after
    /// trivial-param removal collapses the chain.
    #[test]
    fn undefined_lowers_to_constant_undefined() {
        let src = r#"function f() { let y = undefined; return y; }"#;
        let m = lower_js(src);

        let ret_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }))
            .expect("function should have a Return");
        match &ret_block.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Undefined)),
            } => {}
            t => panic!("expected `ret undefined`; got {t:?}"),
        }
    }

    /// `NaN` and `Infinity` use `Constant::Number(f64::NAN)` and
    /// `Constant::Number(f64::INFINITY)` — the existing canonical form
    /// rather than dedicated variants. This is the Q1 design decision: a
    /// future constant-folding pass that produces NaN from `0/0` agrees
    /// with the unbound-ident reads on the same canonical form, avoiding
    /// two ways to spell the same value.
    #[test]
    fn nan_and_infinity_use_number_constants() {
        for (src, expected_bits) in [
            ("function f() { return NaN; }", f64::NAN.to_bits()),
            ("function f() { return Infinity; }", f64::INFINITY.to_bits()),
        ] {
            let m = lower_js(src);
            let ret_block = m.functions[0]
                .lowered()
                .blocks
                .iter()
                .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }))
                .expect("function should have a Return");
            match &ret_block.terminator {
                Terminator::Return {
                    val: Some(Operand::Const(Constant::Number(n))),
                } => {
                    assert_eq!(
                        n.to_bits(),
                        expected_bits,
                        "expected canonical Number constant for src `{src}`"
                    );
                }
                t => panic!("expected `ret <Number>`; got {t:?} for src `{src}`"),
            }
        }
    }

    /// SSA pretty-printer annotates Inst-defined values with their source
    /// name, e.g. `%2["x"] = add ...`. Visible only when source_name was
    /// populated by lowering.
    #[test]
    fn ssa_dump_includes_source_name_annotation() {
        let src = r#"function f() { let x = 1 + 2; return x; }"#;
        let m = lower_js(src);
        let dump = pretty_module(&m);
        assert!(
            dump.contains("[\"x\"]"),
            "expected source-name annotation `[\"x\"]` in dump:\n{dump}"
        );
    }

    /// `globalThis.foo` (member access on the explicit `globalThis` ident)
    /// produces SSA byte-identical to the bare `foo`. Both share the same
    /// `LoadProp(GlobalThis, "foo", Throws)` shape — `globalThis` is
    /// recognized as a special-case constant on the read side, and the
    /// Member arm picks `Throws` purity for GlobalThis-rooted access.
    /// This is a *lossy* normalization: the user can no longer tell which
    /// spelling the source used, which is the correct trade-off for a
    /// deobfuscator (the obfuscator's choice of spelling is noise).
    #[test]
    fn globalthis_dot_foo_normalizes_to_bare_foo() {
        let with_explicit = lower_js("function f() { return globalThis.foo; }");
        let bare = lower_js("function f() { return foo; }");
        assert_eq!(
            pretty_module(&with_explicit),
            pretty_module(&bare),
            "globalThis.foo and bare foo must produce identical SSA"
        );
    }

    /// A local `let console = 1;` shadows the special-case lookup —
    /// reads of `console` resolve via the normal Braun path to the local
    /// binding, never to `LoadProp(GlobalThis, "console")`. The whole
    /// shadowing-supports-globals contract relies on lookup() running
    /// before special-case resolution.
    #[test]
    fn lexical_binding_shadows_global() {
        let src = r#"function f() { let console = 1; return console; }"#;
        let m = lower_js(src);

        // No global LoadProp anywhere in the function.
        let leaked = m.functions[0].lowered().instructions.iter().any(|i| {
            matches!(
                i.kind,
                InstKind::LoadProp {
                    obj: Operand::Const(Constant::GlobalThis),
                    ..
                }
            )
        });
        assert!(!leaked, "local `console` must shadow the global lookup");

        // After trivial-param removal the return collapses to `ret 1`.
        let ret_block = m.functions[0]
            .lowered()
            .blocks
            .iter()
            .find(|b| matches!(b.terminator, Terminator::Return { val: Some(_) }))
            .expect("function should have a Return");
        match &ret_block.terminator {
            Terminator::Return {
                val: Some(Operand::Const(Constant::Number(n))),
            } => {
                assert_eq!(*n, 1.0, "shadowed `console` should return 1");
            }
            t => panic!("expected `ret 1`; got {t:?}"),
        }
    }

    // ====================================================================
    // T2 Stream A: closure infrastructure smoke tests
    //
    // The front end (lower.rs) does not yet produce `Stmt::FuncDecl` /
    // `Expr::Func`, so these tests construct tree-IR by hand to prove the
    // pass-1 discovery, AllocClosure emission, and capture binding all
    // line up. They also lock in the contract that an inner literal's
    // FuncId is stamped on its `Cell` at discovery time.
    // ====================================================================

    use ir::tree::{FuncBody, FuncLiteral, Item, Module};
    use std::cell::Cell;

    fn fresh_lit(name: Option<&str>, params: Vec<&str>, body: FuncBody) -> FuncLiteral {
        FuncLiteral {
            name: name.map(String::from),
            params: params.into_iter().map(String::from).collect(),
            is_async: false,
            is_generator: false,
            is_arrow: false,
            body,
            func_id: Cell::new(None),
            passthrough_source: None,
            force_passthrough: Cell::new(false),
            blockers: std::collections::BTreeSet::new(),
        }
    }

    #[test]
    fn nested_funcdecl_lowers_to_alloc_closure() {
        // function outer(x) {
        //   function inner(y) { return x + y; } // captures x
        //   return inner(1);
        // }
        let inner = fresh_lit(
            Some("inner"),
            vec!["y"],
            FuncBody::Stmts(vec![Stmt::Return(Some(Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(Expr::Ident("x".to_string())),
                rhs: Box::new(Expr::Ident("y".to_string())),
            }))]),
        );
        let outer_body = vec![
            Stmt::FuncDecl(inner),
            Stmt::Return(Some(Expr::Call {
                callee: Box::new(Expr::Ident("inner".to_string())),
                args: vec![Expr::Number(1.0)],
            })),
        ];
        let outer = tree::IrFunction {
            name: "outer".to_string(),
            params: vec!["x".to_string()],
            body: outer_body,
            source: None,
        };
        let tm = Module {
            items: vec![Item::Function(outer)],
        };

        let sm = lower_module(&tm);
        sm.validate().unwrap_or_else(|errs| {
            panic!(
                "validation failed:\n{:#?}\n--- dump ---\n{}",
                errs,
                pretty_module(&sm)
            );
        });

        assert_eq!(sm.functions.len(), 2, "outer + inner");
        assert_eq!(sm.functions[0].name, "outer");
        assert_eq!(sm.functions[1].name, "inner");
        assert_eq!(
            sm.functions[1].capture_count, 1,
            "inner captures `x` from outer"
        );

        // Outer must contain an AllocClosure for FuncId(1) with one
        // capture sourced from outer's argument value.
        let mut saw_alloc = false;
        for inst in &sm.functions[0].lowered().instructions {
            if let InstKind::AllocClosure { func, captures } = &inst.kind {
                assert_eq!(func.0, 1, "AllocClosure should target inner FuncId");
                assert_eq!(captures.len(), 1);
                // The single capture is the outer's `x` parameter.
                match captures[0] {
                    Operand::Value(_) => {}
                    other => panic!("expected Value capture for x, got {other:?}"),
                }
                saw_alloc = true;
            }
        }
        assert!(saw_alloc, "outer should emit AllocClosure(@inner, [x])");

        // Inner must read x via `Operand::Capture(0)`. Walk inner's
        // instructions; any add referencing Capture(0) wins.
        let inner_body = sm.functions[1].lowered();
        let mut saw_capture = false;
        for inst in &inner_body.instructions {
            if let InstKind::AddGeneric { lhs, rhs } = &inst.kind
                && (matches!(lhs, Operand::Capture(0)) || matches!(rhs, Operand::Capture(0)))
            {
                saw_capture = true;
            }
        }
        assert!(
            saw_capture,
            "inner should reference the captured x via Operand::Capture(0)"
        );
    }

    #[test]
    fn nested_func_expr_in_let_lowers_to_alloc_closure() {
        // function outer(a) {
        //   let f = function (b) { return a + b; };
        //   return f;
        // }
        let inner = fresh_lit(
            None,
            vec!["b"],
            FuncBody::Stmts(vec![Stmt::Return(Some(Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(Expr::Ident("a".to_string())),
                rhs: Box::new(Expr::Ident("b".to_string())),
            }))]),
        );
        let outer = tree::IrFunction {
            name: "outer".to_string(),
            params: vec!["a".to_string()],
            body: vec![
                Stmt::Let {
                    name: "f".to_string(),
                    init: Some(Expr::Func(Box::new(inner))),
                },
                Stmt::Return(Some(Expr::Ident("f".to_string()))),
            ],
            source: None,
        };
        let tm = Module {
            items: vec![Item::Function(outer)],
        };
        let sm = lower_module(&tm);
        sm.validate().expect("validation must succeed");
        assert_eq!(sm.functions.len(), 2);
        assert_eq!(sm.functions[1].capture_count, 1);
    }

    #[test]
    fn discovery_assigns_funcid_in_dfs_preorder() {
        // outer { inner1 { innermost } inner2 } — IDs should be
        // outer=0, inner1=1, innermost=2, inner2=3.
        let innermost = fresh_lit(Some("innermost"), vec![], FuncBody::Stmts(vec![]));
        let inner1 = fresh_lit(
            Some("inner1"),
            vec![],
            FuncBody::Stmts(vec![Stmt::FuncDecl(innermost)]),
        );
        let inner2 = fresh_lit(Some("inner2"), vec![], FuncBody::Stmts(vec![]));
        let outer = tree::IrFunction {
            name: "outer".to_string(),
            params: vec![],
            body: vec![Stmt::FuncDecl(inner1), Stmt::FuncDecl(inner2)],
            source: None,
        };
        let tm = Module {
            items: vec![Item::Function(outer)],
        };
        let sm = lower_module(&tm);
        let names: Vec<&str> = sm.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["outer", "inner1", "innermost", "inner2"]);
    }
}
