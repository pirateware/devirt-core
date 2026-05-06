//! Dump fallthrough usage across switches in JS sources.
//!
//! For every `switch` statement found in the input file (or directory of
//! files), emits one row per case describing whether its body terminates
//! (break / return / throw / continue) or falls through to the next case.
//!
//! Usage:
//!   cargo run --example dump_switch_fallthrough -p parse -- <path>...
//!
//! Pre-work for the SwitchStmt + BreakStmt ticket: sizes the testing
//! surface for explicit fallthrough before we commit to the IR shape.

use std::fs;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::SourceType;

fn main() {
    let args: Vec<PathBuf> = std::env::args().skip(1).map(PathBuf::from).collect();
    if args.is_empty() {
        eprintln!("usage: dump_switch_fallthrough <path>...");
        std::process::exit(1);
    }

    let mut total_switches = 0usize;
    let mut total_cases = 0usize;
    let mut total_falls = 0usize;
    let mut total_empty_falls = 0usize; // case body is empty AND falls through

    for arg in &args {
        walk_path(arg, &mut |path| {
            let src = match fs::read_to_string(path) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("read {}: {}", path.display(), e);
                    return;
                }
            };
            let allocator = Allocator::default();
            let ret = Parser::new(&allocator, &src, SourceType::default()).parse();
            if !ret.errors.is_empty() {
                eprintln!("parse {}: errors", path.display());
                return;
            }
            let mut switches: Vec<&js::SwitchStatement> = Vec::new();
            collect_switches_program(&ret.program, &mut switches);
            if switches.is_empty() {
                return;
            }
            println!("=== {} ({} switches) ===", path.display(), switches.len());
            for (si, s) in switches.iter().enumerate() {
                total_switches += 1;
                println!("  switch #{} ({} cases):", si, s.cases.len());
                for (ci, c) in s.cases.iter().enumerate() {
                    total_cases += 1;
                    let is_default = c.test.is_none();
                    let last_kind = case_last_terminator(&c.consequent);
                    let is_last = ci + 1 == s.cases.len();
                    let falls = match &last_kind {
                        Some(k) => matches!(k.as_str(), "fall"),
                        None => true, // empty body
                    };
                    if falls && !is_last {
                        total_falls += 1;
                        if c.consequent.is_empty() {
                            total_empty_falls += 1;
                        }
                    }
                    let kind_str = last_kind.as_deref().unwrap_or("<empty>");
                    println!(
                        "    case[{}]{}{}: {} stmts, end={}{}",
                        ci,
                        if is_default { " default" } else { "" },
                        if is_last { " last" } else { "" },
                        c.consequent.len(),
                        kind_str,
                        if falls && !is_last { "  ⇒ FALLTHROUGH" } else { "" },
                    );
                }
            }
        });
    }

    println!();
    println!("== summary ==");
    println!("switches: {}", total_switches);
    println!("cases:    {}", total_cases);
    println!("fallthrough cases (non-last, non-terminating): {}", total_falls);
    println!("  of which empty-body (case A: case B:):       {}", total_empty_falls);
}

fn walk_path(path: &Path, cb: &mut dyn FnMut(&Path)) {
    if path.is_file() {
        if path.extension().and_then(|s| s.to_str()) == Some("js") {
            cb(path);
        }
        return;
    }
    if path.is_dir() {
        for entry in fs::read_dir(path).unwrap().flatten() {
            walk_path(&entry.path(), cb);
        }
    }
}

/// Classify the last statement of a case body.
/// - `Some("break"|"return"|"throw"|"continue")` if terminating.
/// - `Some("fall")` if last stmt is non-terminating (case falls through).
/// - `None` if body is empty.
fn case_last_terminator(stmts: &[js::Statement]) -> Option<String> {
    let last = stmts.last()?;
    Some(match last {
        js::Statement::BreakStatement(_) => "break".into(),
        js::Statement::ReturnStatement(_) => "return".into(),
        js::Statement::ThrowStatement(_) => "throw".into(),
        js::Statement::ContinueStatement(_) => "continue".into(),
        js::Statement::BlockStatement(b) => {
            // Recurse into a trailing block — `case 1: { ...; break; }`.
            match case_last_terminator(&b.body) {
                Some(s) => s,
                None => "fall".into(),
            }
        }
        _ => "fall".into(),
    })
}

fn collect_switches_program<'a>(
    program: &'a js::Program<'a>,
    out: &mut Vec<&'a js::SwitchStatement<'a>>,
) {
    for s in &program.body {
        collect_switches_stmt(s, out);
    }
}

fn collect_switches_stmt<'a>(stmt: &'a js::Statement<'a>, out: &mut Vec<&'a js::SwitchStatement<'a>>) {
    use js::Statement::*;
    match stmt {
        SwitchStatement(s) => {
            out.push(s);
            for c in &s.cases {
                for ss in &c.consequent {
                    collect_switches_stmt(ss, out);
                }
            }
        }
        BlockStatement(b) => {
            for s in &b.body {
                collect_switches_stmt(s, out);
            }
        }
        IfStatement(s) => {
            collect_switches_stmt(&s.consequent, out);
            if let Some(a) = &s.alternate {
                collect_switches_stmt(a, out);
            }
        }
        WhileStatement(s) => collect_switches_stmt(&s.body, out),
        DoWhileStatement(s) => collect_switches_stmt(&s.body, out),
        ForStatement(s) => collect_switches_stmt(&s.body, out),
        ForInStatement(s) => collect_switches_stmt(&s.body, out),
        ForOfStatement(s) => collect_switches_stmt(&s.body, out),
        TryStatement(t) => {
            for s in &t.block.body {
                collect_switches_stmt(s, out);
            }
            if let Some(h) = &t.handler {
                for s in &h.body.body {
                    collect_switches_stmt(s, out);
                }
            }
            if let Some(f) = &t.finalizer {
                for s in &f.body {
                    collect_switches_stmt(s, out);
                }
            }
        }
        LabeledStatement(s) => collect_switches_stmt(&s.body, out),
        FunctionDeclaration(f) => {
            if let Some(body) = &f.body {
                for s in &body.statements {
                    collect_switches_stmt(s, out);
                }
            }
        }
        _ => {
            // Function expressions / arrows nested in expressions are not
            // walked here — sufficient for the diagnostic.
        }
    }
}
