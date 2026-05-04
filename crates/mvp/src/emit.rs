//! Tree-IR → JS source.
//!
//! Direct string emission. A real implementation would build an OXC AST and
//! run oxc_codegen, but for the MVP this is dramatically simpler.

use std::fmt::Write as _;

use crate::ir::tree::{Expr, IrFunction, Item, Module, Stmt, UnOp, binop_str, unop_str};

pub fn emit(module: &Module) -> String {
    let mut out = String::new();
    let mut p = Printer {
        buf: &mut out,
        indent: 0,
    };
    for (i, item) in module.items.iter().enumerate() {
        if i > 0 {
            p.buf.push('\n');
        }
        p.item(item);
    }
    out
}

struct Printer<'a> {
    buf: &'a mut String,
    indent: usize,
}

impl Printer<'_> {
    fn pad(&mut self) {
        for _ in 0..self.indent {
            self.buf.push_str("  ");
        }
    }

    fn item(&mut self, item: &Item) {
        match item {
            Item::Function(f) => {
                self.function(f);
                self.buf.push('\n');
            }
            Item::Stmt(s) => {
                self.pad();
                self.stmt(s);
                self.buf.push('\n');
            }
        }
    }

    fn function(&mut self, f: &IrFunction) {
        self.pad();
        write!(self.buf, "function {}(", f.name).unwrap();
        for (i, p) in f.params.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            self.buf.push_str(p);
        }
        self.buf.push_str(") {\n");
        self.indent += 1;
        for s in &f.body {
            self.pad();
            self.stmt(s);
            self.buf.push('\n');
        }
        self.indent -= 1;
        self.pad();
        self.buf.push('}');
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, init } => {
                write!(self.buf, "let {name}").unwrap();
                if let Some(e) = init {
                    self.buf.push_str(" = ");
                    self.expr(e);
                }
                self.buf.push(';');
            }
            Stmt::Const { name, init } => {
                write!(self.buf, "const {name} = ").unwrap();
                self.expr(init);
                self.buf.push(';');
            }
            Stmt::Assign { target, value } => {
                self.expr(target);
                self.buf.push_str(" = ");
                self.expr(value);
                self.buf.push(';');
            }
            Stmt::ExprStmt(e) => {
                self.expr(e);
                self.buf.push(';');
            }
            Stmt::Return(e) => {
                self.buf.push_str("return");
                if let Some(e) = e {
                    self.buf.push(' ');
                    self.expr(e);
                }
                self.buf.push(';');
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                self.buf.push_str("if (");
                self.expr(cond);
                self.buf.push_str(") {\n");
                self.indent += 1;
                for s in then_body {
                    self.pad();
                    self.stmt(s);
                    self.buf.push('\n');
                }
                self.indent -= 1;
                self.pad();
                self.buf.push('}');
                if let Some(eb) = else_body {
                    self.buf.push_str(" else {\n");
                    self.indent += 1;
                    for s in eb {
                        self.pad();
                        self.stmt(s);
                        self.buf.push('\n');
                    }
                    self.indent -= 1;
                    self.pad();
                    self.buf.push('}');
                }
            }
            Stmt::While { cond, body } => {
                self.buf.push_str("while (");
                self.expr(cond);
                self.buf.push_str(") {\n");
                self.indent += 1;
                for s in body {
                    self.pad();
                    self.stmt(s);
                    self.buf.push('\n');
                }
                self.indent -= 1;
                self.pad();
                self.buf.push('}');
            }
            Stmt::Block(stmts) => {
                self.buf.push_str("{\n");
                self.indent += 1;
                for s in stmts {
                    self.pad();
                    self.stmt(s);
                    self.buf.push('\n');
                }
                self.indent -= 1;
                self.pad();
                self.buf.push('}');
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(self.buf, "{}", *n as i64).unwrap();
                } else {
                    write!(self.buf, "{n}").unwrap();
                }
            }
            Expr::String(s) => {
                self.buf.push('"');
                for c in s.chars() {
                    match c {
                        '\\' => self.buf.push_str("\\\\"),
                        '"' => self.buf.push_str("\\\""),
                        '\n' => self.buf.push_str("\\n"),
                        '\r' => self.buf.push_str("\\r"),
                        '\t' => self.buf.push_str("\\t"),
                        c => self.buf.push(c),
                    }
                }
                self.buf.push('"');
            }
            Expr::Bool(b) => self.buf.push_str(if *b { "true" } else { "false" }),
            Expr::Null => self.buf.push_str("null"),
            Expr::Undefined => self.buf.push_str("undefined"),
            Expr::Ident(name) => self.buf.push_str(name),
            Expr::Binary { op, lhs, rhs } => {
                // Always parenthesize. Not pretty, but trivially correct.
                self.buf.push('(');
                self.expr(lhs);
                write!(self.buf, " {} ", binop_str(*op)).unwrap();
                self.expr(rhs);
                self.buf.push(')');
            }
            Expr::Unary { op, operand } => {
                self.buf.push('(');
                self.buf.push_str(unop_str(*op));
                if matches!(op, UnOp::Typeof | UnOp::Void) {
                    self.buf.push(' ');
                }
                self.expr(operand);
                self.buf.push(')');
            }
            Expr::Call { callee, args } => {
                self.expr(callee);
                self.buf.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.expr(a);
                }
                self.buf.push(')');
            }
            Expr::Member { obj, prop } => {
                self.expr(obj);
                write!(self.buf, ".{prop}").unwrap();
            }
            Expr::Index { obj, index } => {
                self.expr(obj);
                self.buf.push('[');
                self.expr(index);
                self.buf.push(']');
            }
        }
    }
}
