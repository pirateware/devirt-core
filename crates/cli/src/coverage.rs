//! Diagnostic coverage stats for the lowering subset.
//!
//! Walks a tree-IR module and reports, per function, whether its body
//! went through the Lowered SSA path or PassThrough source-slice path,
//! plus a size proxy (lowered: tree-IR Stmt count not crossing
//! nested-function boundaries; passthrough: source-slice byte count).
//!
//! The per-function row plus module totals answer the strategic
//! question: "how much of typical input does the lowered path actually
//! see?" Used by `cargo run -- --coverage-stats <file.js>` from
//! `main.rs`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use ::ir::blocker::BlockerKind;

use crate::ir::tree::{self, Expr, FuncBody, FuncLiteral, Item, Stmt};

#[derive(Debug, Default)]
pub struct ModuleCoverage {
    pub functions: Vec<FunctionCoverage>,
    pub top_level_passthrough_stmts: usize,
    pub top_level_passthrough_bytes: usize,
}

#[derive(Debug)]
pub struct FunctionCoverage {
    /// Path-qualified name: `"outer/inner"` for a nested literal,
    /// `"<anon>"` for an anonymous expression. Top-level functions
    /// have an unqualified name.
    pub name: String,
    pub disposition: Disposition,
    /// Lowered: count of `Stmt` nodes in this function's *own* body
    /// (not recursing into nested function literals). PassThrough: 0.
    pub lowered_stmts: usize,
    /// PassThrough: byte count of the source slice. Lowered: 0.
    pub passthrough_bytes: usize,
    /// PassThrough: set of all constructs that blocked lowering. Lowered: empty.
    pub blockers: BTreeSet<BlockerKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Lowered,
    PassThrough,
}

impl Disposition {
    fn label(&self) -> &'static str {
        match self {
            Disposition::Lowered => "lowered",
            Disposition::PassThrough => "passthrough",
        }
    }
}

pub fn analyze(tm: &tree::Module) -> ModuleCoverage {
    let mut out = ModuleCoverage::default();
    for item in &tm.items {
        match item {
            Item::Function(f) => {
                out.functions.push(FunctionCoverage {
                    name: f.name.clone(),
                    disposition: Disposition::Lowered,
                    lowered_stmts: count_stmts(&f.body),
                    passthrough_bytes: 0,
                    blockers: BTreeSet::new(),
                });
                walk_stmts(&f.body, &f.name, &mut out.functions);
            }
            Item::PassThroughFunction(lit) => {
                let name = lit.name.clone().unwrap_or_else(|| "<anon>".into());
                let bytes = match &lit.body {
                    FuncBody::PassThrough(s) => s.len(),
                    _ => 0,
                };
                out.functions.push(FunctionCoverage {
                    name,
                    disposition: Disposition::PassThrough,
                    lowered_stmts: 0,
                    passthrough_bytes: bytes,
                    blockers: lit.blockers.clone(),
                });
            }
            Item::Stmt(_) => {
                // Hand-built tests only; OXC frontend never emits this
                // for non-function top-level. Ignore.
            }
            Item::PassThroughStmt(src) => {
                out.top_level_passthrough_stmts += 1;
                out.top_level_passthrough_bytes += src.len();
            }
        }
    }
    out
}

/// Count `Stmt` nodes in a body, recursing into control-flow blocks
/// (if/while/Block) but stopping at nested function literals — those
/// have their own coverage row.
fn count_stmts(stmts: &[Stmt]) -> usize {
    let mut n = 0;
    for s in stmts {
        n += 1;
        match s {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                n += count_stmts(then_body);
                if let Some(eb) = else_body {
                    n += count_stmts(eb);
                }
            }
            Stmt::While { body, .. } => n += count_stmts(body),
            Stmt::For { init, body, .. } => {
                if init.is_some() {
                    n += 1;
                }
                n += count_stmts(body);
            }
            Stmt::Continue => {}
            Stmt::Block(b) => n += count_stmts(b),
            // Stmt::FuncDecl / nested Expr::Func bodies are separate
            // coverage rows, not part of this function's own stmt count.
            _ => {}
        }
    }
    n
}

/// Walk a Lowered function body to discover nested function literals
/// (Stmt::FuncDecl and Expr::Func) and emit one coverage row per
/// nested literal. `parent` is the qualified-name prefix.
fn walk_stmts(stmts: &[Stmt], parent: &str, rows: &mut Vec<FunctionCoverage>) {
    for s in stmts {
        match s {
            Stmt::FuncDecl(lit) => emit_lit_row(lit, parent, rows),
            Stmt::If {
                then_body,
                else_body,
                cond,
            } => {
                walk_expr(cond, parent, rows);
                walk_stmts(then_body, parent, rows);
                if let Some(eb) = else_body {
                    walk_stmts(eb, parent, rows);
                }
            }
            Stmt::While { body, cond } => {
                walk_expr(cond, parent, rows);
                walk_stmts(body, parent, rows);
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                if let Some(init) = init {
                    walk_stmts(std::slice::from_ref(init.as_ref()), parent, rows);
                }
                if let Some(c) = cond {
                    walk_expr(c, parent, rows);
                }
                walk_stmts(body, parent, rows);
                if let Some(s) = step {
                    walk_expr(s, parent, rows);
                }
            }
            Stmt::ForOf { iterable, body, .. } => {
                walk_expr(iterable, parent, rows);
                walk_stmts(body, parent, rows);
            }
            Stmt::ForIn { obj, body, .. } => {
                walk_expr(obj, parent, rows);
                walk_stmts(body, parent, rows);
            }
            Stmt::Continue => {}
            Stmt::Block(b) => walk_stmts(b, parent, rows),
            Stmt::Let { init: Some(e), .. } | Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => {
                walk_expr(e, parent, rows);
            }
            Stmt::Const { init, .. } => walk_expr(init, parent, rows),
            Stmt::Assign { target, value } => {
                walk_expr(target, parent, rows);
                walk_expr(value, parent, rows);
            }
            Stmt::Let { init: None, .. } | Stmt::Return(None) => {}
            Stmt::Throw(e) => walk_expr(e, parent, rows),
            Stmt::Try { body, catch, finally } => {
                walk_stmts(body, parent, rows);
                if let Some((_, catch_body)) = catch {
                    walk_stmts(catch_body, parent, rows);
                }
                if let Some(finally_body) = finally {
                    walk_stmts(finally_body, parent, rows);
                }
            }
        }
    }
}

fn walk_expr(e: &Expr, parent: &str, rows: &mut Vec<FunctionCoverage>) {
    match e {
        Expr::Func(lit) => emit_lit_row(lit, parent, rows),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, parent, rows);
            walk_expr(rhs, parent, rows);
        }
        Expr::Unary { operand, .. } => walk_expr(operand, parent, rows),
        Expr::Call { callee, args } => {
            walk_expr(callee, parent, rows);
            for a in args {
                walk_expr(a, parent, rows);
            }
        }
        Expr::Member { obj, .. } => walk_expr(obj, parent, rows),
        Expr::Index { obj, index } => {
            walk_expr(obj, parent, rows);
            walk_expr(index, parent, rows);
        }
        Expr::Array { elements } => {
            for el in elements {
                walk_expr(el, parent, rows);
            }
        }
        Expr::Object { props } => {
            for p in props {
                if let crate::ir::tree::ObjectKey::Computed(e) = &p.key {
                    walk_expr(e, parent, rows);
                }
                walk_expr(&p.value, parent, rows);
            }
        }
        Expr::New { callee, args } => {
            walk_expr(callee, parent, rows);
            for a in args {
                walk_expr(a, parent, rows);
            }
        }
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, parent, rows);
            walk_expr(then_branch, parent, rows);
            walk_expr(else_branch, parent, rows);
        }
        Expr::Update { target, .. } => walk_expr(target, parent, rows),
        _ => {}
    }
}

fn emit_lit_row(lit: &FuncLiteral, parent: &str, rows: &mut Vec<FunctionCoverage>) {
    let leaf = lit.name.clone().unwrap_or_else(|| "<anon>".into());
    let qname = if parent.is_empty() {
        leaf.clone()
    } else {
        format!("{parent}/{leaf}")
    };
    match &lit.body {
        FuncBody::Stmts(b) => {
            rows.push(FunctionCoverage {
                name: qname.clone(),
                disposition: Disposition::Lowered,
                lowered_stmts: count_stmts(b),
                passthrough_bytes: 0,
                blockers: BTreeSet::new(),
            });
            walk_stmts(b, &qname, rows);
        }
        FuncBody::Expression(e) => {
            // Expression-body arrow has exactly one statement-equivalent
            // (the expression). Count as 1 lowered "stmt" and recurse
            // for nested literals inside the expression.
            rows.push(FunctionCoverage {
                name: qname.clone(),
                disposition: Disposition::Lowered,
                lowered_stmts: 1,
                passthrough_bytes: 0,
                blockers: BTreeSet::new(),
            });
            walk_expr(e, &qname, rows);
        }
        FuncBody::PassThrough(s) => {
            rows.push(FunctionCoverage {
                name: qname,
                disposition: Disposition::PassThrough,
                lowered_stmts: 0,
                passthrough_bytes: s.len(),
                blockers: lit.blockers.clone(),
            });
        }
    }
}

pub fn format_table(cov: &ModuleCoverage) -> String {
    let mut out = String::new();
    let mut total_lowered_stmts = 0usize;
    let mut total_pt_bytes = 0usize;
    let mut lowered_fns = 0usize;
    let mut pt_fns = 0usize;

    writeln!(
        &mut out,
        "{:<40} {:<12} {:>12} {:>14}",
        "function", "disposition", "lowered_stmts", "passthrough_B"
    )
    .unwrap();
    writeln!(&mut out, "{}", "-".repeat(82)).unwrap();
    for row in &cov.functions {
        writeln!(
            &mut out,
            "{:<40} {:<12} {:>12} {:>14}",
            truncate(&row.name, 40),
            row.disposition.label(),
            row.lowered_stmts,
            row.passthrough_bytes
        )
        .unwrap();
        total_lowered_stmts += row.lowered_stmts;
        total_pt_bytes += row.passthrough_bytes;
        match row.disposition {
            Disposition::Lowered => lowered_fns += 1,
            Disposition::PassThrough => pt_fns += 1,
        }
    }
    writeln!(&mut out, "{}", "-".repeat(82)).unwrap();
    writeln!(
        &mut out,
        "totals: {lowered_fns} lowered fn(s), {pt_fns} passthrough fn(s); \
         lowered_stmts={total_lowered_stmts}; passthrough_B={total_pt_bytes} \
         (functions) + {pt_top_b} (top-level, {pt_top_n} stmt(s))",
        pt_top_b = cov.top_level_passthrough_bytes,
        pt_top_n = cov.top_level_passthrough_stmts,
    )
    .unwrap();

    // Refined metric (Round 3): split function-body passthrough from
    // top-level passthrough. Function-body passthrough = bytes from
    // `Item::PassThroughFunction` and from PT inner literals only;
    // top-level passthrough = bytes from `Item::PassThroughStmt`.
    // Total passthrough share (numerator = sum of both PT byte
    // counts) is reported for compatibility with prior rounds. The
    // decision rule in docs/COVERAGE_AUDIT.md uses the function-body
    // figure exclusively.
    let total_bytes_proxy = total_lowered_stmts as f64 * 30.0
        + total_pt_bytes as f64
        + cov.top_level_passthrough_bytes as f64;
    if total_bytes_proxy > 0.0 {
        let pt_share =
            (total_pt_bytes as f64 + cov.top_level_passthrough_bytes as f64) / total_bytes_proxy;
        // Function-body share: relative only to function-side bytes
        // (lowered + function PT). Excludes top-level entirely so a
        // program dominated by a top-level driver line doesn't drag
        // the score down.
        let fn_denom = total_lowered_stmts as f64 * 30.0 + total_pt_bytes as f64;
        let fn_pt_share = if fn_denom > 0.0 {
            total_pt_bytes as f64 / fn_denom
        } else {
            0.0
        };
        // Top-level share: relative to top-level bytes only
        // (informational — driver lines are usually pre-existing
        // ergonomics, not target-of-optimization).
        let tl_denom = cov.top_level_passthrough_bytes as f64
            + total_lowered_stmts as f64 * 30.0
            + total_pt_bytes as f64;
        let tl_pt_share = if tl_denom > 0.0 {
            cov.top_level_passthrough_bytes as f64 / tl_denom
        } else {
            0.0
        };
        writeln!(
            &mut out,
            "passthrough share (byte-weighted, ~30B/stmt proxy for lowered): {:.1}%",
            pt_share * 100.0
        )
        .unwrap();
        writeln!(
            &mut out,
            "  function_body_passthrough_pct: {:.1}%  (decision rule input)",
            fn_pt_share * 100.0
        )
        .unwrap();
        writeln!(
            &mut out,
            "  top_level_passthrough_pct:     {:.1}%  (informational)",
            tl_pt_share * 100.0
        )
        .unwrap();
    }
    out
}

/// Format two blocker-analysis views for PassThrough functions:
///
/// 1. **Occurrence count** — how many PT functions contain each `BlockerKind`
/// 2. **Counterfactual unlock** — how many PT functions have this as their *sole* blocker
///
/// A previous version of this table also reported a greedy cumulative
/// unlock projection: "if we fix kinds in counterfactual-descending order,
/// how many PT functions become fully unblocked at each step?" That
/// projection was removed because it was systematically optimistic for
/// any non-trivial round of lowering work. The formula itself was sound,
/// but its input — the per-function blocker set as the disposition walker
/// sees it *today* — is not stable across lowerings, so projecting future
/// outcomes against today's blocker sets miscounts.
///
/// Two structural reasons:
///
/// 1. **Walker recursion changes when a construct is lowered.** Most
///    "node-with-children" blockers (AssignmentExpr, SequenceExpr, etc.)
///    are emitted at a node where the walker stops descending. Lowering
///    the kind requires the walker to start recursing into the node's
///    children, which surfaces blockers that were previously invisible.
///    Functions the projection counted as "single-blocker, will unlock"
///    can be revealed to have additional nested blockers post-lowering,
///    so they stay PT.
/// 2. **Coarse blocker buckets get split.** A blocker like the old
///    `UnsupportedExpr` covered several distinct constructs (plain `=`,
///    `+=`, `&&=`, ...). The projection treated the bucket atomically;
///    lowering one of its constituent constructs only partially drains
///    it. Round 3 (SequenceExpr + AssignmentExpr + ForInStmt, PR #2)
///    demonstrated this: 228 functions projected to unlock, ~117 actually
///    moved out of PT. Most of the gap was the 176 occurrences of what
///    is now `CompoundAssignmentExpr`, previously hidden inside
///    `UnsupportedExpr`.
///
/// Round 1's `MultipleDeclarators` projection held empirically because
/// MultipleDeclarators is a *property* of a declaration, not a node with
/// children — the walker's traversal didn't change when it was lowered,
/// so the pre-projection blocker sets matched the post-outcome blocker
/// sets.
///
/// Counterfactual is preserved because a function blocked by exactly one
/// kind *today* is robust to future walker refinements on other kinds:
/// either we lower its sole blocker (it unlocks) or we don't (it stays
/// PT). It can over-predict if walker refinement later splits its sole
/// blocker, but the failure mode is bounded by that single kind, not by
/// a multi-step greedy chain. For multi-step planning, prefer per-PR
/// validation against actual outcomes over projection-from-a-snapshot.
///
/// See `docs/COVERAGE_AUDIT.md` for the full investigation.
pub fn format_blocker_table(cov: &ModuleCoverage) -> String {
    let pt_fns: Vec<&FunctionCoverage> = cov
        .functions
        .iter()
        .filter(|f| f.disposition == Disposition::PassThrough)
        .collect();

    if pt_fns.is_empty() {
        return String::from("(no passthrough functions — blocker table empty)\n");
    }

    // Collect every BlockerKind seen across all PT functions.
    let all_kinds: BTreeSet<BlockerKind> = pt_fns
        .iter()
        .flat_map(|f| f.blockers.iter().cloned())
        .collect();

    // Per-kind occurrence and counterfactual counts.
    let occurrences: BTreeMap<BlockerKind, usize> = all_kinds
        .iter()
        .map(|k| (k.clone(), pt_fns.iter().filter(|f| f.blockers.contains(k)).count()))
        .collect();
    let counterfactuals: BTreeMap<BlockerKind, usize> = all_kinds
        .iter()
        .map(|k| {
            (
                k.clone(),
                pt_fns
                    .iter()
                    .filter(|f| f.blockers.len() == 1 && f.blockers.contains(k))
                    .count(),
            )
        })
        .collect();

    // Sort by counterfactual count descending, break ties by kind name.
    let mut ranked: Vec<BlockerKind> = all_kinds.into_iter().collect();
    ranked.sort_by(|a, b| {
        counterfactuals[b]
            .cmp(&counterfactuals[a])
            .then_with(|| a.to_string().cmp(&b.to_string()))
    });
    let total_pt = pt_fns.len();

    let mut out = String::new();
    writeln!(
        &mut out,
        "\n## Blocker Analysis ({total_pt} passthrough function(s))\n"
    )
    .unwrap();
    writeln!(
        &mut out,
        "{:<34} {:>11} {:>14}",
        "blocker_kind", "occurrences", "counterfactual"
    )
    .unwrap();
    writeln!(&mut out, "{}", "-".repeat(63)).unwrap();
    for k in &ranked {
        writeln!(
            &mut out,
            "{:<34} {:>11} {:>14}",
            k.to_string(),
            occurrences[k],
            counterfactuals[k],
        )
        .unwrap();
    }
    writeln!(&mut out, "{}", "-".repeat(63)).unwrap();
    writeln!(
        &mut out,
        "occurrences: PT functions containing this kind. \
         counterfactual: PT functions where this is the sole blocker. \
         Cumulative greedy projection removed — see format_blocker_table doc \
         and docs/COVERAGE_AUDIT.md."
    )
    .unwrap();
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
