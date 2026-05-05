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

use std::fmt::Write as _;

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
            });
            walk_expr(e, &qname, rows);
        }
        FuncBody::PassThrough(s) => {
            rows.push(FunctionCoverage {
                name: qname,
                disposition: Disposition::PassThrough,
                lowered_stmts: 0,
                passthrough_bytes: s.len(),
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

    let total_bytes_proxy = total_lowered_stmts as f64 * 30.0
        + total_pt_bytes as f64
        + cov.top_level_passthrough_bytes as f64;
    if total_bytes_proxy > 0.0 {
        let pt_share =
            (total_pt_bytes as f64 + cov.top_level_passthrough_bytes as f64) / total_bytes_proxy;
        writeln!(
            &mut out,
            "passthrough share (byte-weighted, ~30B/stmt proxy for lowered): {:.1}%",
            pt_share * 100.0
        )
        .unwrap();
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
