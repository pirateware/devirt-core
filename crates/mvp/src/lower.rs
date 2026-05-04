//! OXC AST → tree-IR lowering.
//!
//! `parse_and_lower` owns the parser arena for its whole body, then lowers
//! the AST into our owned IR before the arena is dropped.
//!
//! T2 introduces per-function disposition: a function body that uses any
//! construct outside our supported subset becomes `PassThrough` —
//! preserved as an owned source slice and re-emitted at the final
//! emit step. Top-level non-function statements still error on
//! unsupported constructs (T2 only changes function-body handling).

use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use crate::error::Error;
use crate::ir::disposition::{Disposition, decide};
use crate::ir::tree::*;

pub fn parse_and_lower(source: &str) -> Result<Module, Error> {
    parse_and_lower_with_decider(source, decide)
}

/// Test-only entry point that injects a custom disposition decider so
/// the asymmetric debug cross-check in `lower_top_function` can be
/// exercised by hand-crafted "lying" walkers (T2 Stream D).
pub(crate) fn parse_and_lower_with_decider(
    source: &str,
    decider: fn(&js::FormalParameters, &js::FunctionBody) -> Disposition,
) -> Result<Module, Error> {
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
    let lowerer = Lowerer { source };
    lowerer.lower_program_with_decider(&ret.program, decider)
}

/// Carries the source string so PassThrough bodies can be sliced from
/// it. The source must outlive every owned slice we extract — our
/// PassThrough bodies clone substrings into owned `String`s at slice
/// time, so the lifetime is bounded by this struct.
struct Lowerer<'src> {
    source: &'src str,
}

impl Lowerer<'_> {
    fn lower_program_with_decider(
        &self,
        prog: &js::Program,
        decider: fn(&js::FormalParameters, &js::FunctionBody) -> Disposition,
    ) -> Result<Module, Error> {
        let mut items = Vec::with_capacity(prog.body.len());
        for stmt in &prog.body {
            items.push(self.lower_top_stmt(stmt, decider)?);
        }
        Ok(Module { items })
    }

    fn lower_top_stmt(
        &self,
        stmt: &js::Statement,
        decider: fn(&js::FormalParameters, &js::FunctionBody) -> Disposition,
    ) -> Result<Item, Error> {
        match stmt {
            js::Statement::FunctionDeclaration(f) => {
                self.lower_top_function_with_decider(f, decider)
            }
            _ => Ok(Item::Stmt(lower_stmt(stmt)?)),
        }
    }

    /// Same as `lower_top_function`, but the disposition decider is
    /// injected. Production always passes `disposition::decide`; tests
    /// pass a hand-crafted "lying" walker to exercise the asymmetric
    /// debug cross-check (T2 Stream D).
    pub(crate) fn lower_top_function_with_decider(
        &self,
        f: &js::Function,
        decider: fn(&js::FormalParameters, &js::FunctionBody) -> Disposition,
    ) -> Result<Item, Error> {
        let name =
            f.id.as_ref()
                .map(|i| i.name.to_string())
                .ok_or(Error::Unsupported("anonymous function declaration"))?;

        // Empty bodies are vacuously lowerable; `body` is None for
        // ambient declarations we don't accept anyway. Reject with
        // a clear error so we don't silently produce an empty
        // PassThrough.
        let body = f
            .body
            .as_ref()
            .ok_or(Error::Unsupported("function declaration without a body"))?;

        let disposition = decider(&f.params, body);

        if disposition == Disposition::PassThrough {
            return Ok(Item::PassThroughFunction(self.passthrough_literal(
                Some(name),
                f,
                body,
            )?));
        }

        // Lowered: same path as before T2.
        let result = self.lower_function_lowered(&name, f, body);

        // === T2 Stream D: asymmetric debug cross-check ===
        //
        // The disposition walker said `Lowered`. The lowering MUST NOT
        // come back with `Error::Unsupported` — that would mean the
        // walker is too permissive (it accepted a construct the
        // lowering can't actually represent), and we'd silently fall
        // through to a hard error in production. In debug builds we
        // turn that into a loud panic naming the construct so the
        // walker bug is impossible to miss.
        //
        // The OPPOSITE direction (walker too conservative — it said
        // PassThrough but the lowering would have succeeded) is
        // intentionally NOT caught here. That direction is recoverable:
        // a function we could have optimized stays preserved, no
        // correctness issue. Catching it would require speculatively
        // running the lowering on every PassThrough body, which is
        // expensive and produces false positives (e.g. inner-function
        // capture rules that the walker can't see). See T2_PLAN.md
        // Stream D and the design notes in `disposition.rs`.
        //
        // Release builds compile this block out entirely (the
        // `cfg!(debug_assertions)` is a const-folded `false`, the
        // branch is dead code, and the `if` collapses), so there is
        // no runtime cost when assertions are off.
        //
        // TODO(stream-b): nested-function lowering sites (in
        // `lower_function_literal` once Stream B lands) also need this
        // cross-check. Apply the same pattern there.
        if cfg!(debug_assertions)
            && let Err(Error::Unsupported(what)) = &result
        {
            panic!(
                "T2 cross-check: disposition walker said `Lowered` for \
                 function `{name}` but lowering returned \
                 Error::Unsupported({what:?}). The walker in \
                 `ir::disposition` is too permissive — it accepted a \
                 construct the lowering cannot represent. Either tighten \
                 the walker to reject this construct, or extend the \
                 lowering to handle it."
            );
        }

        result
    }

    fn lower_function_lowered(
        &self,
        name: &str,
        f: &js::Function,
        body: &js::FunctionBody,
    ) -> Result<Item, Error> {
        let mut params = Vec::with_capacity(f.params.items.len());
        for p in &f.params.items {
            match &p.pattern.kind {
                js::BindingPatternKind::BindingIdentifier(id) => {
                    params.push(id.name.to_string());
                }
                _ => {
                    // Disposition should have routed this to PassThrough.
                    // Reaching here means walker-too-permissive.
                    return Err(Error::Unsupported("destructuring or default parameter"));
                }
            }
        }

        let stmts = body
            .statements
            .iter()
            .map(lower_stmt)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Item::Function(IrFunction {
            name: name.to_string(),
            params,
            body: stmts,
        }))
    }

    /// Build a `FuncLiteral` whose body is a PassThrough source slice.
    /// Captures parameter names and the async/generator flags from the
    /// OXC `Function`. The slice covers the body's contents between
    /// `{` and `}`, exclusive.
    fn passthrough_literal(
        &self,
        name: Option<String>,
        f: &js::Function,
        body: &js::FunctionBody,
    ) -> Result<FuncLiteral, Error> {
        // Param names: use the binding identifier when present;
        // otherwise emit a synthetic placeholder. The body source is
        // verbatim, so destructuring patterns inside the *body* still
        // work; only the param-list textual rendering uses these
        // names. We emit them faithfully when we can.
        let mut param_names = Vec::with_capacity(f.params.items.len());
        for (i, p) in f.params.items.iter().enumerate() {
            match &p.pattern.kind {
                js::BindingPatternKind::BindingIdentifier(id) => {
                    param_names.push(id.name.to_string());
                }
                _ => {
                    // Best-effort textual round-trip: re-extract the
                    // parameter source slice. Falls back to a synthetic
                    // name if slicing fails.
                    let span = p.pattern.span();
                    param_names.push(
                        self.slice_span(span)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("__p{}", i)),
                    );
                }
            }
        }

        let body_span = body.span();
        let inner = self
            .slice_body_inner(body_span)
            .ok_or(Error::Malformed(
                "PassThrough body span out of range".into(),
            ))?
            .to_string();

        Ok(FuncLiteral {
            name,
            params: param_names,
            is_async: f.r#async,
            is_generator: f.generator,
            is_arrow: false,
            body: FuncBody::PassThrough(inner),
            func_id: std::cell::Cell::new(None),
        })
    }

    /// Slice the source between a span's `{` and `}` (exclusive). The
    /// body-span includes the braces; we strip them so the caller can
    /// re-wrap.
    fn slice_body_inner(&self, span: oxc_span::Span) -> Option<&str> {
        let s = span.start as usize;
        let e = span.end as usize;
        let bytes = self.source.as_bytes();
        if e > self.source.len() || s >= e {
            return None;
        }
        // Skip leading `{`, trailing `}` if present.
        let inner_start = if bytes.get(s) == Some(&b'{') {
            s + 1
        } else {
            s
        };
        let inner_end = if bytes.get(e - 1) == Some(&b'}') {
            e - 1
        } else {
            e
        };
        if inner_end < inner_start {
            return None;
        }
        Some(&self.source[inner_start..inner_end])
    }

    fn slice_span(&self, span: oxc_span::Span) -> Option<&str> {
        let s = span.start as usize;
        let e = span.end as usize;
        if e > self.source.len() || s > e {
            return None;
        }
        Some(&self.source[s..e])
    }
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
