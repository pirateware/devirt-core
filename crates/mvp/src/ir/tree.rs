//! Tree-IR — owned, source-shaped representation produced by lowering.
//!
//! Not yet SSA, not yet a CFG — just enough to demonstrate that the
//! pipeline owns a representation distinct from the OXC AST.

use std::fmt::Write as _;

#[derive(Debug, Clone)]
pub struct Module {
    pub items: Vec<Item>,
}

#[derive(Debug, Clone)]
pub enum Item {
    Function(IrFunction),
    Stmt(Stmt),
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone)]
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
    }
}
