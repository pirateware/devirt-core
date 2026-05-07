//! Per-blocker corpus frequency dump (issue #43).
//!
//! Walks every `.js` sample under one or more roots, parses each file
//! through `parse::parse_and_lower`, and aggregates `BlockerKind`
//! frequencies across every PassThrough function reached. The walk
//! reuses each `FuncLiteral.blockers` field — the set the disposition
//! decider stamped on the function during lowering — so the report is
//! exactly what production lowering decided.
//!
//! Output:
//!
//!   BlockerKind                  | functions hit | total occurrences
//!   DefaultParam                 | 142           | 142
//!   ...
//!
//! "functions hit" = number of distinct PassThrough functions where
//! this BlockerKind appears at least once.
//!
//! "total occurrences" = sum of BlockerKind hits across all functions.
//! Because `disposition::decide` records blockers in a `BTreeSet` (one
//! entry per kind per function regardless of how many times the
//! construct fires inside that function), this column is numerically
//! equal to "functions hit". It is reported as a separate column to
//! match the table shape requested by issue #43, and so a future
//! per-occurrence walker (counted multiset) can be slotted in without
//! a column shuffle.
//!
//! Run with:
//!   cargo run --release --example blocker_frequency
//!   cargo run --release --example blocker_frequency -- <root> [<root>...]
//!
//! Default roots when no args are passed:
//!   crates/cli/coverage_corpus
//!   corpus/realworld
//!   corpus/sources

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use ir::blocker::BlockerKind;
use ir::tree::{FuncBody, FuncLiteral, Item};

fn main() {
    let roots: Vec<PathBuf> = {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.is_empty() {
            default_roots()
        } else {
            args.into_iter().map(PathBuf::from).collect()
        }
    };

    let report = aggregate_roots(&roots);
    print_report(&report);
}

/// Default corpus roots, resolved relative to the workspace root. The
/// example may be invoked from either the workspace root (`cargo run
/// --example`) or from the crate dir; we try both prefixes.
fn default_roots() -> Vec<PathBuf> {
    let candidates = [
        "crates/cli/coverage_corpus",
        "corpus/realworld",
        "corpus/sources",
        // When invoked from inside crates/cli (e.g. via `cargo run -p cli`).
        "coverage_corpus",
        "../../corpus/realworld",
        "../../corpus/sources",
    ];
    candidates
        .iter()
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

/// Aggregated frequency report.
#[derive(Default, Debug)]
pub struct Report {
    /// `BlockerKind` -> (function-hits, total-occurrences). With the
    /// set-based decider these two are equal; see module-level comment.
    pub by_kind: BTreeMap<BlockerKind, (usize, usize)>,
    pub samples_walked: usize,
    pub samples_failed: usize,
    pub functions_seen: usize,
    pub passthrough_functions: usize,
}

/// Walk the given roots, collect blocker frequencies. Public so the
/// in-crate test (and any future caller) can drive aggregation against
/// a fixture without re-implementing the walk.
pub fn aggregate_roots(roots: &[PathBuf]) -> Report {
    let mut report = Report::default();
    for root in roots {
        for path in collect_js_files(root) {
            let src = match fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => {
                    report.samples_failed += 1;
                    continue;
                }
            };
            let module = match parse::parse_and_lower(&src) {
                Ok(m) => m,
                Err(_) => {
                    report.samples_failed += 1;
                    continue;
                }
            };
            report.samples_walked += 1;
            walk_items(&module.items, &mut |lit| {
                report.functions_seen += 1;
                if !lit.blockers.is_empty() {
                    report.passthrough_functions += 1;
                    for b in &lit.blockers {
                        let entry = report.by_kind.entry(b.clone()).or_insert((0, 0));
                        entry.0 += 1;
                        entry.1 += 1;
                    }
                }
            });
        }
    }
    report
}

fn collect_js_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        if root.extension().is_some_and(|e| e == "js") {
            out.push(root.to_path_buf());
        }
        return out;
    }
    let entries = match fs::read_dir(root) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_js_files(&path));
        } else if path.extension().is_some_and(|e| e == "js") {
            out.push(path);
        }
    }
    out
}

fn print_report(r: &Report) {
    println!("Corpus blocker-frequency report");
    println!("===============================");
    println!("samples walked    : {}", r.samples_walked);
    if r.samples_failed > 0 {
        println!("samples failed    : {}", r.samples_failed);
    }
    println!("functions seen    : {}", r.functions_seen);
    println!("passthrough funcs : {}", r.passthrough_functions);
    println!();

    // Sort by (function-hits desc, name asc) so the highest-priority
    // blockers appear first.
    let mut rows: Vec<(&BlockerKind, usize, usize)> = r
        .by_kind
        .iter()
        .map(|(k, (h, o))| (k, *h, *o))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.to_string().cmp(&b.0.to_string())));

    println!(
        "{:<30} | {:<13} | {}",
        "BlockerKind", "functions hit", "total occurrences"
    );
    println!("{}", "-".repeat(70));
    for (kind, hits, occ) in rows {
        println!("{:<30} | {:<13} | {}", kind.to_string(), hits, occ);
    }
}

// ---------------------------------------------------------------------
// Tree walker: every FuncLiteral reachable from the module, plus the
// top-level Item::Function / Item::PassThroughFunction.
// ---------------------------------------------------------------------

fn walk_items<'a>(items: &'a [Item], cb: &mut dyn FnMut(&'a FuncLiteral)) {
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

fn walk_stmts<'a>(stmts: &'a [ir::tree::Stmt], cb: &mut dyn FnMut(&'a FuncLiteral)) {
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
            Stmt::Switch { discriminant, cases } => {
                walk_expr(discriminant, cb);
                for c in cases {
                    if let Some(test) = &c.test {
                        walk_expr(test, cb);
                    }
                    walk_stmts(&c.body, cb);
                }
            }
            Stmt::Let { init: Some(e), .. } | Stmt::Const { init: e, .. } => {
                walk_expr(e, cb);
            }
            Stmt::Assign { value, .. } => walk_expr(value, cb),
            Stmt::ExprStmt(e) | Stmt::Return(Some(e)) | Stmt::Throw(e) => walk_expr(e, cb),
            _ => {}
        }
    }
}

fn walk_expr<'a>(e: &'a ir::tree::Expr, cb: &mut dyn FnMut(&'a FuncLiteral)) {
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

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Aggregating against a fixture with two distinct blockers should
    /// surface both BlockerKinds in the report, and the
    /// passthrough-function counter should reflect both functions.
    #[test]
    fn fixture_surfaces_expected_blockers() {
        let dir = tempdir();
        let path = dir.join("fixture.js");
        let mut f = fs::File::create(&path).unwrap();
        // Two PT functions, each with a distinct, easy-to-name blocker:
        //   - `f` has a default parameter (DefaultParam)
        //   - `g` has a for-in loop (ForInStmt)
        writeln!(
            f,
            r#"
            function f(x = 1) {{ return x; }}
            function g(o) {{
                var s = 0;
                for (var k in o) {{ s = s + 1; }}
                return s;
            }}
            "#,
        )
        .unwrap();
        drop(f);

        let report = aggregate_roots(&[dir.clone()]);
        assert_eq!(report.samples_walked, 1);
        assert_eq!(
            report.passthrough_functions, 2,
            "both fixture functions should be PT; got {:?}",
            report.by_kind
        );
        let (dp_hits, dp_occ) = report
            .by_kind
            .get(&BlockerKind::DefaultParam)
            .copied()
            .unwrap_or_default();
        assert_eq!(dp_hits, 1, "DefaultParam hits: {:?}", report.by_kind);
        assert_eq!(dp_occ, 1);
        let (fi_hits, _) = report
            .by_kind
            .get(&BlockerKind::ForInStmt)
            .copied()
            .unwrap_or_default();
        assert_eq!(fi_hits, 1, "ForInStmt hits: {:?}", report.by_kind);

        // Cleanup.
        let _ = fs::remove_dir_all(&dir);
    }

    /// Empty corpus root yields an empty report (and doesn't panic).
    #[test]
    fn empty_root_is_empty_report() {
        let dir = tempdir();
        let report = aggregate_roots(&[dir.clone()]);
        assert_eq!(report.samples_walked, 0);
        assert!(report.by_kind.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    fn tempdir() -> PathBuf {
        let base = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let p = base.join(format!("blocker_freq_test_{pid}_{nanos}"));
        fs::create_dir_all(&p).unwrap();
        p
    }
}
