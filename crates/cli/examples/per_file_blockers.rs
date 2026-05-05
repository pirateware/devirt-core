//! Per-file blocker dump: for each PT function in the file, list its
//! blocker set. Used to confirm binding-blocker hypotheses before
//! committing to a lowering ticket.
//!
//! Run with:
//!   cargo run --release --example per_file_blockers -- <path-to-file.js>

use std::fs;
use std::path::PathBuf;

use ir::tree::{FuncBody, Item};

fn main() {
    let path: PathBuf = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: per_file_blockers <path>");

    let src = fs::read_to_string(&path).expect("read");
    let module = parse::parse_and_lower(&src).expect("parse");

    let mut tally: std::collections::BTreeMap<String, usize> =
        Default::default();
    let mut sole_tally: std::collections::BTreeMap<String, usize> =
        Default::default();
    let mut total_pt: usize = 0;
    let mut total_pt_bytes: usize = 0;

    walk(&module.items, &mut |lit| {
        if !lit.blockers.is_empty() {
            total_pt += 1;
            if let FuncBody::PassThrough(s) = &lit.body {
                total_pt_bytes += s.len();
            }
            for b in &lit.blockers {
                *tally.entry(format!("{:?}", b)).or_default() += 1;
            }
            if lit.blockers.len() == 1 {
                let only = lit.blockers.iter().next().unwrap();
                *sole_tally.entry(format!("{:?}", only)).or_default() += 1;
            }
        }
    });

    println!("File: {}", path.display());
    println!("PT functions: {}  (bytes: {})", total_pt, total_pt_bytes);
    println!("\noccurrences (PT functions containing blocker):");
    for (k, v) in &tally {
        println!("  {:<30} {}", k, v);
    }
    println!("\nsole-blocker counterfactual (functions where this is the only blocker):");
    for (k, v) in &sole_tally {
        println!("  {:<30} {}", k, v);
    }
}

fn walk<'a>(items: &'a [Item], cb: &mut dyn FnMut(&'a ir::tree::FuncLiteral)) {
    for item in items {
        match item {
            Item::Function(f) => walk_stmts(&f.body, cb),
            Item::PassThroughFunction(lit) => {
                cb(lit);
                if let FuncBody::Stmts(b) = &lit.body {
                    walk_stmts(b, cb);
                }
            }
            _ => {}
        }
    }
}

fn walk_stmts<'a>(stmts: &'a [ir::tree::Stmt], cb: &mut dyn FnMut(&'a ir::tree::FuncLiteral)) {
    use ir::tree::Stmt;
    for s in stmts {
        match s {
            Stmt::FuncDecl(lit) => {
                cb(lit);
                if let FuncBody::Stmts(b) = &lit.body {
                    walk_stmts(b, cb);
                }
            }
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                walk_stmts(then_body, cb);
                if let Some(e) = else_body {
                    walk_stmts(e, cb);
                }
            }
            Stmt::While { body, .. }
            | Stmt::For { body, .. }
            | Stmt::ForOf { body, .. }
            | Stmt::ForIn { body, .. } => walk_stmts(body, cb),
            Stmt::Block(b) => walk_stmts(b, cb),
            Stmt::Try {
                body,
                catch,
                finally,
            } => {
                walk_stmts(body, cb);
                if let Some((_, cb_body)) = catch {
                    walk_stmts(cb_body, cb);
                }
                if let Some(fb) = finally {
                    walk_stmts(fb, cb);
                }
            }
            Stmt::Let { init: Some(e), .. } | Stmt::Const { init: e, .. } => {
                walk_expr(e, cb);
            }
            Stmt::Assign { value, .. } => walk_expr(value, cb),
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) => walk_expr(e, cb),
            _ => {}
        }
    }
}

fn walk_expr<'a>(e: &'a ir::tree::Expr, cb: &mut dyn FnMut(&'a ir::tree::FuncLiteral)) {
    use ir::tree::Expr;
    match e {
        Expr::Func(lit) => {
            cb(lit);
            if let FuncBody::Stmts(b) = &lit.body {
                walk_stmts(b, cb);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(lhs, cb);
            walk_expr(rhs, cb);
        }
        Expr::Call { callee, args } | Expr::New { callee, args } => {
            walk_expr(callee, cb);
            for a in args {
                walk_expr(a, cb);
            }
        }
        Expr::Member { obj, .. } => walk_expr(obj, cb),
        Expr::Index { obj, index } => {
            walk_expr(obj, cb);
            walk_expr(index, cb);
        }
        Expr::Unary { operand, .. } => walk_expr(operand, cb),
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => {
            walk_expr(cond, cb);
            walk_expr(then_branch, cb);
            walk_expr(else_branch, cb);
        }
        Expr::Array { elements } => {
            for el in elements {
                walk_expr(el, cb);
            }
        }
        Expr::Object { props } => {
            for p in props {
                walk_expr(&p.value, cb);
            }
        }
        Expr::Sequence { exprs } | Expr::Template { exprs, .. } => {
            for inner in exprs {
                walk_expr(inner, cb);
            }
        }
        Expr::Assign { target, value } => {
            walk_expr(target, cb);
            walk_expr(value, cb);
        }
        Expr::CompoundAssign { target, value, .. } => {
            walk_expr(target, cb);
            walk_expr(value, cb);
        }
        Expr::Update { target, .. } => walk_expr(target, cb),
        Expr::Spread(inner) => walk_expr(inner, cb),
        _ => {}
    }
}
