//! Tree-IR → JS source.
//!
//! Direct string emission. A real implementation would build an OXC AST and
//! run oxc_codegen, but for the MVP this is dramatically simpler.

use std::fmt::Write as _;

use ir::tree::{
    Expr, FuncBody, FuncLiteral, IrFunction, Item, Module, ObjectKey, Stmt, UnOp, binop_str,
    unop_str,
};

/// Re-parse a PassThrough body via `oxc_parser` and serialize it back
/// with `oxc_codegen`, producing `{ ... }` text. The wrapper around the
/// body synthesizes a function whose form (regular / async /
/// generator) matches the captured literal so the parser accepts
/// `await` / `yield` inside the body.
///
/// Arrow PassThrough is currently unreachable — disposition does not
/// route arrows to PassThrough today. We still synthesize a regular-
/// function wrapper for them; the body grammar overlaps for everything
/// the parser cares about (arrows can't contain `yield` even if their
/// body is a block, and `await` is allowed only in async arrows; both
/// cases route through the regular/async wrapper as configured).
fn reparse_and_codegen_body(lit: &FuncLiteral, src: &str) -> String {
    use oxc_allocator::Allocator;
    use oxc_codegen::{Codegen, Context, Gen};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let params_joined = lit.params.join(", ");
    let keyword = if lit.is_async {
        if lit.is_generator {
            "async function*"
        } else {
            "async function"
        }
    } else if lit.is_generator {
        "function*"
    } else {
        "function"
    };
    // Parenthesize so it lands as an expression in the parser.
    let wrapper = format!("({} ({}) {{{}}})", keyword, params_joined, src);

    let allocator = Allocator::default();
    let source_type = SourceType::default();
    let parsed = Parser::new(&allocator, &wrapper, source_type).parse();
    if !parsed.errors.is_empty() {
        panic!(
            "PassThrough body re-parse failed for `{}`:\n{:#?}\nwrapper:\n{}",
            lit.name.as_deref().unwrap_or("<anon>"),
            parsed.errors,
            wrapper
        );
    }
    let program = parsed.program;

    // Walk down: Program → ExpressionStatement → ParenthesizedExpression
    // (or directly Function) → Function. We only need the function body.
    let stmt = program
        .body
        .first()
        .expect("re-parsed PassThrough wrapper has no top-level statement");
    let expr = match stmt {
        oxc_ast::ast::Statement::ExpressionStatement(es) => &es.expression,
        other => panic!(
            "re-parsed PassThrough wrapper top-level statement is not an expression: {other:?}"
        ),
    };
    // Strip parenthesis wrapper.
    let func_expr = match expr {
        oxc_ast::ast::Expression::ParenthesizedExpression(p) => &p.expression,
        e => e,
    };
    let func = match func_expr {
        oxc_ast::ast::Expression::FunctionExpression(f) => f,
        other => panic!(
            "re-parsed PassThrough wrapper inner expression is not a function: {other:?}"
        ),
    };
    let body = func
        .body
        .as_ref()
        .expect("re-parsed function expression missing body");

    // Codegen the function body alone. `body.print` writes the
    // `{ ... }` form including braces. We need the original
    // source_text wired so codegen can fall back to it for any
    // span-driven decision; since Program owns it, we reuse it.
    let mut codegen = Codegen::new();
    body.print(&mut codegen, Context::default());
    codegen.into_source_text()
}

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
            Item::PassThroughFunction(lit) => {
                self.pad();
                let name = lit
                    .name
                    .as_deref()
                    .expect("Item::PassThroughFunction requires a name");
                self.write_func_header(lit, Some(name));
                self.write_func_body(lit);
                self.buf.push('\n');
            }
            Item::Stmt(s) => {
                self.pad();
                self.stmt(s);
                self.buf.push('\n');
            }
            Item::PassThroughStmt(src) => {
                // Verbatim source slice from `lower.rs::lower_top_stmt`.
                // Trailing newline is appended to keep one statement
                // per line in the emitted output, matching the other
                // arms.
                self.pad();
                self.buf.push_str(src);
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
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.buf.push_str("for (");
                if let Some(init) = init {
                    // Init can be a `let`/`var`/`const` decl or an expr
                    // statement; both kinds emit with a trailing
                    // semicolon. We strip that semicolon and add the
                    // `for`'s own separator instead.
                    let mark = self.buf.len();
                    self.stmt(init);
                    if self.buf.ends_with(';') {
                        self.buf.truncate(self.buf.len() - 1);
                    }
                    let _ = mark;
                }
                self.buf.push_str("; ");
                if let Some(c) = cond {
                    self.expr(c);
                }
                self.buf.push_str("; ");
                if let Some(s) = step {
                    self.expr(s);
                }
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
            Stmt::ForOf {
                kind,
                binding,
                iterable,
                body,
            } => {
                write!(self.buf, "for ({kind} {binding} of ").unwrap();
                self.expr(iterable);
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
            Stmt::ForIn {
                kind,
                binding,
                obj,
                body,
            } => {
                write!(self.buf, "for ({kind} {binding} in ").unwrap();
                self.expr(obj);
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
            Stmt::Continue => {
                self.buf.push_str("continue;");
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
            Stmt::FuncDecl(lit) => {
                // Inner function declarations: `function name(...) {...}`.
                // FuncDecl variants always have a name (statement-position
                // function declarations require one in JS).
                let name = lit.name.as_deref().expect("Stmt::FuncDecl requires a name");
                self.write_func_header(lit, Some(name));
                self.write_func_body(lit);
            }
            Stmt::Try {
                body,
                catch,
                finally,
            } => {
                self.buf.push_str("try {\n");
                self.indent += 1;
                for s in body {
                    self.pad();
                    self.stmt(s);
                    self.buf.push('\n');
                }
                self.indent -= 1;
                self.pad();
                self.buf.push('}');
                if let Some((param, catch_body)) = catch {
                    self.buf.push_str(" catch");
                    if let Some(p) = param {
                        write!(self.buf, " ({p})").unwrap();
                    }
                    self.buf.push_str(" {\n");
                    self.indent += 1;
                    for s in catch_body {
                        self.pad();
                        self.stmt(s);
                        self.buf.push('\n');
                    }
                    self.indent -= 1;
                    self.pad();
                    self.buf.push('}');
                }
                if let Some(finally_body) = finally {
                    self.buf.push_str(" finally {\n");
                    self.indent += 1;
                    for s in finally_body {
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
    }

    fn write_func_header(&mut self, lit: &FuncLiteral, name: Option<&str>) {
        if lit.is_arrow {
            // Arrows have no leading keyword. The header writes the
            // parameter list only; `=>` and the body follow in
            // write_func_body.
            self.buf.push('(');
            for (i, p) in lit.params.iter().enumerate() {
                if i > 0 {
                    self.buf.push_str(", ");
                }
                self.buf.push_str(p);
            }
            self.buf.push(')');
            return;
        }
        if lit.is_async {
            self.buf.push_str("async ");
        }
        self.buf.push_str("function");
        if lit.is_generator {
            self.buf.push('*');
        }
        if let Some(n) = name {
            self.buf.push(' ');
            self.buf.push_str(n);
        }
        self.buf.push('(');
        for (i, p) in lit.params.iter().enumerate() {
            if i > 0 {
                self.buf.push_str(", ");
            }
            self.buf.push_str(p);
        }
        self.buf.push(')');
    }

    fn write_func_body(&mut self, lit: &FuncLiteral) {
        let is_arrow = lit.is_arrow;
        match &lit.body {
            FuncBody::Stmts(stmts) => {
                if is_arrow {
                    self.buf.push_str(" => {\n");
                } else {
                    self.buf.push_str(" {\n");
                }
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
            FuncBody::Expression(e) => {
                self.buf.push_str(" => ");
                self.expr(e);
            }
            FuncBody::PassThrough(src) => {
                // Source-preserved body. Re-parse + codegen via
                // `oxc_codegen` so the body's text survives later
                // local IR transformations of the surrounding code.
                // Acceptable degradation: comments are dropped.
                if is_arrow {
                    self.buf.push_str(" => ");
                } else {
                    self.buf.push(' ');
                }
                let body_str = reparse_and_codegen_body(lit, src);
                self.buf.push_str(&body_str);
            }
        }
    }

    fn expr(&mut self, e: &Expr) {
        match e {
            Expr::Number(n) => {
                if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                    // -0.0 as i64 is 0, so check sign before casting.
                    if *n == 0.0 && n.is_sign_negative() {
                        self.buf.push_str("(-0)");
                    } else {
                        write!(self.buf, "{}", *n as i64).unwrap();
                    }
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
            Expr::Array { elements } => {
                self.buf.push('[');
                for (i, el) in elements.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.expr(el);
                }
                self.buf.push(']');
            }
            Expr::Object { props } => {
                self.buf.push('{');
                for (i, p) in props.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    match &p.key {
                        ObjectKey::Spread => {
                            // Spread property: `...expr`
                            self.buf.push_str("...");
                            self.expr(&p.value);
                            // No `: value` suffix — spread has no separate value.
                            continue;
                        }
                        ObjectKey::Static(s) => {
                            // Always emit a quoted string key — safe for
                            // identifiers, numerics, and arbitrary text.
                            self.buf.push('"');
                            for c in s.chars() {
                                match c {
                                    '\\' => self.buf.push_str("\\\\"),
                                    '"' => self.buf.push_str("\\\""),
                                    c => self.buf.push(c),
                                }
                            }
                            self.buf.push('"');
                        }
                        ObjectKey::Computed(e) => {
                            self.buf.push('[');
                            self.expr(e);
                            self.buf.push(']');
                        }
                    }
                    self.buf.push_str(": ");
                    self.expr(&p.value);
                }
                self.buf.push('}');
            }
            Expr::New { callee, args } => {
                self.buf.push_str("new ");
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
            Expr::Ternary {
                cond,
                then_branch,
                else_branch,
            } => {
                self.buf.push('(');
                self.expr(cond);
                self.buf.push_str(" ? ");
                self.expr(then_branch);
                self.buf.push_str(" : ");
                self.expr(else_branch);
                self.buf.push(')');
            }
            Expr::Update { op, target, prefix } => {
                let s = match op {
                    ir::tree::UpdateOp::Inc => "++",
                    ir::tree::UpdateOp::Dec => "--",
                };
                if *prefix {
                    self.buf.push_str(s);
                    self.expr(target);
                } else {
                    self.expr(target);
                    self.buf.push_str(s);
                }
            }
            Expr::Func(lit) => {
                // Function expressions: parenthesize so the caller can
                // splice them anywhere an expression is legal (statement
                // positions starting with `function` would otherwise be
                // ambiguous with declarations).
                if !lit.is_arrow {
                    self.buf.push('(');
                }
                self.write_func_header(lit, lit.name.as_deref());
                self.write_func_body(lit);
                if !lit.is_arrow {
                    self.buf.push(')');
                }
            }
            Expr::Spread(inner) => {
                self.buf.push_str("...");
                self.expr(inner);
            }
            Expr::Sequence { exprs } => {
                self.buf.push('(');
                for (i, e) in exprs.iter().enumerate() {
                    if i > 0 {
                        self.buf.push_str(", ");
                    }
                    self.expr(e);
                }
                self.buf.push(')');
            }
            Expr::Assign { target, value } => {
                self.buf.push('(');
                self.expr(target);
                self.buf.push_str(" = ");
                self.expr(value);
                self.buf.push(')');
            }
            Expr::CompoundAssign { op, target, value } => {
                self.buf.push('(');
                self.expr(target);
                write!(self.buf, " {}= ", binop_str(*op)).unwrap();
                self.expr(value);
                self.buf.push(')');
            }
            Expr::Template { quasis, exprs } => {
                self.buf.push('`');
                let mut quasis_iter = quasis.iter();
                let mut exprs_iter = exprs.iter();
                if let Some(first) = quasis_iter.next() {
                    self.buf.push_str(first);
                }
                loop {
                    let Some(expr) = exprs_iter.next() else { break };
                    self.buf.push_str("${");
                    self.expr(expr);
                    self.buf.push('}');
                    if let Some(q) = quasis_iter.next() {
                        self.buf.push_str(q);
                    }
                }
                self.buf.push('`');
            }
        }
    }
}
