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
//!
//! Stream B wires nested function declarations, function expressions,
//! and arrow functions through the same disposition decision —
//! lowerable inner functions become `Stmt::FuncDecl(FuncLiteral)` /
//! `Expr::Func(FuncLiteral)`; un-lowerable ones become FuncLiteral
//! with `FuncBody::PassThrough(slice)`.

pub mod disposition;
pub mod lower_ssa;
pub mod lower_tree;

use oxc_allocator::Allocator;
use oxc_ast::ast as js;
use oxc_parser::Parser;
use oxc_span::{GetSpan, SourceType};

use ir::error::Error;
use crate::disposition::{Disposition, decide, decide_arrow};
use ir::tree::*;

/// Convert a source-level compound assignment operator into the
/// underlying binary operator.
fn compound_to_binop(op: js::AssignmentOperator) -> Result<BinOp, Error> {
    use js::AssignmentOperator as A;
    Ok(match op {
        A::Addition => BinOp::Add,
        A::Subtraction => BinOp::Sub,
        A::Multiplication => BinOp::Mul,
        A::Division => BinOp::Div,
        A::Remainder => BinOp::Mod,
        A::ShiftLeft => BinOp::Shl,
        A::ShiftRight => BinOp::Shr,
        A::ShiftRightZeroFill => BinOp::UShr,
        A::BitwiseAnd => BinOp::BitAnd,
        A::BitwiseOR => BinOp::BitOr,
        A::BitwiseXOR => BinOp::BitXor,
        // Exponential and logical/nullish-assign are rejected at the
        // walker; reaching here is a walker bug.
        _ => return Err(Error::Unsupported("compound assignment operator")),
    })
}

pub fn parse_and_lower(source: &str) -> Result<Module, Error> {
    parse_and_lower_with_decider(source, decide)
}

/// Test-only entry point that injects a custom disposition decider so
/// the asymmetric debug cross-check in `lower_top_function` can be
/// exercised by hand-crafted "lying" walkers (T2 Stream D).
pub fn parse_and_lower_with_decider(
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
    let lowerer = Lowerer {
        source,
        temp_counter: std::cell::Cell::new(0),
    };
    lowerer.lower_program_with_decider(&ret.program, decider)
}

/// Carries the source string so PassThrough bodies can be sliced from
/// it. The source must outlive every owned slice we extract — our
/// PassThrough bodies clone substrings into owned `String`s at slice
/// time, so the lifetime is bounded by this struct.
struct Lowerer<'src> {
    source: &'src str,
    /// Counter for synthesized temporaries (compound-assignment
    /// receivers/keys). The names use a `__t<n>__` shape that's
    /// reserved for this pass; collisions with user code are
    /// theoretically possible but vanishingly unlikely.
    temp_counter: std::cell::Cell<usize>,
}

impl<'src> Lowerer<'src> {
    fn fresh_temp(&self) -> String {
        let n = self.temp_counter.get();
        self.temp_counter.set(n + 1);
        format!("__t{n}__")
    }
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
            other => {
                // Top-level non-function statements have no SSA
                // representation (the SSA model is function-centric).
                // Preserve them as verbatim source slices so the
                // round-trip survives any construct OXC can parse —
                // including IIFEs, top-level expressions, classes,
                // `for`, etc. This is the top-level analogue of the
                // per-function PassThrough fallback.
                let span = other.span();
                let src = self
                    .slice_span(span)
                    .ok_or(Error::Malformed(
                        "top-level stmt source span out of range".into(),
                    ))?
                    .to_string();
                Ok(Item::PassThroughStmt(src))
            }
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
            .map(|s| self.lower_stmt(s))
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
            body: FuncBody::PassThrough(inner.clone()),
            func_id: std::cell::Cell::new(None),
            passthrough_source: Some(inner),
            force_passthrough: std::cell::Cell::new(false),
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

    // ------------------------------------------------------------------
    // Function-shaped helpers (Stream B)
    // ------------------------------------------------------------------

    /// Build a `FuncLiteral` for a nested function declaration / function
    /// expression. Routes through `disposition::decide`: lowerable bodies
    /// produce `FuncBody::Stmts`; un-lowerable ones produce
    /// `FuncBody::PassThrough(slice)`.
    fn lower_function_literal(
        &self,
        f: &js::Function,
        is_decl: bool,
    ) -> Result<FuncLiteral, Error> {
        let name = f.id.as_ref().map(|i| i.name.to_string());
        let body = f
            .body
            .as_ref()
            .ok_or(Error::Unsupported("function without a body"))?;

        if is_decl && name.is_none() {
            return Err(Error::Unsupported("anonymous nested function declaration"));
        }

        let disposition = decide(&f.params, body);
        if disposition == Disposition::PassThrough {
            return self.passthrough_literal(name, f, body);
        }

        let mut params = Vec::with_capacity(f.params.items.len());
        for p in &f.params.items {
            match &p.pattern.kind {
                js::BindingPatternKind::BindingIdentifier(id) => {
                    params.push(id.name.to_string());
                }
                _ => {
                    return Err(Error::Unsupported("destructuring or default parameter"));
                }
            }
        }

        let stmts = body
            .statements
            .iter()
            .map(|s| self.lower_stmt(s))
            .collect::<Result<Vec<_>, _>>()?;

        // Stash the body's source slice so pass 1 of SSA lowering can
        // re-classify this literal to PassThrough cheaply if the
        // forward-ref / reassigned-capture rule fires.
        let passthrough_source = self.slice_body_inner(body.span()).map(|s| s.to_string());

        Ok(FuncLiteral {
            name,
            params,
            is_async: f.r#async,
            is_generator: f.generator,
            is_arrow: false,
            body: FuncBody::Stmts(stmts),
            func_id: std::cell::Cell::new(None),
            passthrough_source,
            force_passthrough: std::cell::Cell::new(false),
        })
    }

    /// Build a `FuncLiteral` for an arrow function. Block-body arrows
    /// produce `FuncBody::Stmts`; expression-body arrows (`x => x+1`)
    /// produce `FuncBody::Expression`. Un-lowerable arrows go through
    /// `FuncBody::PassThrough`.
    fn lower_arrow_literal(&self, a: &js::ArrowFunctionExpression) -> Result<FuncLiteral, Error> {
        let disposition = decide_arrow(a);
        if disposition == Disposition::PassThrough {
            return self.passthrough_arrow(a);
        }

        let mut params = Vec::with_capacity(a.params.items.len());
        for p in &a.params.items {
            match &p.pattern.kind {
                js::BindingPatternKind::BindingIdentifier(id) => {
                    params.push(id.name.to_string());
                }
                _ => {
                    return Err(Error::Unsupported("destructuring or default parameter"));
                }
            }
        }

        // Arrow body in OXC is always wrapped in a `FunctionBody`. When
        // `a.expression` is true, body.statements has exactly one
        // ExpressionStatement (synthesized by the parser to host the
        // expression body); otherwise it's a regular block.
        let (body, passthrough_source) = if a.expression {
            let stmt = a.body.statements.first().ok_or_else(|| {
                Error::Malformed("expression-body arrow missing statement".into())
            })?;
            let expr_inner = match stmt {
                js::Statement::ExpressionStatement(es) => &es.expression,
                _ => {
                    return Err(Error::Malformed(
                        "expression-body arrow's statement is not an ExpressionStatement".into(),
                    ));
                }
            };
            // Stash a `return <expr>;` form so pass-1 reclassification
            // produces a body that re-emits as `=> { return <expr>; }`.
            let pt = self
                .slice_span(expr_inner.span())
                .map(|s| format!("return {};", s));
            (
                FuncBody::Expression(Box::new(self.lower_expr(expr_inner)?)),
                pt,
            )
        } else {
            let stmts = a
                .body
                .statements
                .iter()
                .map(|s| self.lower_stmt(s))
                .collect::<Result<Vec<_>, _>>()?;
            let pt = self.slice_body_inner(a.body.span()).map(|s| s.to_string());
            (FuncBody::Stmts(stmts), pt)
        };

        Ok(FuncLiteral {
            name: None,
            params,
            is_async: a.r#async,
            is_generator: false,
            is_arrow: true,
            body,
            func_id: std::cell::Cell::new(None),
            passthrough_source,
            force_passthrough: std::cell::Cell::new(false),
        })
    }

    /// PassThrough fallback for an un-lowerable arrow. We always emit a
    /// block-body PassThrough — for an expression-body arrow we wrap
    /// the expression text in `return ...;` so the re-emit (which uses
    /// `=> { <slice> }`) parses as a block-body arrow with the same
    /// observable behavior.
    fn passthrough_arrow(&self, a: &js::ArrowFunctionExpression) -> Result<FuncLiteral, Error> {
        let mut param_names = Vec::with_capacity(a.params.items.len());
        for (i, p) in a.params.items.iter().enumerate() {
            match &p.pattern.kind {
                js::BindingPatternKind::BindingIdentifier(id) => {
                    param_names.push(id.name.to_string());
                }
                _ => {
                    let span = p.pattern.span();
                    param_names.push(
                        self.slice_span(span)
                            .map(|s| s.to_string())
                            .unwrap_or_else(|| format!("__p{}", i)),
                    );
                }
            }
        }

        let inner = if a.expression {
            // Slice the expression text and wrap it in `return ...;`
            // so `=> { <slice> }` produces a semantically-equivalent
            // block-body arrow at re-emit time.
            let stmt = a.body.statements.first().ok_or_else(|| {
                Error::Malformed("expression-body arrow missing statement".into())
            })?;
            let expr = match stmt {
                js::Statement::ExpressionStatement(es) => &es.expression,
                _ => {
                    return Err(Error::Malformed(
                        "expression-body arrow's statement is not an ExpressionStatement".into(),
                    ));
                }
            };
            let span = expr.span();
            let text = self.slice_span(span).ok_or_else(|| {
                Error::Malformed("arrow expression-body slice out of range".into())
            })?;
            format!("return {};", text)
        } else {
            let body_span = a.body.span();
            self.slice_body_inner(body_span)
                .ok_or(Error::Malformed(
                    "PassThrough arrow body span out of range".into(),
                ))?
                .to_string()
        };

        Ok(FuncLiteral {
            name: None,
            params: param_names,
            is_async: a.r#async,
            is_generator: false,
            is_arrow: true,
            body: FuncBody::PassThrough(inner.clone()),
            func_id: std::cell::Cell::new(None),
            passthrough_source: Some(inner),
            force_passthrough: std::cell::Cell::new(false),
        })
    }

    // ------------------------------------------------------------------
    // Statement / expression lowering
    // ------------------------------------------------------------------

    fn lower_stmt(&self, stmt: &js::Statement) -> Result<Stmt, Error> {
        match stmt {
            js::Statement::VariableDeclaration(decl) => self.lower_var_decl(decl),
            js::Statement::ExpressionStatement(es) => self.lower_expr_stmt(&es.expression),
            js::Statement::ReturnStatement(r) => {
                let v = match &r.argument {
                    Some(e) => Some(self.lower_expr(e)?),
                    None => None,
                };
                Ok(Stmt::Return(v))
            }
            js::Statement::IfStatement(s) => Ok(Stmt::If {
                cond: self.lower_expr(&s.test)?,
                then_body: self.stmts_from(&s.consequent)?,
                else_body: match &s.alternate {
                    Some(alt) => Some(self.stmts_from(alt)?),
                    None => None,
                },
            }),
            js::Statement::WhileStatement(s) => Ok(Stmt::While {
                cond: self.lower_expr(&s.test)?,
                body: self.stmts_from(&s.body)?,
            }),
            js::Statement::BlockStatement(b) => {
                let stmts = b
                    .body
                    .iter()
                    .map(|s| self.lower_stmt(s))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Stmt::Block(stmts))
            }
            js::Statement::FunctionDeclaration(f) => {
                Ok(Stmt::FuncDecl(self.lower_function_literal(f, true)?))
            }
            js::Statement::ForStatement(f) => self.lower_for_stmt(f),
            js::Statement::ContinueStatement(c) => {
                if c.label.is_some() {
                    return Err(Error::Unsupported("labeled continue"));
                }
                Ok(Stmt::Continue)
            }
            _ => Err(Error::Unsupported(stmt_kind(stmt))),
        }
    }

    fn lower_for_stmt(&self, f: &js::ForStatement) -> Result<Stmt, Error> {
        let init = match &f.init {
            None => None,
            Some(js::ForStatementInit::VariableDeclaration(decl)) => {
                Some(Box::new(self.lower_var_decl(decl)?))
            }
            Some(other) => {
                let e = other
                    .as_expression()
                    .ok_or(Error::Unsupported("non-expression for-init"))?;
                Some(Box::new(Stmt::ExprStmt(self.lower_expr(e)?)))
            }
        };
        let cond = match &f.test {
            Some(e) => Some(self.lower_expr(e)?),
            None => None,
        };
        let step = match &f.update {
            Some(e) => Some(self.lower_expr(e)?),
            None => None,
        };
        let body = self.stmts_from(&f.body)?;
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    fn lower_var_decl(&self, decl: &js::VariableDeclaration) -> Result<Stmt, Error> {
        if decl.declarations.len() != 1 {
            return Err(Error::Unsupported("multi-declarator variable declaration"));
        }
        let d = &decl.declarations[0];
        let name = match &d.id.kind {
            js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
            _ => return Err(Error::Unsupported("destructuring binding")),
        };
        let init = match &d.init {
            Some(e) => Some(self.lower_expr(e)?),
            None => None,
        };
        match decl.kind {
            js::VariableDeclarationKind::Let | js::VariableDeclarationKind::Var => {
                Ok(Stmt::Let { name, init })
            }
            js::VariableDeclarationKind::Const => {
                let init =
                    init.ok_or_else(|| Error::Malformed("const without initializer".into()))?;
                Ok(Stmt::Const { name, init })
            }
            _ => Err(Error::Unsupported("`using` declaration")),
        }
    }

    fn lower_expr_stmt(&self, expr: &js::Expression) -> Result<Stmt, Error> {
        if let js::Expression::AssignmentExpression(a) = expr {
            if matches!(a.operator, js::AssignmentOperator::Assign) {
                let target = self.lower_assign_target(&a.left)?;
                let value = self.lower_expr(&a.right)?;
                return Ok(Stmt::Assign { target, value });
            }
            // Compound assignment: x op= rhs
            // For an identifier target, the equivalent is `x = x op rhs`.
            // For property targets, the receiver and key must each be
            // evaluated exactly once — the read and write share them.
            // We desugar to a Block whose let-bindings name the receiver
            // (and key, for computed access) in temporaries before
            // performing the read+write.
            let op = compound_to_binop(a.operator)?;
            let rhs = self.lower_expr(&a.right)?;
            return self.lower_compound_assign(&a.left, op, rhs);
        }
        Ok(Stmt::ExprStmt(self.lower_expr(expr)?))
    }

    fn lower_compound_assign(
        &self,
        target: &js::AssignmentTarget,
        op: BinOp,
        rhs: Expr,
    ) -> Result<Stmt, Error> {
        match target {
            js::AssignmentTarget::AssignmentTargetIdentifier(id) => {
                let name = id.name.to_string();
                Ok(Stmt::Assign {
                    target: Expr::Ident(name.clone()),
                    value: Expr::Binary {
                        op,
                        lhs: Box::new(Expr::Ident(name)),
                        rhs: Box::new(rhs),
                    },
                })
            }
            js::AssignmentTarget::StaticMemberExpression(m) => {
                let obj = self.lower_expr(&m.object)?;
                let prop = m.property.name.to_string();
                let t = self.fresh_temp();
                let read = Expr::Member {
                    obj: Box::new(Expr::Ident(t.clone())),
                    prop: prop.clone(),
                };
                Ok(Stmt::Block(vec![
                    Stmt::Let {
                        name: t.clone(),
                        init: Some(obj),
                    },
                    Stmt::Assign {
                        target: Expr::Member {
                            obj: Box::new(Expr::Ident(t)),
                            prop,
                        },
                        value: Expr::Binary {
                            op,
                            lhs: Box::new(read),
                            rhs: Box::new(rhs),
                        },
                    },
                ]))
            }
            js::AssignmentTarget::ComputedMemberExpression(m) => {
                let obj = self.lower_expr(&m.object)?;
                let key = self.lower_expr(&m.expression)?;
                let to = self.fresh_temp();
                let tk = self.fresh_temp();
                let read = Expr::Index {
                    obj: Box::new(Expr::Ident(to.clone())),
                    index: Box::new(Expr::Ident(tk.clone())),
                };
                Ok(Stmt::Block(vec![
                    Stmt::Let {
                        name: to.clone(),
                        init: Some(obj),
                    },
                    Stmt::Let {
                        name: tk.clone(),
                        init: Some(key),
                    },
                    Stmt::Assign {
                        target: Expr::Index {
                            obj: Box::new(Expr::Ident(to)),
                            index: Box::new(Expr::Ident(tk)),
                        },
                        value: Expr::Binary {
                            op,
                            lhs: Box::new(read),
                            rhs: Box::new(rhs),
                        },
                    },
                ]))
            }
            _ => Err(Error::Unsupported("complex compound-assignment target")),
        }
    }

    fn stmts_from(&self, stmt: &js::Statement) -> Result<Vec<Stmt>, Error> {
        if let js::Statement::BlockStatement(b) = stmt {
            b.body.iter().map(|s| self.lower_stmt(s)).collect()
        } else {
            Ok(vec![self.lower_stmt(stmt)?])
        }
    }

    fn lower_update_target(&self, t: &js::SimpleAssignmentTarget) -> Result<Expr, Error> {
        match t {
            js::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) => {
                Ok(Expr::Ident(id.name.to_string()))
            }
            js::SimpleAssignmentTarget::StaticMemberExpression(m) => Ok(Expr::Member {
                obj: Box::new(self.lower_expr(&m.object)?),
                prop: m.property.name.to_string(),
            }),
            js::SimpleAssignmentTarget::ComputedMemberExpression(m) => Ok(Expr::Index {
                obj: Box::new(self.lower_expr(&m.object)?),
                index: Box::new(self.lower_expr(&m.expression)?),
            }),
            _ => Err(Error::Unsupported("complex update target")),
        }
    }

    fn lower_assign_target(&self, t: &js::AssignmentTarget) -> Result<Expr, Error> {
        match t {
            js::AssignmentTarget::AssignmentTargetIdentifier(id) => {
                Ok(Expr::Ident(id.name.to_string()))
            }
            js::AssignmentTarget::StaticMemberExpression(m) => Ok(Expr::Member {
                obj: Box::new(self.lower_expr(&m.object)?),
                prop: m.property.name.to_string(),
            }),
            js::AssignmentTarget::ComputedMemberExpression(m) => Ok(Expr::Index {
                obj: Box::new(self.lower_expr(&m.object)?),
                index: Box::new(self.lower_expr(&m.expression)?),
            }),
            _ => Err(Error::Unsupported("complex assignment target")),
        }
    }

    fn lower_expr(&self, expr: &js::Expression) -> Result<Expr, Error> {
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
            js::Expression::ParenthesizedExpression(p) => self.lower_expr(&p.expression),

            js::Expression::BinaryExpression(b) => Ok(Expr::Binary {
                op: lower_binop(b.operator)?,
                lhs: Box::new(self.lower_expr(&b.left)?),
                rhs: Box::new(self.lower_expr(&b.right)?),
            }),
            js::Expression::LogicalExpression(l) => {
                let op = match l.operator {
                    js::LogicalOperator::And => BinOp::LogAnd,
                    js::LogicalOperator::Or => BinOp::LogOr,
                    _ => return Err(Error::Unsupported("nullish coalescing (??)")),
                };
                Ok(Expr::Binary {
                    op,
                    lhs: Box::new(self.lower_expr(&l.left)?),
                    rhs: Box::new(self.lower_expr(&l.right)?),
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
                    operand: Box::new(self.lower_expr(&u.argument)?),
                })
            }

            js::Expression::CallExpression(c) => {
                let callee = self.lower_expr(&c.callee)?;
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
                            args.push(self.lower_expr(e)?);
                        }
                    }
                }
                Ok(Expr::Call {
                    callee: Box::new(callee),
                    args,
                })
            }

            js::Expression::StaticMemberExpression(m) => Ok(Expr::Member {
                obj: Box::new(self.lower_expr(&m.object)?),
                prop: m.property.name.to_string(),
            }),
            js::Expression::ComputedMemberExpression(m) => Ok(Expr::Index {
                obj: Box::new(self.lower_expr(&m.object)?),
                index: Box::new(self.lower_expr(&m.expression)?),
            }),

            js::Expression::FunctionExpression(f) => {
                Ok(Expr::Func(Box::new(self.lower_function_literal(f, false)?)))
            }
            js::Expression::ArrowFunctionExpression(a) => {
                Ok(Expr::Func(Box::new(self.lower_arrow_literal(a)?)))
            }

            js::Expression::NewExpression(n) => {
                let callee = self.lower_expr(&n.callee)?;
                let mut args = Vec::with_capacity(n.arguments.len());
                for a in &n.arguments {
                    match a {
                        js::Argument::SpreadElement(_) => {
                            return Err(Error::Unsupported("spread in new args"));
                        }
                        other => {
                            let e = other
                                .as_expression()
                                .ok_or(Error::Unsupported("non-expression new arg"))?;
                            args.push(self.lower_expr(e)?);
                        }
                    }
                }
                Ok(Expr::New {
                    callee: Box::new(callee),
                    args,
                })
            }

            js::Expression::ObjectExpression(obj) => {
                let mut props = Vec::with_capacity(obj.properties.len());
                for prop in &obj.properties {
                    match prop {
                        js::ObjectPropertyKind::ObjectProperty(p) => {
                            if p.method || p.kind != js::PropertyKind::Init {
                                return Err(Error::Unsupported(
                                    "object method/getter/setter",
                                ));
                            }
                            let value = self.lower_expr(&p.value)?;
                            let key = if p.computed {
                                let e = p.key.as_expression().ok_or(Error::Unsupported(
                                    "non-expression computed key",
                                ))?;
                                ObjectKey::Computed(Box::new(self.lower_expr(e)?))
                            } else {
                                match &p.key {
                                    js::PropertyKey::StaticIdentifier(id) => {
                                        ObjectKey::Static(id.name.to_string())
                                    }
                                    js::PropertyKey::StringLiteral(s) => {
                                        ObjectKey::Static(s.value.to_string())
                                    }
                                    js::PropertyKey::NumericLiteral(n) => {
                                        ObjectKey::Static(n.value.to_string())
                                    }
                                    _ => {
                                        return Err(Error::Unsupported("object key kind"));
                                    }
                                }
                            };
                            props.push(ObjectProp { key, value });
                        }
                        js::ObjectPropertyKind::SpreadProperty(_) => {
                            return Err(Error::Unsupported("spread in object literal"));
                        }
                    }
                }
                Ok(Expr::Object { props })
            }

            js::Expression::ConditionalExpression(c) => Ok(Expr::Ternary {
                cond: Box::new(self.lower_expr(&c.test)?),
                then_branch: Box::new(self.lower_expr(&c.consequent)?),
                else_branch: Box::new(self.lower_expr(&c.alternate)?),
            }),

            js::Expression::UpdateExpression(u) => {
                let op = match u.operator {
                    js::UpdateOperator::Increment => UpdateOp::Inc,
                    js::UpdateOperator::Decrement => UpdateOp::Dec,
                };
                let target = self.lower_update_target(&u.argument)?;
                Ok(Expr::Update {
                    op,
                    target: Box::new(target),
                    prefix: u.prefix,
                })
            }

            js::Expression::ArrayExpression(arr) => {
                let mut elements = Vec::with_capacity(arr.elements.len());
                for el in &arr.elements {
                    match el {
                        js::ArrayExpressionElement::SpreadElement(_) => {
                            return Err(Error::Unsupported("spread in array literal"));
                        }
                        js::ArrayExpressionElement::Elision(_) => {
                            return Err(Error::Unsupported("elision in array literal"));
                        }
                        other => {
                            let e = other
                                .as_expression()
                                .ok_or(Error::Unsupported("non-expression array element"))?;
                            elements.push(self.lower_expr(e)?);
                        }
                    }
                }
                Ok(Expr::Array { elements })
            }

            _ => Err(Error::Unsupported("expression kind")),
        }
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
