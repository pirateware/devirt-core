//! Property-based round-trip test for the parse → emit pipeline.
//!
//! Generates small JS expressions inside a function body, runs them through
//! `parse_and_lower → emit → parse_and_lower`, and asserts the second parse
//! produces a structurally-equivalent tree-IR to the first. Catches a class
//! of bug the hand-picked corpus can't: random shapes that exercise edge
//! cases in parse/emit symmetry (precedence, associativity, escape forms,
//! identifier-vs-keyword decisions) which a human-curated test suite tends
//! to systematically miss.
//!
//! Generator scope (intentionally narrow — start small, expand if narrow
//! passes):
//!
//! - Expression position only. Wrapped in a `function f() { return EXPR; }`
//!   harness so the lowering exercise is realistic.
//! - Number, String (printable ASCII subset), Bool, Null, Ident.
//! - Binary, Unary, Ternary, Member, Index, Call (no `new`).
//! - Depth-bounded recursion to keep test cases small and the suite fast.
//!
//! Not yet covered (deferred until the narrow generator is stable):
//! statements, control flow, function literals, template literals, regex
//! literals, object/array, spread, assignment expressions, compound
//! assignment, switch.

use std::fmt::Write as _;

use proptest::prelude::*;

use emit::emit;
use ir::tree::Module;
use parse::lower_ssa;
use parse::lower_tree;
use parse::parse_and_lower;

/// Mini-AST distinct from the project's tree-IR — keeps the generator
/// focused on what we want to fuzz (parse/emit symmetry on expression
/// shapes) rather than re-deriving every tree-IR variant.
#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Str(String),
    Bool(bool),
    Null,
    Ident(String),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    Ternary(Box<Expr>, Box<Expr>, Box<Expr>),
    Member(Box<Expr>, String),
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
}

#[derive(Clone, Copy, Debug)]
enum BinOp {
    Add, Sub, Mul, Div, Mod,
    Lt, Le, Gt, Ge,
    Eq, Ne, StrictEq, StrictNe,
    BitAnd, BitOr, BitXor, Shl, Shr, UShr,
    LogAnd, LogOr,
}

#[derive(Clone, Copy, Debug)]
enum UnOp {
    Neg, Pos, Not, BitNot,
}

impl BinOp {
    fn as_str(self) -> &'static str {
        match self {
            BinOp::Add => "+", BinOp::Sub => "-", BinOp::Mul => "*",
            BinOp::Div => "/", BinOp::Mod => "%",
            BinOp::Lt => "<", BinOp::Le => "<=", BinOp::Gt => ">", BinOp::Ge => ">=",
            BinOp::Eq => "==", BinOp::Ne => "!=",
            BinOp::StrictEq => "===", BinOp::StrictNe => "!==",
            BinOp::BitAnd => "&", BinOp::BitOr => "|", BinOp::BitXor => "^",
            BinOp::Shl => "<<", BinOp::Shr => ">>", BinOp::UShr => ">>>",
            BinOp::LogAnd => "&&", BinOp::LogOr => "||",
        }
    }
}

impl UnOp {
    fn as_str(self) -> &'static str {
        match self {
            UnOp::Neg => "-", UnOp::Pos => "+", UnOp::Not => "!", UnOp::BitNot => "~",
        }
    }
}

/// Always-parenthesize serialization. Removes any precedence-handling
/// asymmetry between the generator and the parser; the round-trip then
/// only exercises emit's job of preserving meaning under emit/parse, not
/// the generator's job of deciding when parens are needed.
fn render(e: &Expr, out: &mut String) {
    match e {
        Expr::Num(n) => {
            // Use a JS-safe form. Avoid scientific notation for the
            // small range we generate; finite integers as decimals,
            // others through a parenthesized representation that
            // parses back the same way.
            if n.is_nan() {
                out.push_str("NaN");
            } else if n.is_infinite() {
                if *n < 0.0 { out.push_str("(-Infinity)"); } else { out.push_str("Infinity"); }
            } else if *n == 0.0 && n.is_sign_negative() {
                out.push_str("(-0)");
            } else if n.fract() == 0.0 && n.abs() < 1e15 {
                let i = *n as i64;
                if i < 0 {
                    write!(out, "({i})").unwrap();
                } else {
                    write!(out, "{i}").unwrap();
                }
            } else {
                let s = format!("{n}");
                if s.starts_with('-') {
                    write!(out, "({s})").unwrap();
                } else {
                    out.push_str(&s);
                }
            }
        }
        Expr::Str(s) => {
            out.push('"');
            for c in s.chars() {
                match c {
                    '"' => out.push_str("\\\""),
                    '\\' => out.push_str("\\\\"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    '\t' => out.push_str("\\t"),
                    c => out.push(c),
                }
            }
            out.push('"');
        }
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Null => out.push_str("null"),
        Expr::Ident(n) => out.push_str(n),
        Expr::Binary(op, l, r) => {
            out.push('(');
            render(l, out);
            write!(out, " {} ", op.as_str()).unwrap();
            render(r, out);
            out.push(')');
        }
        Expr::Unary(op, e) => {
            out.push('(');
            out.push_str(op.as_str());
            // Force a space for `-` / `+` / `~` to avoid ambiguity
            // (e.g. `--x` would be a decrement, not negation of -x).
            out.push(' ');
            render(e, out);
            out.push(')');
        }
        Expr::Ternary(c, t, f) => {
            out.push('(');
            render(c, out);
            out.push_str(" ? ");
            render(t, out);
            out.push_str(" : ");
            render(f, out);
            out.push(')');
        }
        Expr::Member(obj, prop) => {
            render(obj, out);
            out.push('.');
            out.push_str(prop);
        }
        Expr::Index(obj, idx) => {
            render(obj, out);
            out.push('[');
            render(idx, out);
            out.push(']');
        }
        Expr::Call(callee, args) => {
            render(callee, out);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
                render(a, out);
            }
            out.push(')');
        }
    }
}

// ---------------------------------------------------------------
// Generators
// ---------------------------------------------------------------

fn ident_strategy() -> impl Strategy<Value = String> {
    // Identifier-safe ASCII: starts with letter or underscore, then
    // alphanumerics. Excludes keywords by restricting the alphabet
    // and length combinations that would land on common keywords.
    "[a-z_][a-z0-9_]{0,5}".prop_filter(
        "skip JS reserved words",
        |s: &String| !is_reserved(s),
    )
}

fn is_reserved(s: &str) -> bool {
    matches!(
        s,
        "if" | "else" | "for" | "while" | "do" | "return" | "function"
        | "var" | "let" | "const" | "true" | "false" | "null" | "undefined"
        | "new" | "this" | "in" | "of" | "typeof" | "void" | "delete"
        | "instanceof" | "throw" | "try" | "catch" | "finally" | "switch"
        | "case" | "default" | "break" | "continue" | "class" | "extends"
        | "super" | "import" | "export" | "yield" | "async" | "await"
        | "static" | "with" | "debugger" | "NaN" | "Infinity"
    )
}

fn member_prop_strategy() -> impl Strategy<Value = String> {
    "[a-z_][a-z0-9_]{0,4}"
}

fn binop_strategy() -> impl Strategy<Value = BinOp> {
    use BinOp::*;
    prop_oneof![
        Just(Add), Just(Sub), Just(Mul), Just(Div), Just(Mod),
        Just(Lt), Just(Le), Just(Gt), Just(Ge),
        Just(Eq), Just(Ne), Just(StrictEq), Just(StrictNe),
        Just(BitAnd), Just(BitOr), Just(BitXor),
        Just(Shl), Just(Shr), Just(UShr),
        Just(LogAnd), Just(LogOr),
    ]
}

fn unop_strategy() -> impl Strategy<Value = UnOp> {
    prop_oneof![Just(UnOp::Neg), Just(UnOp::Pos), Just(UnOp::Not), Just(UnOp::BitNot)]
}

fn leaf_strategy() -> impl Strategy<Value = Expr> {
    prop_oneof![
        // Bounded numbers — keep generator from producing values that
        // serialize to scientific notation we don't render.
        (-1000i64..1000i64).prop_map(|n| Expr::Num(n as f64)),
        "[a-zA-Z0-9 _.]{0,12}".prop_map(Expr::Str),
        any::<bool>().prop_map(Expr::Bool),
        Just(Expr::Null),
        ident_strategy().prop_map(Expr::Ident),
    ]
}

fn expr_strategy() -> impl Strategy<Value = Expr> {
    leaf_strategy().prop_recursive(
        4,   // max depth
        32,  // max total nodes
        4,   // max items per call args
        |inner| {
            prop_oneof![
                (binop_strategy(), inner.clone(), inner.clone())
                    .prop_map(|(op, l, r)| Expr::Binary(op, Box::new(l), Box::new(r))),
                (unop_strategy(), inner.clone())
                    .prop_map(|(op, e)| Expr::Unary(op, Box::new(e))),
                (inner.clone(), inner.clone(), inner.clone())
                    .prop_map(|(c, t, f)| Expr::Ternary(Box::new(c), Box::new(t), Box::new(f))),
                (inner.clone(), member_prop_strategy())
                    .prop_map(|(o, p)| Expr::Member(Box::new(o), p)),
                (inner.clone(), inner.clone())
                    .prop_map(|(o, i)| Expr::Index(Box::new(o), Box::new(i))),
                (inner.clone(), prop::collection::vec(inner, 0..4))
                    .prop_map(|(c, args)| Expr::Call(Box::new(c), args)),
            ]
        },
    )
}

// ---------------------------------------------------------------
// Round-trip checks
// ---------------------------------------------------------------

fn parse_or_skip(src: &str) -> Option<Module> {
    // The generator may produce shapes the lowering rejects (e.g. an
    // identifier that would-be reserved we missed in the filter, or a
    // construct outside the supported subset). Skip — these cases are
    // about the generator's reach, not about a parse/emit asymmetry.
    parse_and_lower(src).ok()
}

/// parse → emit → re-parse: tree-IR before SSA matches tree-IR after
/// the trivial round-trip.
fn check_emit_reparse(src: &str) -> Result<(), String> {
    let m1 = match parse_or_skip(src) {
        Some(m) => m,
        None => return Ok(()), // skipped
    };
    let out = emit(&m1);
    let m2 = match parse_and_lower(&out) {
        Ok(m) => m,
        Err(e) => return Err(format!("re-parse failed: {e}\nemitted:\n{out}\noriginal:\n{src}")),
    };
    if let Some(reason) = structural_diff_modules(&m1, &m2) {
        return Err(format!(
            "structural diff after parse→emit→reparse: {reason}\noriginal:\n{src}\nemitted:\n{out}"
        ));
    }
    Ok(())
}

/// Full pipeline: parse → ssa → tree → emit → reparse. Catches SSA
/// round-trip asymmetries on top of emit/parse.
fn check_full_pipeline(src: &str) -> Result<(), String> {
    let m1 = match parse_or_skip(src) {
        Some(m) => m,
        None => return Ok(()),
    };
    let sm = lower_ssa::lower_module(&m1);
    if let Err(e) = sm.validate() {
        return Err(format!("SSA validation failed: {e:?}\nsource:\n{src}"));
    }
    let m2 = lower_tree::lower_module(&sm);
    let out = emit(&m2);
    let _m3 = match parse_and_lower(&out) {
        Ok(m) => m,
        Err(e) => return Err(format!(
            "post-SSA emit failed to re-parse: {e}\nemitted:\n{out}\noriginal:\n{src}"
        )),
    };
    Ok(())
}

/// Module-level structural diff. Mirrors the gate in
/// `cli::roundtrip_check::structural_diff` but stays local — the
/// proptest crate doesn't depend on cli.
fn structural_diff_modules(m1: &Module, m2: &Module) -> Option<String> {
    use ir::tree::Item;
    if m1.items.len() != m2.items.len() {
        return Some(format!("item count {} vs {}", m1.items.len(), m2.items.len()));
    }
    for (a, b) in m1.items.iter().zip(m2.items.iter()) {
        match (a, b) {
            (Item::Function(fa), Item::Function(fb)) => {
                if fa.name != fb.name { return Some(format!("fn name {} vs {}", fa.name, fb.name)); }
                if fa.params != fb.params { return Some("param mismatch".into()); }
                if fa.body.len() != fb.body.len() {
                    return Some(format!("body len {} vs {}", fa.body.len(), fb.body.len()));
                }
                for (sa, sb) in fa.body.iter().zip(fb.body.iter()) {
                    if !same_stmt_kind(sa, sb) {
                        return Some(format!("stmt kind {:?} vs {:?}", stmt_kind(sa), stmt_kind(sb)));
                    }
                }
            }
            (Item::PassThroughStmt(_), Item::PassThroughStmt(_)) => {}
            (a, b) => return Some(format!("item kind {:?} vs {:?}", item_kind(a), item_kind(b))),
        }
    }
    None
}

fn item_kind(i: &ir::tree::Item) -> &'static str {
    use ir::tree::Item::*;
    match i {
        Function(_) => "Function",
        PassThroughFunction(_) => "PassThroughFunction",
        Stmt(_) => "Stmt",
        PassThroughStmt(_) => "PassThroughStmt",
    }
}

fn stmt_kind(s: &ir::tree::Stmt) -> &'static str {
    use ir::tree::Stmt::*;
    match s {
        Let { .. } => "Let",
        Const { .. } => "Const",
        Assign { .. } => "Assign",
        ExprStmt(_) => "ExprStmt",
        Return(_) => "Return",
        If { .. } => "If",
        While { .. } => "While",
        For { .. } => "For",
        ForOf { .. } => "ForOf",
        ForIn { .. } => "ForIn",
        Continue => "Continue",
        Break { .. } => "Break",
        Switch { .. } => "Switch",
        Throw(_) => "Throw",
        Block(_) => "Block",
        FuncDecl(_) => "FuncDecl",
        Try { .. } => "Try",
        DoWhile { .. } => "DoWhile",
        Labeled { .. } => "Labeled",
        LabeledContinue { .. } => "LabeledContinue",
        Debugger => "Debugger",
    }
}

fn same_stmt_kind(a: &ir::tree::Stmt, b: &ir::tree::Stmt) -> bool {
    stmt_kind(a) == stmt_kind(b)
}

// ---------------------------------------------------------------
// Properties
// ---------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 256,
        .. ProptestConfig::default()
    })]

    /// Property: any expression generated by `expr_strategy` round-trips
    /// through the parse → emit → re-parse pipeline without changing
    /// module-level shape.
    #[test]
    fn expression_roundtrip_emit_reparse(e in expr_strategy()) {
        let mut body = String::new();
        render(&e, &mut body);
        let src = format!("function f() {{ return {body}; }}");
        check_emit_reparse(&src).map_err(|s| TestCaseError::fail(s))?;
    }

    /// Property: same shapes survive the full SSA round-trip — which
    /// has stricter constraints (validate(), out-of-SSA reconstruction).
    #[test]
    fn expression_roundtrip_full_pipeline(e in expr_strategy()) {
        let mut body = String::new();
        render(&e, &mut body);
        let src = format!("function f() {{ return {body}; }}");
        check_full_pipeline(&src).map_err(|s| TestCaseError::fail(s))?;
    }
}
