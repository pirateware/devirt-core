//! Diagnostic: dump PT functions where MultipleDeclarators is the
//! sole blocker. Used to identify the shape(s) of multi-declarator
//! that the round-1 lowering missed.
//!
//! Run with:
//!   cargo run --release --example dump_sole_multidecl -- corpus/realworld

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

    let mut hits: Vec<(String, String, String)> = Vec::new(); // (file, name, source)
    let mut shape_counts: std::collections::BTreeMap<&'static str, usize> =
        Default::default();

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
            walk_items(
                &module.items,
                path.file_name().unwrap().to_string_lossy().as_ref(),
                "",
                &mut hits,
                &mut shape_counts,
            );
        }
    }

    println!(
        "Functions where MultipleDeclarators is the SOLE blocker: {}\n",
        hits.len()
    );
    println!("Shape tally (heuristic, by source-text pattern):");
    for (k, v) in &shape_counts {
        println!("  {:<35} {}", k, v);
    }
    println!();

    println!("Sample (first {}):", limit.min(hits.len()));
    for (file, name, body) in hits.iter().take(limit) {
        println!("\n--- {file} :: {name} ---");
        // Print first ~6 lines or 400 chars, whichever is shorter.
        let max_chars = 600usize;
        let snippet: String = body.chars().take(max_chars).collect();
        for line in snippet.lines().take(8) {
            println!("  {line}");
        }
        if body.len() > max_chars {
            println!("  ... [{} more bytes]", body.len() - max_chars);
        }
    }
}

fn walk_items(
    items: &[Item],
    file: &str,
    parent: &str,
    hits: &mut Vec<(String, String, String)>,
    shapes: &mut std::collections::BTreeMap<&'static str, usize>,
) {
    for item in items {
        match item {
            Item::Function(f) => {
                walk_stmts(&f.body, file, &f.name, hits, shapes);
            }
            Item::PassThroughFunction(lit) => {
                let qual = if parent.is_empty() {
                    lit.name.clone().unwrap_or_else(|| "<anon>".into())
                } else {
                    format!(
                        "{}/{}",
                        parent,
                        lit.name.as_deref().unwrap_or("<anon>")
                    )
                };
                if is_sole(&lit.blockers, BlockerKind::MultipleDeclarators) {
                    if let FuncBody::PassThrough(src) = &lit.body {
                        classify(src, shapes);
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
    shapes: &mut std::collections::BTreeMap<&'static str, usize>,
) {
    use ir::tree::Stmt;
    for s in stmts {
        match s {
            Stmt::FuncDecl(lit) => visit_lit(lit, file, parent, hits, shapes),
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                walk_stmts(then_body, file, parent, hits, shapes);
                if let Some(e) = else_body {
                    walk_stmts(e, file, parent, hits, shapes);
                }
            }
            Stmt::While { body, .. } => walk_stmts(body, file, parent, hits, shapes),
            Stmt::For { body, .. } => walk_stmts(body, file, parent, hits, shapes),
            Stmt::ForOf { body, .. } => walk_stmts(body, file, parent, hits, shapes),
            Stmt::ForIn { body, .. } => walk_stmts(body, file, parent, hits, shapes),
            Stmt::Block(b) => walk_stmts(b, file, parent, hits, shapes),
            Stmt::Try {
                body,
                catch,
                finally,
            } => {
                walk_stmts(body, file, parent, hits, shapes);
                if let Some((_, cb)) = catch {
                    walk_stmts(cb, file, parent, hits, shapes);
                }
                if let Some(fb) = finally {
                    walk_stmts(fb, file, parent, hits, shapes);
                }
            }
            Stmt::Let { init, .. } => {
                if let Some(e) = init {
                    walk_expr(e, file, parent, hits, shapes);
                }
            }
            Stmt::Const { init, .. } => walk_expr(init, file, parent, hits, shapes),
            Stmt::Assign { value, .. } => walk_expr(value, file, parent, hits, shapes),
            Stmt::ExprStmt(e) => walk_expr(e, file, parent, hits, shapes),
            Stmt::Return(Some(e)) => walk_expr(e, file, parent, hits, shapes),
            _ => {}
        }
    }
}

fn walk_expr(
    e: &ir::tree::Expr,
    file: &str,
    parent: &str,
    hits: &mut Vec<(String, String, String)>,
    shapes: &mut std::collections::BTreeMap<&'static str, usize>,
) {
    use ir::tree::Expr;
    match e {
        Expr::Func(lit) => visit_lit(lit, file, parent, hits, shapes),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, file, parent, hits, shapes);
            walk_expr(rhs, file, parent, hits, shapes);
        }
        Expr::Call { callee, args } | Expr::New { callee, args } => {
            walk_expr(callee, file, parent, hits, shapes);
            for a in args {
                walk_expr(a, file, parent, hits, shapes);
            }
        }
        Expr::Member { obj, .. } => walk_expr(obj, file, parent, hits, shapes),
        Expr::Index { obj, index } => {
            walk_expr(obj, file, parent, hits, shapes);
            walk_expr(index, file, parent, hits, shapes);
        }
        Expr::Unary { operand, .. } => walk_expr(operand, file, parent, hits, shapes),
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, file, parent, hits, shapes);
            walk_expr(then_branch, file, parent, hits, shapes);
            walk_expr(else_branch, file, parent, hits, shapes);
        }
        Expr::Array { elements } => {
            for el in elements {
                walk_expr(el, file, parent, hits, shapes);
            }
        }
        Expr::Object { props } => {
            for p in props {
                walk_expr(&p.value, file, parent, hits, shapes);
            }
        }
        Expr::Sequence { exprs } | Expr::Template { exprs, .. } => {
            for inner in exprs {
                walk_expr(inner, file, parent, hits, shapes);
            }
        }
        Expr::Assign { target, value } => {
            walk_expr(target, file, parent, hits, shapes);
            walk_expr(value, file, parent, hits, shapes);
        }
        Expr::CompoundAssign { target, value, .. } => {
            walk_expr(target, file, parent, hits, shapes);
            walk_expr(value, file, parent, hits, shapes);
        }
        Expr::Update { target, .. } => walk_expr(target, file, parent, hits, shapes),
        Expr::Spread(inner) => walk_expr(inner, file, parent, hits, shapes),
        _ => {}
    }
}

fn visit_lit(
    lit: &ir::tree::FuncLiteral,
    file: &str,
    parent: &str,
    hits: &mut Vec<(String, String, String)>,
    shapes: &mut std::collections::BTreeMap<&'static str, usize>,
) {
    let qual = format!(
        "{}/{}",
        parent,
        lit.name.as_deref().unwrap_or("<anon>")
    );
    if is_sole(&lit.blockers, BlockerKind::MultipleDeclarators) {
        if let FuncBody::PassThrough(src) = &lit.body {
            classify(src, shapes);
            hits.push((file.to_string(), qual.clone(), src.clone()));
        }
    }
    // Recurse into body if it's stmts (some literals are Stmts even when
    // containing inner sole-blocker literals).
    if let FuncBody::Stmts(body) = &lit.body {
        walk_stmts(body, file, &qual, hits, shapes);
    }
}

fn is_sole(blockers: &std::collections::BTreeSet<BlockerKind>, kind: BlockerKind) -> bool {
    blockers.len() == 1 && blockers.contains(&kind)
}

fn classify(
    src: &str,
    shapes: &mut std::collections::BTreeMap<&'static str, usize>,
) {
    // Heuristics on the source text. Not perfect, but enough to spot the
    // dominant shape.
    if regex_lite_for_init(src) {
        *shapes.entry("for_init_multi_declarator").or_default() += 1;
    } else if src.contains("switch") && src.contains("case") {
        *shapes.entry("switch_case_lexical_decl").or_default() += 1;
    } else if has_destructuring_decl(src) {
        *shapes.entry("destructuring_in_multi_declarator").or_default() += 1;
    } else if src.contains("class ") {
        *shapes.entry("class_body_present").or_default() += 1;
    } else {
        *shapes.entry("other").or_default() += 1;
    }
}

fn regex_lite_for_init(src: &str) -> bool {
    // Very rough: look for `for (var ...,` or `for (let ...,` or `for (const ...,`
    // before the first `;`.
    for kw in ["for (var ", "for (let ", "for (const ", "for(var ", "for(let ", "for(const "] {
        if let Some(idx) = src.find(kw) {
            let after = &src[idx..];
            if let Some(semi) = after.find(';') {
                if after[..semi].contains(',') {
                    return true;
                }
            }
        }
    }
    false
}

fn has_destructuring_decl(src: &str) -> bool {
    // Look for `var {`, `let {`, `const {`, `var [`, `let [`, `const [` followed
    // somewhere before the next `;` by a comma — heuristic, often wrong, but
    // enough to flag candidates.
    for kw in ["var {", "let {", "const {", "var [", "let [", "const ["] {
        if let Some(idx) = src.find(kw) {
            let after = &src[idx..];
            if let Some(semi) = after.find(';') {
                if after[..semi].matches(',').count() >= 1 {
                    return true;
                }
            }
        }
    }
    false
}
