//! js-deobfuscator MVP — parse / IR / emit, all in one file.
//!
//! Pipeline:
//!     JS source --[oxc_parser]--> OXC AST --[lower]--> IR --[emit]--> JS source
//!
//! This is intentionally a single-file demo. The real architecture
//! (SSA + CFG + effects lattice + passes + abstract interpreter) is described
//! in PLAN.md. This file shows the *shape* of the pipeline so the rest of the
//! project can be filled in around it.
//!
//! Subset of JS supported:
//!   - number / string / boolean / null / undefined literals
//!   - identifiers, member access (.x, [x])
//!   - binary + unary operators
//!   - function calls
//!   - let / const / plain `=` assignment
//!   - function declarations
//!   - if / else
//!   - while
//!   - return
//!   - block statements
//!
//! Anything outside this subset returns a structured `UnsupportedConstruct`
//! error rather than silently producing wrong IR. That's the rule from
//! CLAUDE.md: no silent approximations.
//!
//! Run:
//!     cargo run                    # runs the built-in fibonacci demo
//!     cargo run -- path/to/file.js # processes the given file

use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::SourceType;
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::process::ExitCode;

// ============================================================================
// IR
// ============================================================================
// A minimal owned tree-IR. Not yet SSA, not yet a CFG — just enough to
// demonstrate that we have our own representation distinct from the AST.

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

// ============================================================================
// Errors
// ============================================================================

#[derive(Debug)]
pub enum Error {
    Parse(String),
    Unsupported(&'static str),
    Malformed(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Parse(s) => write!(f, "parse error: {s}"),
            Error::Unsupported(what) => write!(f, "unsupported construct: {what}"),
            Error::Malformed(s) => write!(f, "malformed source: {s}"),
        }
    }
}

impl std::error::Error for Error {}

// ============================================================================
// Parse + lower
// ============================================================================
// `parse_and_lower` owns the parser arena for its whole body, then lowers
// the AST into our owned IR before the arena is dropped.

pub fn parse_and_lower(source: &str) -> Result<Module, Error> {
    let allocator = Allocator::default();
    let source_type = SourceType::default();
    let ret = Parser::new(&allocator, source, source_type).parse();
    if !ret.errors.is_empty() {
        let msg = ret
            .errors
            .iter()
            .map(|e| format!("{e:?}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Parse(msg));
    }
    lower_program(&ret.program)
}

fn lower_program(prog: &js::Program) -> Result<Module, Error> {
    let mut items = Vec::with_capacity(prog.body.len());
    for stmt in &prog.body {
        items.push(lower_top_stmt(stmt)?);
    }
    Ok(Module { items })
}

fn lower_top_stmt(stmt: &js::Statement) -> Result<Item, Error> {
    match stmt {
        js::Statement::FunctionDeclaration(f) => Ok(Item::Function(lower_function(f)?)),
        _ => Ok(Item::Stmt(lower_stmt(stmt)?)),
    }
}

fn lower_function(f: &js::Function) -> Result<IrFunction, Error> {
    let name =
        f.id.as_ref()
            .map(|i| i.name.to_string())
            .ok_or(Error::Unsupported("anonymous function declaration"))?;

    let mut params = Vec::with_capacity(f.params.items.len());
    for p in &f.params.items {
        match &p.pattern.kind {
            js::BindingPatternKind::BindingIdentifier(id) => {
                params.push(id.name.to_string());
            }
            _ => return Err(Error::Unsupported("destructuring or default parameter")),
        }
    }

    let body = match &f.body {
        Some(b) => b
            .statements
            .iter()
            .map(lower_stmt)
            .collect::<Result<Vec<_>, _>>()?,
        None => Vec::new(),
    };

    Ok(IrFunction { name, params, body })
}

fn lower_stmt(stmt: &js::Statement) -> Result<Stmt, Error> {
    match stmt {
        js::Statement::VariableDeclaration(decl) => lower_var_decl(decl),
        js::Statement::ExpressionStatement(es) => lower_expr_stmt(&es.expression),
        js::Statement::ReturnStatement(r) => {
            let v = match &r.argument {
                Some(e) => Some(lower_expr(e)?),
                None => None,
            };
            Ok(Stmt::Return(v))
        }
        js::Statement::IfStatement(s) => Ok(Stmt::If {
            cond: lower_expr(&s.test)?,
            then_body: stmts_from(&s.consequent)?,
            else_body: match &s.alternate {
                Some(alt) => Some(stmts_from(alt)?),
                None => None,
            },
        }),
        js::Statement::WhileStatement(s) => Ok(Stmt::While {
            cond: lower_expr(&s.test)?,
            body: stmts_from(&s.body)?,
        }),
        js::Statement::BlockStatement(b) => {
            let stmts = b
                .body
                .iter()
                .map(lower_stmt)
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Stmt::Block(stmts))
        }
        js::Statement::FunctionDeclaration(_) => {
            Err(Error::Unsupported("nested function declaration"))
        }
        _ => Err(Error::Unsupported(stmt_kind(stmt))),
    }
}

fn lower_var_decl(decl: &js::VariableDeclaration) -> Result<Stmt, Error> {
    if decl.declarations.len() != 1 {
        return Err(Error::Unsupported("multi-declarator variable declaration"));
    }
    let d = &decl.declarations[0];
    let name = match &d.id.kind {
        js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
        _ => return Err(Error::Unsupported("destructuring binding")),
    };
    let init = match &d.init {
        Some(e) => Some(lower_expr(e)?),
        None => None,
    };
    match decl.kind {
        js::VariableDeclarationKind::Let | js::VariableDeclarationKind::Var => {
            Ok(Stmt::Let { name, init })
        }
        js::VariableDeclarationKind::Const => {
            let init = init.ok_or_else(|| Error::Malformed("const without initializer".into()))?;
            Ok(Stmt::Const { name, init })
        }
        _ => Err(Error::Unsupported("`using` declaration")),
    }
}

fn lower_expr_stmt(expr: &js::Expression) -> Result<Stmt, Error> {
    // Assignment is a JS expression but lowers naturally to a stmt in our IR.
    if let js::Expression::AssignmentExpression(a) = expr {
        if matches!(a.operator, js::AssignmentOperator::Assign) {
            let target = lower_assign_target(&a.left)?;
            let value = lower_expr(&a.right)?;
            return Ok(Stmt::Assign { target, value });
        }
        return Err(Error::Unsupported("compound assignment (+=, -=, etc.)"));
    }
    Ok(Stmt::ExprStmt(lower_expr(expr)?))
}

fn stmts_from(stmt: &js::Statement) -> Result<Vec<Stmt>, Error> {
    if let js::Statement::BlockStatement(b) = stmt {
        b.body.iter().map(lower_stmt).collect()
    } else {
        Ok(vec![lower_stmt(stmt)?])
    }
}

fn lower_assign_target(t: &js::AssignmentTarget) -> Result<Expr, Error> {
    match t {
        js::AssignmentTarget::AssignmentTargetIdentifier(id) => {
            Ok(Expr::Ident(id.name.to_string()))
        }
        js::AssignmentTarget::StaticMemberExpression(m) => Ok(Expr::Member {
            obj: Box::new(lower_expr(&m.object)?),
            prop: m.property.name.to_string(),
        }),
        js::AssignmentTarget::ComputedMemberExpression(m) => Ok(Expr::Index {
            obj: Box::new(lower_expr(&m.object)?),
            index: Box::new(lower_expr(&m.expression)?),
        }),
        _ => Err(Error::Unsupported("complex assignment target")),
    }
}

fn lower_expr(expr: &js::Expression) -> Result<Expr, Error> {
    match expr {
        js::Expression::NumericLiteral(n) => Ok(Expr::Number(n.value)),
        js::Expression::StringLiteral(s) => Ok(Expr::String(s.value.to_string())),
        js::Expression::BooleanLiteral(b) => Ok(Expr::Bool(b.value)),
        js::Expression::NullLiteral(_) => Ok(Expr::Null),
        js::Expression::Identifier(id) => {
            if id.name == "undefined" {
                Ok(Expr::Undefined)
            } else {
                Ok(Expr::Ident(id.name.to_string()))
            }
        }
        js::Expression::ParenthesizedExpression(p) => lower_expr(&p.expression),

        js::Expression::BinaryExpression(b) => Ok(Expr::Binary {
            op: lower_binop(b.operator)?,
            lhs: Box::new(lower_expr(&b.left)?),
            rhs: Box::new(lower_expr(&b.right)?),
        }),
        js::Expression::LogicalExpression(l) => {
            let op = match l.operator {
                js::LogicalOperator::And => BinOp::LogAnd,
                js::LogicalOperator::Or => BinOp::LogOr,
                _ => return Err(Error::Unsupported("nullish coalescing (??)")),
            };
            Ok(Expr::Binary {
                op,
                lhs: Box::new(lower_expr(&l.left)?),
                rhs: Box::new(lower_expr(&l.right)?),
            })
        }
        js::Expression::UnaryExpression(u) => {
            let op = match u.operator {
                js::UnaryOperator::UnaryNegation => UnOp::Neg,
                js::UnaryOperator::UnaryPlus => UnOp::Pos,
                js::UnaryOperator::LogicalNot => UnOp::Not,
                js::UnaryOperator::BitwiseNot => UnOp::BitNot,
                js::UnaryOperator::Typeof => UnOp::Typeof,
                js::UnaryOperator::Void => UnOp::Void,
                _ => return Err(Error::Unsupported("delete operator")),
            };
            Ok(Expr::Unary {
                op,
                operand: Box::new(lower_expr(&u.argument)?),
            })
        }

        js::Expression::CallExpression(c) => {
            let callee = lower_expr(&c.callee)?;
            let mut args = Vec::with_capacity(c.arguments.len());
            for a in &c.arguments {
                match a {
                    js::Argument::SpreadElement(_) => {
                        return Err(Error::Unsupported("spread argument"));
                    }
                    other => {
                        // Argument's non-spread variants are inlined Expression variants;
                        // OXC provides a method to view them as &Expression.
                        let e = other
                            .as_expression()
                            .ok_or(Error::Unsupported("non-expression argument"))?;
                        args.push(lower_expr(e)?);
                    }
                }
            }
            Ok(Expr::Call {
                callee: Box::new(callee),
                args,
            })
        }

        js::Expression::StaticMemberExpression(m) => Ok(Expr::Member {
            obj: Box::new(lower_expr(&m.object)?),
            prop: m.property.name.to_string(),
        }),
        js::Expression::ComputedMemberExpression(m) => Ok(Expr::Index {
            obj: Box::new(lower_expr(&m.object)?),
            index: Box::new(lower_expr(&m.expression)?),
        }),

        _ => Err(Error::Unsupported("expression kind")),
    }
}

fn lower_binop(op: js::BinaryOperator) -> Result<BinOp, Error> {
    use js::BinaryOperator as B;
    Ok(match op {
        B::Addition => BinOp::Add,
        B::Subtraction => BinOp::Sub,
        B::Multiplication => BinOp::Mul,
        B::Division => BinOp::Div,
        B::Remainder => BinOp::Mod,
        B::Equality => BinOp::Eq,
        B::StrictEquality => BinOp::StrictEq,
        B::Inequality => BinOp::Ne,
        B::StrictInequality => BinOp::StrictNe,
        B::LessThan => BinOp::Lt,
        B::LessEqualThan => BinOp::Le,
        B::GreaterThan => BinOp::Gt,
        B::GreaterEqualThan => BinOp::Ge,
        B::BitwiseAnd => BinOp::BitAnd,
        B::BitwiseOR => BinOp::BitOr,
        B::BitwiseXOR => BinOp::BitXor,
        B::ShiftLeft => BinOp::Shl,
        B::ShiftRight => BinOp::Shr,
        B::ShiftRightZeroFill => BinOp::UShr,
        _ => return Err(Error::Unsupported("binary operator")),
    })
}

fn stmt_kind(s: &js::Statement) -> &'static str {
    match s {
        js::Statement::ForStatement(_) => "for statement",
        js::Statement::ForInStatement(_) => "for-in statement",
        js::Statement::ForOfStatement(_) => "for-of statement",
        js::Statement::DoWhileStatement(_) => "do-while statement",
        js::Statement::SwitchStatement(_) => "switch statement",
        js::Statement::TryStatement(_) => "try statement",
        js::Statement::ThrowStatement(_) => "throw statement",
        js::Statement::BreakStatement(_) => "break statement",
        js::Statement::ContinueStatement(_) => "continue statement",
        js::Statement::LabeledStatement(_) => "labeled statement",
        js::Statement::ClassDeclaration(_) => "class declaration",
        _ => "unrecognized statement",
    }
}

// ============================================================================
// Emit: IR -> JS source
// ============================================================================
// Direct string emission. A real implementation would build an OXC AST and
// run oxc_codegen, but for the MVP this is dramatically simpler and easier
// to read.

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

fn binop_str(op: BinOp) -> &'static str {
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

fn unop_str(op: UnOp) -> &'static str {
    match op {
        UnOp::Neg => "-",
        UnOp::Pos => "+",
        UnOp::Not => "!",
        UnOp::BitNot => "~",
        UnOp::Typeof => "typeof",
        UnOp::Void => "void",
    }
}

// ============================================================================
// IR pretty-printer (for the demo / debugging)
// ============================================================================

pub fn dump_ir(module: &Module) -> String {
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

// ============================================================================
// CLI
// ============================================================================

const DEMO: &str = r#"
function reverse(str) {
    return str.split('').reverse().join('');
}

function isPalindrome(str) {
    return cleaned == reverse(cleaned);
}

"#;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let source = if args.len() >= 2 {
        match fs::read_to_string(&args[1]) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error reading {}: {e}", args[1]);
                return ExitCode::FAILURE;
            }
        }
    } else {
        DEMO.to_string()
    };

    println!("===== INPUT =====");
    print!("{source}");
    if !source.ends_with('\n') {
        println!();
    }

    let module = match parse_and_lower(&source) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("\nerror: {e}");
            return ExitCode::FAILURE;
        }
    };

    println!("\n===== IR =====");
    print!("{}", dump_ir(&module));

    println!("===== EMITTED =====");
    print!("{}", emit(&module));

    ExitCode::SUCCESS
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(src: &str) -> String {
        let m = parse_and_lower(src).expect("parse_and_lower failed");
        let out = emit(&m);
        // Re-parse the output to make sure it's still valid JS.
        parse_and_lower(&out).expect("emitted JS failed to re-parse");
        out
    }

    #[test]
    fn fib() {
        let out = roundtrip(DEMO);
        assert!(out.contains("function fib"));
        assert!(out.contains("return"));
    }

    #[test]
    fn arithmetic() {
        let _ = roundtrip("const x = (1 + 2) * 3;");
    }

    #[test]
    fn member_and_call() {
        let _ = roundtrip("console.log(\"hi\", 42);");
    }

    #[test]
    fn while_loop() {
        let _ = roundtrip("let i = 0; while (i < 10) { i = i + 1; }");
    }

    #[test]
    fn unsupported_class_errors_loudly() {
        let err = parse_and_lower("class Foo {}").unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }

    #[test]
    fn unsupported_for_errors_loudly() {
        let err = parse_and_lower("for (let i = 0; i < 10; i++) {}").unwrap_err();
        assert!(matches!(err, Error::Unsupported(_)));
    }
}
