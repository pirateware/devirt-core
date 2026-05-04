//! Tree-IR — owned, source-shaped representation produced by lowering.
//!
//! Not yet SSA, not yet a CFG — just enough to demonstrate that the
//! pipeline owns a representation distinct from the OXC AST.

// Closure variants (`Stmt::FuncDecl`, `Expr::Func`, `FuncLiteral`,
// `FuncBody`) are wired up by later T2 stages; suppress unused-variant
// warnings until then.
#![allow(dead_code)]

use std::cell::Cell;
use std::fmt::Write as _;

use crate::ir::ssa::FuncId;

#[derive(Debug, Clone)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Function(IrFunction),
    /// Top-level function whose body contains constructs the lowering
    /// can't represent in the IR. The body is preserved as raw source
    /// text; the renamer/passes don't see it. Re-emitted verbatim
    /// (modulo the wrapper) at the final emit step.
    PassThroughFunction(FuncLiteral),
    Stmt(Stmt),
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
#[allow(clippy::enum_variant_names)]
pub enum Stmt {
    Let {
        name: String,
        init: Option<Expr>,
    },
    Const {
        name: String,
        init: Expr,
    },
    Assign {
        target: Expr,
        value: Expr,
    },
    ExprStmt(Expr),
    Return(Option<Expr>),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Block(Vec<Stmt>),
    /// Nested function declaration (`function name(...) { ... }` inside
    /// another function body). Top-level declarations stay as
    /// `Item::Function`; this variant is reserved for inner ones, which
    /// the lowering emits at their textual position (we do not hoist
    /// — see project memory: closures-disposition-rule).
    FuncDecl(FuncLiteral),
}

/// Shared shape for function expressions, arrow functions, and nested
/// function declarations. Top-level functions still use `IrFunction`;
/// the two are kept distinct because top-level functions cannot be
/// PassThrough or anonymous.
#[derive(Debug, Clone)]
pub struct FuncLiteral {
    /// `Some` for `function name(...){}` and `function name(){...}`
    /// expressions; `None` for anonymous expressions and arrows.
    pub name: Option<String>,
    pub params: Vec<String>,
    pub is_async: bool,
    pub is_generator: bool,
    pub is_arrow: bool,
    pub body: FuncBody,
    /// FuncId assigned during SSA lowering's pass 1 (function discovery).
    /// `None` until pass 1 fills it; pass 2 reads it when emitting an
    /// `AllocClosure` for the enclosing scope. `Cell` so the tree-IR
    /// stays `&`-borrowable while the converter runs.
    pub func_id: Cell<Option<FuncId>>,
    /// Verbatim source slice for this literal's body (between `{` and
    /// `}`, exclusive — same shape as `FuncBody::PassThrough(s)`).
    /// Stashed at lower.rs time for every Lowered nested literal so
    /// pass 1's forward-ref / reassigned-capture rule can re-classify
    /// the literal to PassThrough cheaply, without re-slicing source.
    /// `None` means "no source available" — top-level Lowered functions
    /// don't need this, and tests that build tree-IR by hand may omit it
    /// (re-classification is then skipped).
    pub passthrough_source: Option<String>,
    /// Set by pass 1 of SSA lowering when the forward-ref or
    /// reassigned-capture rule fires (see project memory
    /// `closures-disposition-rule`). When `true`, pass 2 routes this
    /// literal to a PassThrough body using `passthrough_source`. Tests
    /// that omit `passthrough_source` will not re-classify even if this
    /// flag is set; the violation is logged via debug_assert in that case.
    pub force_passthrough: Cell<bool>,
}

#[derive(Debug, Clone)]
pub enum FuncBody {
    /// Lowered body — the normal pipeline shape.
    Stmts(Vec<Stmt>),
    /// Arrow expression body — `(x) => x + 1`. Only valid when
    /// `is_arrow` is true.
    Expression(Box<Expr>),
    /// Source-preserved body — the original JS source text between
    /// the body's `{` and `}` (exclusive of the braces). Re-parsed and
    /// re-emitted at the final emit step.
    PassThrough(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Number(f64),
    String(String),
    Bool(bool),
    Null,
    Undefined,
    Ident(String),
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: UnOp,
        operand: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Member {
        obj: Box<Expr>,
        prop: String,
    },
    Index {
        obj: Box<Expr>,
        index: Box<Expr>,
    },
    Func(Box<FuncLiteral>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    StrictEq,
    Ne,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    LogAnd,
    LogOr,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Pos,
    Not,
    BitNot,
    Typeof,
    Void,
}

pub fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Mod => "%",
        BinOp::Eq => "==",
        BinOp::StrictEq => "===",
        BinOp::Ne => "!=",
        BinOp::StrictNe => "!==",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::LogAnd => "&&",
        BinOp::LogOr => "||",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::UShr => ">>>",
    }
}

pub fn unop_str(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Pos => "+",
        UnOp::Not => "!",
        UnOp::BitNot => "~",
        UnOp::Typeof => "typeof",
        UnOp::Void => "void",
    }
}

pub fn dump(module: &Module) -> String {
    let mut out = String::new();
    for item in &module.items {
        match item {
            Item::Function(f) => {
                writeln!(&mut out, "fn {}({}) {{", f.name, f.params.join(", ")).unwrap();
                for s in &f.body {
                    dump_stmt(&mut out, s, 1);
                }
                out.push_str("}\n\n");
            }
            Item::PassThroughFunction(lit) => {
                let name = lit.name.as_deref().unwrap_or("<anon>");
                writeln!(&mut out, "fn {name}({}) passthrough", lit.params.join(", ")).unwrap();
                if let FuncBody::PassThrough(src) = &lit.body {
                    for line in src.lines() {
                        writeln!(&mut out, "  {line}").unwrap();
                    }
                }
                out.push_str("end\n\n");
            }
            Item::Stmt(s) => {
                dump_stmt(&mut out, s, 0);
            }
        }
    }
    out
}

fn dump_stmt(out: &mut String, s: &Stmt, depth: usize) {
    let pad = "  ".repeat(depth);
    match s {
        Stmt::Let { name, init } => {
            write!(out, "{pad}let {name}").unwrap();
            if let Some(e) = init {
                write!(out, " = {}", dump_expr(e)).unwrap();
            }
            out.push('\n');
        }
        Stmt::Const { name, init } => {
            writeln!(out, "{pad}const {name} = {}", dump_expr(init)).unwrap();
        }
        Stmt::Assign { target, value } => {
            writeln!(out, "{pad}{} = {}", dump_expr(target), dump_expr(value)).unwrap();
        }
        Stmt::ExprStmt(e) => writeln!(out, "{pad}{}", dump_expr(e)).unwrap(),
        Stmt::Return(e) => match e {
            Some(e) => writeln!(out, "{pad}return {}", dump_expr(e)).unwrap(),
            None => writeln!(out, "{pad}return").unwrap(),
        },
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            writeln!(out, "{pad}if {} {{", dump_expr(cond)).unwrap();
            for s in then_body {
                dump_stmt(out, s, depth + 1);
            }
            if let Some(eb) = else_body {
                writeln!(out, "{pad}}} else {{").unwrap();
                for s in eb {
                    dump_stmt(out, s, depth + 1);
                }
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        Stmt::While { cond, body } => {
            writeln!(out, "{pad}while {} {{", dump_expr(cond)).unwrap();
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        Stmt::Block(stmts) => {
            writeln!(out, "{pad}{{").unwrap();
            for s in stmts {
                dump_stmt(out, s, depth + 1);
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        Stmt::FuncDecl(lit) => {
            dump_func_literal(out, lit, depth);
        }
    }
}

fn dump_func_literal(out: &mut String, lit: &FuncLiteral, depth: usize) {
    let pad = "  ".repeat(depth);
    let name = lit.name.as_deref().unwrap_or("<anon>");
    let kw = if lit.is_arrow {
        "arrow"
    } else if lit.is_generator {
        "fn*"
    } else {
        "fn"
    };
    let modifiers = if lit.is_async { "async " } else { "" };
    write!(
        out,
        "{pad}{modifiers}{kw} {name}({})",
        lit.params.join(", ")
    )
    .unwrap();
    match &lit.body {
        FuncBody::Stmts(body) => {
            out.push_str(" {\n");
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        FuncBody::Expression(e) => {
            writeln!(out, " => {}", dump_expr(e)).unwrap();
        }
        FuncBody::PassThrough(src) => {
            out.push_str(" passthrough {\n");
            for line in src.lines() {
                writeln!(out, "{pad}  {line}").unwrap();
            }
            writeln!(out, "{pad}}}").unwrap();
        }
    }
}

fn dump_expr(e: &Expr) -> String {
    match e {
        Expr::Number(n) => format!("{n}"),
        Expr::String(s) => format!("{s:?}"),
        Expr::Bool(b) => format!("{b}"),
        Expr::Null => "null".to_string(),
        Expr::Undefined => "undefined".to_string(),
        Expr::Ident(n) => n.clone(),
        Expr::Binary { op, lhs, rhs } => {
            format!("({} {} {})", dump_expr(lhs), binop_str(*op), dump_expr(rhs))
        }
        Expr::Unary { op, operand } => format!("({}{})", unop_str(*op), dump_expr(operand)),
        Expr::Call { callee, args } => format!(
            "{}({})",
            dump_expr(callee),
            args.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::Member { obj, prop } => format!("{}.{}", dump_expr(obj), prop),
        Expr::Index { obj, index } => format!("{}[{}]", dump_expr(obj), dump_expr(index)),
        Expr::Func(lit) => {
            let name = lit.name.as_deref().unwrap_or("<anon>");
            let kw = if lit.is_arrow {
                "arrow"
            } else if lit.is_generator {
                "fn*"
            } else {
                "fn"
            };
            let modifiers = if lit.is_async { "async " } else { "" };
            let body_marker = match &lit.body {
                FuncBody::Stmts(_) => "{...}",
                FuncBody::Expression(_) => "=> ...",
                FuncBody::PassThrough(_) => "passthrough {...}",
            };
            format!(
                "{modifiers}{kw} {name}({}) {body_marker}",
                lit.params.join(", ")
            )
        }
    }
}
