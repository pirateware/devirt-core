//! Diagnostic: dump PT functions where UnsupportedBinOp is the sole
//! blocker. Identifies which specific binary operator(s) the
//! disposition walker rejects — should be a small list (likely just
//! `in` and `instanceof`, which aren't in `binop_lowerable`).
//!
//! Run with:
//!   cargo run --release --example dump_sole_unsupported_binop -- corpus/realworld

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use ir::blocker::BlockerKind;
use ir::tree::{FuncBody, Item};

fn main() {
    let corpus_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("corpus/realworld"));

    let limit: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);

    let mut hits: Vec<(String, String, String)> = Vec::new();
    let mut op_counts: BTreeMap<String, usize> = BTreeMap::new();

    for tier in ["tier1", "tier2", "tier3"] {
        let tier_dir = corpus_dir.join(tier);
        if !tier_dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&tier_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().map(|e| e != "js").unwrap_or(true) {
                continue;
            }
            let src = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let module = match parse::parse_and_lower(&src) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let file_name = path.file_name().unwrap().to_string_lossy().to_string();
            walk_items(&module.items, &file_name, &mut hits, &mut op_counts);
        }
    }

    println!(
        "Functions where UnsupportedBinOp is the SOLE blocker: {}\n",
        hits.len()
    );
    println!(
        "Operator tally (which operator(s) the walker rejected — counted by appearance in body):"
    );
    for (op, count) in &op_counts {
        println!("  {:<25} {}", op, count);
    }
    println!();

    println!("Sample (first {}):", limit.min(hits.len()));
    for (file, name, body) in hits.iter().take(limit) {
        println!("\n--- {file} :: {name} ---");
        let snippet: String = body.chars().take(400).collect();
        for line in snippet.lines().take(6) {
            println!("  {line}");
        }
        if body.len() > 400 {
            println!("  ... [{} more bytes]", body.len() - 400);
        }
    }
}

fn walk_items(
    items: &[Item],
    file: &str,
    hits: &mut Vec<(String, String, String)>,
    op_counts: &mut BTreeMap<String, usize>,
) {
    for item in items {
        match item {
            Item::Function(f) => {
                walk_stmts(&f.body, file, &f.name, hits, op_counts);
            }
            Item::PassThroughFunction(lit) => {
                let qual = lit.name.clone().unwrap_or_else(|| "<anon>".into());
                if is_sole(&lit.blockers, BlockerKind::UnsupportedBinOp) {
                    if let FuncBody::PassThrough(src) = &lit.body {
                        count_unsupported_ops(src, op_counts);
                        hits.push((file.to_string(), qual, src.clone()));
                    }
                }
            }
            _ => {}
        }
    }
}

fn walk_stmts(
    stmts: &[ir::tree::Stmt],
    file: &str,
    parent: &str,
    hits: &mut Vec<(String, String, String)>,
    op_counts: &mut BTreeMap<String, usize>,
) {
    use ir::tree::Stmt;
    for s in stmts {
        match s {
            Stmt::FuncDecl(lit) => visit_lit(lit, file, parent, hits, op_counts),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                walk_stmts(then_body, file, parent, hits, op_counts);
                if let Some(e) = else_body {
                    walk_stmts(e, file, parent, hits, op_counts);
                }
            }
            Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::ForOf { body, .. }
            | Stmt::ForIn { body, .. } => walk_stmts(body, file, parent, hits, op_counts),
            Stmt::Block(b) => walk_stmts(b, file, parent, hits, op_counts),
            Stmt::Try {
                body,
                catch,
                finally,
            } => {
                walk_stmts(body, file, parent, hits, op_counts);
                if let Some((_, cb)) = catch {
                    walk_stmts(cb, file, parent, hits, op_counts);
                }
                if let Some(fb) = finally {
                    walk_stmts(fb, file, parent, hits, op_counts);
                }
            }
            Stmt::Let { init: Some(e), .. } | Stmt::Const { init: e, .. } => {
                walk_expr(e, file, parent, hits, op_counts);
            }
            Stmt::Assign { value, .. } => walk_expr(value, file, parent, hits, op_counts),
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => {
                walk_expr(e, file, parent, hits, op_counts)
            }
            _ => {}
        }
    }
}

fn walk_expr(
    e: &ir::tree::Expr,
    file: &str,
    parent: &str,
    hits: &mut Vec<(String, String, String)>,
    op_counts: &mut BTreeMap<String, usize>,
) {
    use ir::tree::Expr;
    match e {
        Expr::Func(lit) => visit_lit(lit, file, parent, hits, op_counts),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, file, parent, hits, op_counts);
            walk_expr(rhs, file, parent, hits, op_counts);
        }
        Expr::Call { callee, args } | Expr::New { callee, args } => {
            walk_expr(callee, file, parent, hits, op_counts);
            for a in args {
                walk_expr(a, file, parent, hits, op_counts);
            }
        }
        Expr::Member { obj, .. } => walk_expr(obj, file, parent, hits, op_counts),
        Expr::Index { obj, index } => {
            walk_expr(obj, file, parent, hits, op_counts);
            walk_expr(index, file, parent, hits, op_counts);
        }
        Expr::Unary { operand, .. } => walk_expr(operand, file, parent, hits, op_counts),
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, file, parent, hits, op_counts);
            walk_expr(then_branch, file, parent, hits, op_counts);
            walk_expr(else_branch, file, parent, hits, op_counts);
        }
        Expr::Array { elements } => {
            for el in elements {
                walk_expr(el, file, parent, hits, op_counts);
            }
        }
        Expr::Object { props } => {
            for p in props {
                walk_expr(&p.value, file, parent, hits, op_counts);
            }
        }
        Expr::Sequence { exprs } | Expr::Template { exprs, .. } => {
            for inner in exprs {
                walk_expr(inner, file, parent, hits, op_counts);
            }
        }
        Expr::Assign { target, value } => {
            walk_expr(target, file, parent, hits, op_counts);
            walk_expr(value, file, parent, hits, op_counts);
        }
        Expr::CompoundAssign { target, value, .. } => {
            walk_expr(target, file, parent, hits, op_counts);
            walk_expr(value, file, parent, hits, op_counts);
        }
        Expr::Update { target, .. } => walk_expr(target, file, parent, hits, op_counts),
        Expr::Spread(inner) => walk_expr(inner, file, parent, hits, op_counts),
        _ => {}
    }
}

fn visit_lit(
    lit: &ir::tree::FuncLiteral,
    file: &str,
    parent: &str,
    hits: &mut Vec<(String, String, String)>,
    op_counts: &mut BTreeMap<String, usize>,
) {
    let qual = format!("{}/{}", parent, lit.name.as_deref().unwrap_or("<anon>"));
    if is_sole(&lit.blockers, BlockerKind::UnsupportedBinOp) {
        if let FuncBody::PassThrough(src) = &lit.body {
            count_unsupported_ops(src, op_counts);
            hits.push((file.to_string(), qual.clone(), src.clone()));
        }
    }
    if let FuncBody::Stmts(body) = &lit.body {
        walk_stmts(body, file, &qual, hits, op_counts);
    }
}

fn is_sole(blockers: &std::collections::BTreeSet<BlockerKind>, kind: BlockerKind) -> bool {
    blockers.len() == 1 && blockers.contains(&kind)
}

/// Heuristic counter for unlowered binops in a function body source
/// slice. The disposition walker rejects binops not in
/// `binop_lowerable` (today: `in`, `instanceof`). We grep for those
/// tokens directly — there's no other surface in JS where these
/// keywords appear as bare tokens. False positives are vanishingly
/// rare (the only adjacent shape is `for (... in ...)`, which is a
/// statement, not a binary operator — and would not produce
/// `UnsupportedBinOp` anyway).
fn count_unsupported_ops(body_src: &str, out: &mut BTreeMap<String, usize>) {
    // `instanceof` token: word boundary on both sides.
    let n_instanceof = count_keyword(body_src, "instanceof");
    if n_instanceof > 0 {
        *out.entry("instanceof".into()).or_default() += n_instanceof;
    }
    // `in` token: tricky because `for (k in o)` also matches. Subtract
    // those by counting `for (.. in)`. Crude but signal-preserving.
    let n_in = count_keyword(body_src, "in");
    let n_for_in = count_for_in(body_src);
    let n_in_binop = n_in.saturating_sub(n_for_in);
    if n_in_binop > 0 {
        *out.entry("in (binop)".into()).or_default() += n_in_binop;
    }
}

fn count_keyword(s: &str, kw: &str) -> usize {
    let mut count = 0;
    let bytes = s.as_bytes();
    let kw_bytes = kw.as_bytes();
    let mut i = 0;
    while i + kw_bytes.len() <= bytes.len() {
        if &bytes[i..i + kw_bytes.len()] == kw_bytes
            && (i == 0 || !is_word_char(bytes[i - 1]))
            && (i + kw_bytes.len() == bytes.len() || !is_word_char(bytes[i + kw_bytes.len()]))
        {
            count += 1;
            i += kw_bytes.len();
        } else {
            i += 1;
        }
    }
    count
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

fn count_for_in(s: &str) -> usize {
    // Match `for (... in` — heuristic. Simple substring approximation:
    // every occurrence of `for (` followed by `in` before the matching
    // `)`. We just count `for (` occurrences and assume each is a
    // `for(...in)` if `in` appears in the next ~80 chars before `)`.
    let mut count = 0;
    let mut i = 0;
    let bytes = s.as_bytes();
    while let Some(p) = s[i..].find("for") {
        let abs = i + p;
        // word-boundary before
        if abs > 0 && is_word_char(bytes[abs - 1]) {
            i = abs + 3;
            continue;
        }
        // followed by optional whitespace then `(`
        let after = &s[abs + 3..];
        let trimmed = after.trim_start();
        if !trimmed.starts_with('(') {
            i = abs + 3;
            continue;
        }
        // search the next ~120 chars for ` in ` before `)`
        let head: String = trimmed.chars().take(120).collect();
        if let Some(close) = head.find(')')
            && let Some(in_pos) = head[..close].find(" in ")
            && in_pos > 0
        {
            count += 1;
        }
        i = abs + 3;
    }
    count
}
