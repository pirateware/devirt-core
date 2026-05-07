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

use std::collections::BTreeSet;

use ir::blocker::BlockerKind;
use ir::error::Error;
use crate::disposition::{Disposition, decide, decide_arrow};
use ir::tree::*;

/// Build `if (name === undefined) name = default_expr;` — used by the
/// param-list desugaring (issue #29) for default values.
fn default_undefined_assign(name: &str, default_expr: Expr) -> Stmt {
    Stmt::If {
        cond: Expr::Binary {
            op: BinOp::StrictEq,
            lhs: Box::new(Expr::Ident(name.to_string())),
            rhs: Box::new(Expr::Undefined),
        },
        then_body: vec![Stmt::Assign {
            target: Expr::Ident(name.to_string()),
            value: default_expr,
        }],
        else_body: None,
    }
}

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
        A::Exponential => BinOp::Pow,
        // Logical/nullish-assign (`||=`, `&&=`, `??=`) are rejected at
        // the walker; reaching here is a walker bug.
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

/// Reserved name for the synthesized top-level wrapper function. The
/// emit layer recognizes this name and renders the function as an IIFE
/// (`(function __top_init__(){ ... })();`) instead of a plain decl.
/// Issue #32: wrapping non-function top-level statements so the
/// optimizer sees inside them (without it, top-level IIFEs and the
/// expressions inside them are invisible to general passes like
/// ConstructorFnInline).
pub const TOP_INIT_NAME: &str = "__top_init__";

impl Lowerer<'_> {
    fn lower_program_with_decider(
        &self,
        prog: &js::Program,
        decider: fn(&js::FormalParameters, &js::FunctionBody) -> Disposition,
    ) -> Result<Module, Error> {
        // Decide whether to synthesize `__top_init__` (issue #32).
        //
        // We wrap when there's at least one non-function top-level
        // statement we know how to lower into the IR, AND there are no
        // top-level `let` / `const` declarations (whose TDZ scoping
        // would change visibility once they become function-locals).
        // Otherwise fall back to the historical per-stmt routing where
        // non-function tops become `Item::PassThroughStmt`.
        //
        // `var`-decls at top level are split: their *declarations* are
        // hoisted out as `var x;` PassThroughStmt entries before the
        // wrapper (so other top-level functions can still see the
        // binding via `globalThis`), and the *initializers* become bare
        // assignments inside the wrapper (which the SSA layer routes
        // through `StoreProp(GlobalThis, name, val)` — exactly the
        // sloppy-mode `var x = ...` semantics we want at top level).
        //
        // Directive prologue (`"use strict";` etc.) is preserved as
        // PassThroughStmt at file top, before everything else.
        let mut items: Vec<Item> = Vec::with_capacity(prog.body.len() + prog.directives.len());

        // === 1. Pull leading directive prologue ===
        // OXC stores directives in `prog.directives`, separate from
        // `prog.body`. Preserve each as a `PassThroughStmt` at file
        // top so re-emit produces them verbatim. The `body` may also
        // start with `ExpressionStatement(StringLiteral)` entries that
        // were demoted to non-directive statements (parser lookahead
        // already classified them); we still treat any leading
        // string-literal-expression-statement as a directive shape so
        // the round-trip lands them at the top in either case.
        for d in &prog.directives {
            let span = d.span;
            let src = self
                .slice_span(span)
                .ok_or(Error::Malformed(
                    "directive prologue source span out of range".into(),
                ))?
                .to_string();
            items.push(Item::PassThroughStmt(src));
        }
        let mut idx = 0usize;
        while idx < prog.body.len() && is_directive_prologue(&prog.body[idx]) {
            let span = prog.body[idx].span();
            let src = self
                .slice_span(span)
                .ok_or(Error::Malformed(
                    "directive prologue source span out of range".into(),
                ))?
                .to_string();
            items.push(Item::PassThroughStmt(src));
            idx += 1;
        }

        // === 1b. Recognize a synthetic-wrapper IIFE re-emit and
        // flatten it before applying eligibility / wrapping. The emit
        // layer renders `__top_init__` as `(function () { ... })();`,
        // so when re-parsing our own output we'd otherwise wrap a body
        // that's just one IIFE — losing the structural symmetry the
        // round-trip check enforces.
        //
        // Collect a borrowed slice of "wrap candidates" (`Statement`
        // nodes from the OXC arena) using owned-Vec-of-borrows; if a
        // candidate matches the synthetic IIFE shape, splice in its
        // function body's statements instead. This only fires for the
        // exact emit shape (anonymous, no params, no args, not
        // async/generator), so user code that happens to write the
        // same pattern is also flattened — which is fine, the
        // observable semantics are identical.
        // Each candidate is `(stmt, from_synthetic_iife)`. Statements
        // that came from inside a synthetic IIFE were already
        // function-local in the original source, so their `let`/`const`
        // bindings carry no top-level visibility implications and must
        // not block the wrap path. Top-level `let`/`const` (the other
        // case) genuinely changes scope semantics if wrapped, so it
        // remains a hard wrap-blocker.
        let candidates: Vec<(&js::Statement, bool)> = {
            let mut out: Vec<(&js::Statement, bool)> = Vec::with_capacity(prog.body.len() - idx);
            for stmt in prog.body.iter().skip(idx) {
                if let Some(inner) = synthetic_top_init_iife_body(stmt) {
                    for s in inner {
                        out.push((s, true));
                    }
                } else {
                    out.push((stmt, false));
                }
            }
            out
        };

        // === 2. Decide wrap eligibility ===
        //
        // Wrap-eligible candidates split into three groups:
        //  - `FunctionDeclaration`: stays as a top-level item
        //    (function decls hoist; not part of `__top_init__`).
        //  - `VariableDeclaration` (var only): split into a global
        //    `var x;` hoist + bare assignment inside `__top_init__`.
        //    `let`/`const` block wrapping (TDZ scoping changes).
        //  - Anything else that's "wrap-safe" (no fallthrough switch,
        //    no try/forIn/class/etc.) is lowered as a normal statement
        //    inside the wrapper body.
        let mut has_top_init_body = false;
        let mut has_let_or_const = false;
        let mut all_wrappable = true;
        for (stmt, from_iife) in candidates.iter().copied() {
            match stmt {
                js::Statement::FunctionDeclaration(_) => {}
                js::Statement::VariableDeclaration(d) => {
                    has_top_init_body = true;
                    match d.kind {
                        js::VariableDeclarationKind::Let
                        | js::VariableDeclarationKind::Const => {
                            // Only the genuinely top-level `let`/`const`
                            // gates wrapping; ones spliced out of a
                            // synthetic IIFE were already function-
                            // scoped and stay function-scoped.
                            if !from_iife {
                                has_let_or_const = true;
                            }
                        }
                        _ => {}
                    }
                    if !stmt_wrap_safe(stmt) {
                        all_wrappable = false;
                    }
                }
                _ => {
                    has_top_init_body = true;
                    if !stmt_wrap_safe(stmt) {
                        all_wrappable = false;
                    }
                }
            }
        }
        let do_wrap = has_top_init_body && !has_let_or_const && all_wrappable;

        if !do_wrap {
            // Important: when bailing, walk the *original* program body
            // (not `candidates`) so the un-wrapped re-emission keeps
            // each top-level statement's source slice intact. The
            // synthetic-IIFE flattening is only meaningful inside the
            // wrap path.
            for stmt in prog.body.iter().skip(idx) {
                items.push(self.lower_top_stmt(stmt, decider)?);
            }
            return Ok(Module { items });
        }

        // === 3. Wrap path ===
        // Collect var-decl names for hoisting; lower top-init stmts
        // into a single body. Function decls stay as their own items at
        // their original source position relative to the wrapper.
        // Run this in a closure so any `Error::Unsupported` raised by
        // the deeper lowering (e.g. an inner `object method` we still
        // can't represent) can fall back to the un-wrapped form
        // instead of failing the whole parse.
        let wrap_attempt: Result<(Vec<Item>, Vec<String>, Vec<Stmt>), Error> = (|| {
            let mut wrap_items: Vec<Item> = Vec::new();
            let mut hoisted_var_names: Vec<String> = Vec::new();
            let mut top_init_body: Vec<Stmt> = Vec::new();

            for (stmt, from_iife) in candidates.iter().copied() {
                match stmt {
                    js::Statement::FunctionDeclaration(f) => {
                        wrap_items.push(self.lower_top_function_with_decider(&**f, decider)?);
                    }
                    // Top-level `var` only — spliced var-decls from an
                    // IIFE are already function-locals; keep them as
                    // regular Stmt::Let lowerings rather than hoisting.
                    js::Statement::VariableDeclaration(decl)
                        if !from_iife && matches!(decl.kind, js::VariableDeclarationKind::Var) =>
                    {
                        // Top-level `var`: split into global `var x;`
                        // hoist plus bare assignments inside the
                        // wrapper. The bare assignment routes through
                        // GlobalThis at the SSA layer — same observable
                        // effect as sloppy-mode top-level `var`.
                        for d in &decl.declarations {
                            let name = match &d.id.kind {
                                js::BindingPatternKind::BindingIdentifier(id) => {
                                    id.name.to_string()
                                }
                                _ => return Err(Error::Unsupported("destructuring binding")),
                            };
                            hoisted_var_names.push(name.clone());
                            if let Some(init_expr) = &d.init {
                                let init = self.lower_expr(init_expr)?;
                                top_init_body.push(Stmt::Assign {
                                    target: Expr::Ident(name),
                                    value: init,
                                });
                            }
                        }
                    }
                    // Everything else: route through the regular
                    // statement lowering.
                    other => {
                        let stmts = self.lower_stmts_list(std::slice::from_ref(other))?;
                        top_init_body.extend(stmts);
                    }
                }
            }
            Ok((wrap_items, hoisted_var_names, top_init_body))
        })();

        let (wrap_items, hoisted_var_names, top_init_body) = match wrap_attempt {
            Ok(parts) => parts,
            Err(Error::Unsupported(_)) => {
                // The wrap path tripped on a construct the lowering
                // can't represent yet (e.g. object method shorthand
                // inside an inner literal). Bail to the historical
                // un-wrapped form: every non-function top-level
                // statement becomes an `Item::PassThroughStmt`. We've
                // already collected the directive prologue into
                // `items`; redo the rest from `prog.body`.
                for stmt in prog.body.iter().skip(idx) {
                    items.push(self.lower_top_stmt(stmt, decider)?);
                }
                return Ok(Module { items });
            }
            Err(e) => return Err(e),
        };

        items.extend(wrap_items);

        // Hoist `var` declarations as a single PassThroughStmt placed
        // just before the wrapper. Preserves global visibility for any
        // top-level function that reads these names.
        if !hoisted_var_names.is_empty() {
            // Insert the var hoist at the position where the wrapper
            // will land — i.e., right before the wrapper. We append
            // first; the wrapper goes after.
            let var_decl = format!("var {};", hoisted_var_names.join(", "));
            // Place hoist after directive prologue but before any top
            // function decl so the binding is visible everywhere.
            // Functions hoist textually anyway, so order between them
            // and the var hoist is observationally indistinguishable —
            // we put the hoist at the directive position for simplicity.
            // To preserve original ordering (function decls placed where
            // they textually appeared), we append the wrapper at the end
            // and put the hoist directly before the wrapper. Both are
            // function-scoped/var-hoisted so visibility is preserved.
            items.push(Item::PassThroughStmt(var_decl));
        }

        // Always emit the wrapper, even if `top_init_body` is empty —
        // the eligibility check guaranteed `has_top_init_body`, so
        // body must be non-empty here. Sanity-check.
        debug_assert!(
            !top_init_body.is_empty() || !hoisted_var_names.is_empty(),
            "wrap path entered with empty body and no hoisted vars"
        );
        if !top_init_body.is_empty() {
            items.push(Item::Function(IrFunction {
                name: TOP_INIT_NAME.to_string(),
                params: Vec::new(),
                body: top_init_body,
                source: None,
            }));
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
                //
                // After issue #32 this path is taken only for files
                // that were not eligible for `__top_init__` wrapping
                // (top-level let/const, or class/for/etc. at top level).
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

        if let Disposition::PassThrough(blockers) = disposition {
            return Ok(Item::PassThroughFunction(
                self.passthrough_literal(Some(name), f, body, blockers)?,
            ));
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
        // Lower the parameter list, picking up any prologue stmts the
        // desugaring needs (issue #29: default + destructuring params
        // become identifier params + a body prologue).
        let (params, mut prologue) = self.lower_param_list(&f.params)?;

        let body_stmts = self.lower_stmts_list(&body.statements)?;
        prologue.extend(body_stmts);
        let stmts = prologue;

        // Stash the body source slice so `lower_ssa` can fall back to
        // a PassThrough function if the body contains a construct that
        // cannot be expressed in SSA (currently `Stmt::ForIn` and
        // `Stmt::Try`). `slice_body_inner` may return None for synthetic
        // spans; in that case the SSA fallback is unavailable for this
        // function.
        let source = self
            .slice_body_inner(body.span())
            .map(|s| s.to_string());

        Ok(Item::Function(IrFunction {
            name: name.to_string(),
            params,
            body: stmts,
            source,
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
        blockers: BTreeSet<BlockerKind>,
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
            blockers,
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
    // Parameter-list desugaring (issue #29)
    // ------------------------------------------------------------------
    //
    // Default, destructuring, and try/catch destructuring params lower
    // by *desugaring at parse time* into a simple-identifier param list
    // plus a prologue of statements that materialize the original
    // bindings. The function's arity is preserved (one slot per source
    // parameter); only the *binding shape* changes from
    // pattern-on-param to identifier-on-param + body-prologue stmts.
    //
    // Evaluation order: prologue statements are emitted in source-param
    // order, with the default-undefined check immediately before the
    // destructuring expansion. This matches ES2015 §9.2.10
    // (FunctionDeclarationInstantiation) — defaults evaluate left-to-
    // right and may reference earlier params.
    //
    // Strict-mode note: ES2015 forbids `arguments` from tracking param
    // mutation when the function has any non-simple parameter. Our
    // desugaring doesn't surface `arguments` at all, so this is a
    // non-issue (the SSA layer doesn't model `arguments` either).
    //
    // RestParam (`...args`) is *not* lifted by this helper — it requires
    // a variadic call ABI we don't yet model. Disposition still rejects
    // RestParam at the param-list level.

    /// Lower a parameter list. Returns the simple-identifier param names
    /// and a prologue of `Stmt`s that the caller must prepend to the
    /// function body (or wrap in a Block before the body, in the arrow
    /// expression-body case where there is no body to prepend to).
    ///
    /// Identifier-only params produce no prologue. Default and
    /// destructuring params produce prologue stmts.
    fn lower_param_list(
        &self,
        params: &js::FormalParameters,
    ) -> Result<(Vec<String>, Vec<Stmt>), Error> {
        // RestParam is still a disposition blocker; if we reach here
        // something is wrong with the walker.
        if params.rest.is_some() {
            return Err(Error::Unsupported("rest parameter (`...args`)"));
        }
        let mut names = Vec::with_capacity(params.items.len());
        let mut prologue: Vec<Stmt> = Vec::new();
        for (i, p) in params.items.iter().enumerate() {
            self.lower_one_param(i, &p.pattern, &mut names, &mut prologue)?;
        }
        Ok((names, prologue))
    }

    /// Lower one formal parameter. The slot's name (either the original
    /// identifier or a synthesized `__p<i>` placeholder) is pushed to
    /// `names`; any prologue statements needed to bind the original
    /// pattern's identifiers go to `prologue`.
    fn lower_one_param(
        &self,
        slot: usize,
        pat: &js::BindingPattern,
        names: &mut Vec<String>,
        prologue: &mut Vec<Stmt>,
    ) -> Result<(), Error> {
        match &pat.kind {
            js::BindingPatternKind::BindingIdentifier(id) => {
                names.push(id.name.to_string());
                Ok(())
            }
            js::BindingPatternKind::AssignmentPattern(ap) => {
                // `name = expr` or `pattern = expr`.
                let default_expr = self.lower_expr(&ap.right)?;
                match &ap.left.kind {
                    js::BindingPatternKind::BindingIdentifier(id) => {
                        // Simple default param: `function f(x = E)`.
                        // Emit `if (x === undefined) x = E;`.
                        let name = id.name.to_string();
                        names.push(name.clone());
                        prologue.push(default_undefined_assign(&name, default_expr));
                        Ok(())
                    }
                    _ => {
                        // Destructuring with a default: `function f({a} = {})`.
                        // Bind a synthetic param, default it, then
                        // destructure.
                        let slot_name = format!("__p{slot}");
                        names.push(slot_name.clone());
                        prologue.push(default_undefined_assign(&slot_name, default_expr));
                        self.emit_destructure_into(
                            &ap.left,
                            Expr::Ident(slot_name),
                            prologue,
                        )?;
                        Ok(())
                    }
                }
            }
            js::BindingPatternKind::ObjectPattern(_) | js::BindingPatternKind::ArrayPattern(_) => {
                // `function f({a, b})` / `function f([a, b])`. Bind a
                // synthetic param then destructure into the original
                // names.
                let slot_name = format!("__p{slot}");
                names.push(slot_name.clone());
                self.emit_destructure_into(pat, Expr::Ident(slot_name), prologue)?;
                Ok(())
            }
        }
    }

    /// Emit prologue `let`-declarations that destructure `source` into
    /// the bindings of `pat`. `source` should be a side-effect-free
    /// reference (typically a fresh identifier — the caller binds the
    /// destructured value to a temp first if its source has effects).
    fn emit_destructure_into(
        &self,
        pat: &js::BindingPattern,
        source: Expr,
        out: &mut Vec<Stmt>,
    ) -> Result<(), Error> {
        match &pat.kind {
            js::BindingPatternKind::BindingIdentifier(id) => {
                out.push(Stmt::Let {
                    name: id.name.to_string(),
                    init: Some(source),
                });
                Ok(())
            }
            js::BindingPatternKind::AssignmentPattern(ap) => {
                // Pattern-with-default: bind the source, default if
                // undefined, then continue destructuring the inner
                // pattern. We need a fresh temp because `source` may
                // be referenced multiple times and could be a member
                // load with effects in a nested destructuring.
                let tmp = self.fresh_temp();
                out.push(Stmt::Let {
                    name: tmp.clone(),
                    init: Some(source),
                });
                let default_expr = self.lower_expr(&ap.right)?;
                out.push(default_undefined_assign(&tmp, default_expr));
                self.emit_destructure_into(&ap.left, Expr::Ident(tmp), out)
            }
            js::BindingPatternKind::ObjectPattern(op) => {
                if op.rest.is_some() {
                    return Err(Error::Unsupported("object rest in destructuring"));
                }
                // Bind source to a temp so each property load reuses
                // the same value (avoiding double-evaluation if the
                // source is a member access later in a recursive
                // destructure — though at the param level it's an
                // identifier and the temp is technically unnecessary,
                // it keeps the lowering uniform).
                let (root_expr, maybe_temp) = self.bind_destructure_source(source, out);
                for prop in &op.properties {
                    let key_expr = self.binding_property_load(prop, root_expr.clone())?;
                    self.emit_destructure_into(&prop.value, key_expr, out)?;
                }
                drop(maybe_temp);
                Ok(())
            }
            js::BindingPatternKind::ArrayPattern(ap) => {
                if ap.rest.is_some() {
                    return Err(Error::Unsupported("array rest in destructuring"));
                }
                let (root_expr, maybe_temp) = self.bind_destructure_source(source, out);
                for (i, el) in ap.elements.iter().enumerate() {
                    if let Some(p) = el {
                        let load = Expr::Index {
                            obj: Box::new(root_expr.clone()),
                            index: Box::new(Expr::Number(i as f64)),
                        };
                        self.emit_destructure_into(p, load, out)?;
                    }
                    // Elision: skip the slot.
                }
                drop(maybe_temp);
                Ok(())
            }
        }
    }

    /// If `source` is already a plain identifier, just return it
    /// (cloning is cheap). Otherwise bind it to a fresh temp and return
    /// the temp identifier so subsequent property loads reuse the same
    /// SSA value once the IR lowers.
    fn bind_destructure_source(
        &self,
        source: Expr,
        out: &mut Vec<Stmt>,
    ) -> (Expr, Option<String>) {
        if let Expr::Ident(_) = source {
            return (source, None);
        }
        let tmp = self.fresh_temp();
        out.push(Stmt::Let {
            name: tmp.clone(),
            init: Some(source),
        });
        (Expr::Ident(tmp.clone()), Some(tmp))
    }

    /// Build the property load expression for one BindingProperty.
    fn binding_property_load(
        &self,
        prop: &js::BindingProperty,
        root: Expr,
    ) -> Result<Expr, Error> {
        if prop.computed {
            let e = prop
                .key
                .as_expression()
                .ok_or(Error::Unsupported("non-expression computed destructuring key"))?;
            return Ok(Expr::Index {
                obj: Box::new(root),
                index: Box::new(self.lower_expr(e)?),
            });
        }
        match &prop.key {
            js::PropertyKey::StaticIdentifier(id) => Ok(Expr::Member {
                obj: Box::new(root),
                prop: id.name.to_string(),
            }),
            js::PropertyKey::StringLiteral(s) => Ok(Expr::Member {
                obj: Box::new(root),
                prop: s.value.to_string(),
            }),
            js::PropertyKey::NumericLiteral(n) => Ok(Expr::Index {
                obj: Box::new(root),
                index: Box::new(Expr::Number(n.value)),
            }),
            _ => Err(Error::Unsupported("destructuring key kind")),
        }
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
        if let Disposition::PassThrough(blockers) = disposition {
            return self.passthrough_literal(name, f, body, blockers);
        }

        let (params, mut prologue) = self.lower_param_list(&f.params)?;

        let body_stmts = self.lower_stmts_list(&body.statements)?;
        prologue.extend(body_stmts);
        let stmts = prologue;

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
            blockers: BTreeSet::new(),
        })
    }

    /// Build a `FuncLiteral` for an arrow function. Block-body arrows
    /// produce `FuncBody::Stmts`; expression-body arrows (`x => x+1`)
    /// produce `FuncBody::Expression`. Un-lowerable arrows go through
    /// `FuncBody::PassThrough`.
    fn lower_arrow_literal(&self, a: &js::ArrowFunctionExpression) -> Result<FuncLiteral, Error> {
        let disposition = decide_arrow(a);
        if let Disposition::PassThrough(blockers) = disposition {
            return self.passthrough_arrow(a, blockers);
        }

        let (params, mut prologue) = self.lower_param_list(&a.params)?;

        // Arrow body in OXC is always wrapped in a `FunctionBody`. When
        // `a.expression` is true, body.statements has exactly one
        // ExpressionStatement (synthesized by the parser to host the
        // expression body); otherwise it's a regular block.
        //
        // If the param list produced a prologue (default / destructuring
        // desugaring per issue #29), we cannot keep an expression-body
        // arrow — the prologue stmts have nowhere to live in
        // `FuncBody::Expression`. Convert to `FuncBody::Stmts` with a
        // synthetic `return <expr>;` after the prologue.
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
            let lowered = self.lower_expr(expr_inner)?;
            if prologue.is_empty() {
                (FuncBody::Expression(Box::new(lowered)), pt)
            } else {
                prologue.push(Stmt::Return(Some(lowered)));
                (FuncBody::Stmts(prologue), pt)
            }
        } else {
            let body_stmts = self.lower_stmts_list(&a.body.statements)?;
            prologue.extend(body_stmts);
            let pt = self.slice_body_inner(a.body.span()).map(|s| s.to_string());
            (FuncBody::Stmts(prologue), pt)
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
            blockers: BTreeSet::new(),
        })
    }

    /// PassThrough fallback for an un-lowerable arrow. We always emit a
    /// block-body PassThrough — for an expression-body arrow we wrap
    /// the expression text in `return ...;` so the re-emit (which uses
    /// `=> { <slice> }`) parses as a block-body arrow with the same
    /// observable behavior.
    fn passthrough_arrow(&self, a: &js::ArrowFunctionExpression, blockers: BTreeSet<BlockerKind>) -> Result<FuncLiteral, Error> {
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
            blockers,
        })
    }

    // ------------------------------------------------------------------
    // Statement / expression lowering
    // ------------------------------------------------------------------

    fn lower_stmt(&self, stmt: &js::Statement) -> Result<Stmt, Error> {
        match stmt {
            js::Statement::VariableDeclaration(decl) => {
                // Single-declarator path only — multi-declarator is handled by
                // lower_stmts_list / stmts_from which expand the decl in-place.
                // Disposition guarantees single-declarator in for-init contexts.
                let stmts = self.lower_var_decl(decl)?;
                debug_assert_eq!(stmts.len(), 1, "lower_stmt called for multi-declarator decl");
                stmts.into_iter().next().ok_or_else(|| Error::Malformed("empty var decl".into()))
            }
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
                Ok(Stmt::Block(self.lower_stmts_list(&b.body)?))
            }
            js::Statement::FunctionDeclaration(f) => {
                Ok(Stmt::FuncDecl(self.lower_function_literal(f, true)?))
            }
            js::Statement::ForStatement(f) => self.lower_for_stmt(f),
            js::Statement::ForOfStatement(f) => self.lower_for_of_stmt(f),
            js::Statement::ForInStatement(f) => self.lower_for_in_stmt(f),
            js::Statement::ContinueStatement(c) => {
                if c.label.is_some() {
                    return Err(Error::Unsupported("labeled continue"));
                }
                Ok(Stmt::Continue)
            }
            js::Statement::TryStatement(t) => self.lower_try_stmt(t),
            js::Statement::ThrowStatement(t) => Ok(Stmt::Throw(self.lower_expr(&t.argument)?)),
            js::Statement::BreakStatement(b) => Ok(Stmt::Break {
                label: b.label.as_ref().map(|l| l.name.to_string()),
            }),
            js::Statement::SwitchStatement(s) => {
                let discriminant = self.lower_expr(&s.discriminant)?;
                let mut cases = Vec::with_capacity(s.cases.len());
                for c in &s.cases {
                    let test = match &c.test {
                        Some(e) => Some(self.lower_expr(e)?),
                        None => None,
                    };
                    let body = self.lower_stmts_list(&c.consequent)?;
                    cases.push(SwitchCase { test, body });
                }
                Ok(Stmt::Switch { discriminant, cases })
            }
            _ => Err(Error::Unsupported(stmt_kind(stmt))),
        }
    }

    fn lower_for_stmt(&self, f: &js::ForStatement) -> Result<Stmt, Error> {
        // For-init multi-declarator (`var` only — disposition keeps
        // `let`/`const` blocked): keep the first declarator in the
        // for-init slot, hoist the rest as preceding `var` statements,
        // and wrap the whole thing in a Block. `var` is function-scoped
        // so the hoist preserves all observable behavior.
        let mut hoisted_prelude: Vec<Stmt> = Vec::new();
        let init = match &f.init {
            None => None,
            Some(js::ForStatementInit::VariableDeclaration(decl)) => {
                let mut stmts = self.lower_var_decl(decl)?;
                if stmts.is_empty() {
                    None
                } else {
                    let kept = stmts.remove(0);
                    hoisted_prelude = stmts;
                    Some(Box::new(kept))
                }
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
        let for_stmt = Stmt::For {
            init,
            cond,
            step,
            body,
        };
        if hoisted_prelude.is_empty() {
            Ok(for_stmt)
        } else {
            // Wrap so the caller still gets a single Stmt back. Block
            // re-emits as `{ ... }` which is correct since `var` ignores
            // block scoping anyway.
            hoisted_prelude.push(for_stmt);
            Ok(Stmt::Block(hoisted_prelude))
        }
    }

    fn lower_for_in_stmt(&self, f: &js::ForInStatement) -> Result<Stmt, Error> {
        let (kind, binding) = match &f.left {
            js::ForStatementLeft::VariableDeclaration(decl) => {
                if decl.declarations.len() != 1 {
                    return Err(Error::Unsupported("for-in with multiple declarators"));
                }
                let d = &decl.declarations[0];
                let name = match &d.id.kind {
                    js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
                    _ => return Err(Error::Unsupported("for-in with destructuring binding")),
                };
                let kind = match decl.kind {
                    js::VariableDeclarationKind::Const => "const".to_string(),
                    js::VariableDeclarationKind::Let => "let".to_string(),
                    js::VariableDeclarationKind::Var => "var".to_string(),
                    _ => return Err(Error::Unsupported("for-in with exotic binding kind")),
                };
                (kind, name)
            }
            _ => return Err(Error::Unsupported("for-in with non-declaration left side")),
        };
        let obj = self.lower_expr(&f.right)?;
        let body = self.stmts_from(&f.body)?;
        Ok(Stmt::ForIn {
            kind,
            binding,
            obj: Box::new(obj),
            body,
        })
    }

    fn lower_for_of_stmt(&self, f: &js::ForOfStatement) -> Result<Stmt, Error> {
        if f.r#await {
            return Err(Error::Unsupported("for-await-of statement"));
        }
        let (kind, binding) = match &f.left {
            js::ForStatementLeft::VariableDeclaration(decl) => {
                if decl.declarations.len() != 1 {
                    return Err(Error::Unsupported("for-of with multiple declarators"));
                }
                let d = &decl.declarations[0];
                let name = match &d.id.kind {
                    js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
                    _ => return Err(Error::Unsupported("for-of with destructuring binding")),
                };
                let kind = match decl.kind {
                    js::VariableDeclarationKind::Const => "const".to_string(),
                    js::VariableDeclarationKind::Let => "let".to_string(),
                    js::VariableDeclarationKind::Var => "var".to_string(),
                    _ => return Err(Error::Unsupported("for-of with exotic binding kind")),
                };
                (kind, name)
            }
            _ => return Err(Error::Unsupported("for-of with non-declaration left side")),
        };
        let iterable = self.lower_expr(&f.right)?;
        let body = self.stmts_from(&f.body)?;
        Ok(Stmt::ForOf {
            kind,
            binding,
            iterable: Box::new(iterable),
            body,
        })
    }

    fn lower_try_stmt(&self, t: &js::TryStatement) -> Result<Stmt, Error> {
        let body = self.lower_stmts_list(&t.block.body)?;

        let catch = match &t.handler {
            None => None,
            Some(handler) => {
                // Identifier params bind directly. Destructuring catch
                // params (issue #29: `catch ({message})`) desugar by
                // binding a fresh synthetic catch name and emitting a
                // destructuring prologue at the top of the catch body —
                // mirrors the function-param desugaring path.
                let (param_name, mut prologue) = match handler.param.as_ref() {
                    None => (None, Vec::new()),
                    Some(p) => match &p.pattern.kind {
                        js::BindingPatternKind::BindingIdentifier(id) => {
                            (Some(id.name.to_string()), Vec::new())
                        }
                        _ => {
                            // Synthesize a catch-binding name that
                            // wouldn't collide with user code, then
                            // destructure into the original pattern's
                            // identifiers.
                            let slot = self.fresh_temp();
                            let mut prologue: Vec<Stmt> = Vec::new();
                            self.emit_destructure_into(
                                &p.pattern,
                                Expr::Ident(slot.clone()),
                                &mut prologue,
                            )?;
                            (Some(slot), prologue)
                        }
                    },
                };
                let catch_body = self.lower_stmts_list(&handler.body.body)?;
                prologue.extend(catch_body);
                Some((param_name, prologue))
            }
        };

        let finally = match &t.finalizer {
            None => None,
            Some(finalizer) => Some(self.lower_stmts_list(&finalizer.body)?),
        };

        Ok(Stmt::Try { body, catch, finally })
    }

    /// Lower a variable declaration, returning one `Stmt` per declarator.
    ///
    /// Returning a `Vec` lets the caller flat-map multi-declarator declarations
    /// into the surrounding statement list without creating a new block scope.
    /// Disposition ensures that for-init declarations always have exactly one
    /// declarator (multi-declarator for-init stays blocked), so callers in
    /// for-init position can safely unwrap the single element.
    fn lower_var_decl(&self, decl: &js::VariableDeclaration) -> Result<Vec<Stmt>, Error> {
        let mut stmts = Vec::with_capacity(decl.declarations.len());
        for d in &decl.declarations {
            let name = match &d.id.kind {
                js::BindingPatternKind::BindingIdentifier(id) => id.name.to_string(),
                _ => return Err(Error::Unsupported("destructuring binding")),
            };
            let init = match &d.init {
                Some(e) => Some(self.lower_expr(e)?),
                None => None,
            };
            let stmt = match decl.kind {
                js::VariableDeclarationKind::Let | js::VariableDeclarationKind::Var => {
                    Stmt::Let { name, init }
                }
                js::VariableDeclarationKind::Const => {
                    let init = init
                        .ok_or_else(|| Error::Malformed("const without initializer".into()))?;
                    Stmt::Const { name, init }
                }
                _ => return Err(Error::Unsupported("`using` declaration")),
            };
            stmts.push(stmt);
        }
        Ok(stmts)
    }

    /// Lower a slice of JS statements, expanding multi-declarator declarations
    /// in-place so the result is a flat IR statement list.
    fn lower_stmts_list(&self, stmts: &[js::Statement]) -> Result<Vec<Stmt>, Error> {
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            if let js::Statement::VariableDeclaration(decl) = s {
                out.extend(self.lower_var_decl(decl)?);
            } else {
                out.push(self.lower_stmt(s)?);
            }
        }
        Ok(out)
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
            self.lower_stmts_list(&b.body)
        } else if let js::Statement::VariableDeclaration(decl) = stmt {
            self.lower_var_decl(decl)
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
                // Canonicalize the global numeric-value identifiers
                // (`undefined`, `NaN`, `Infinity`) to literal-shape IR
                // nodes. This matches the existing `undefined` fold
                // and converges tree_back / tree_module2 on the same
                // canonical form so the structural roundtrip check
                // doesn't see them as Number-vs-Ident divergence.
                //
                // Shadowing (`let NaN = 5;`, etc.) is legal JS but
                // does not occur in the corpus; we accept the same
                // assumption that `undefined` already relies on. If a
                // shadowed binding ever appears, this fold would
                // incorrectly canonicalize the rebound name — flagged
                // as a known limitation, not a silent miscompile,
                // because the value would still be IEEE-correct.
                match id.name.as_str() {
                    "undefined" => Ok(Expr::Undefined),
                    "NaN" => Ok(Expr::Number(f64::NAN)),
                    "Infinity" => Ok(Expr::Number(f64::INFINITY)),
                    _ => Ok(Expr::Ident(id.name.to_string())),
                }
            }
            js::Expression::ThisExpression(_) => Ok(Expr::This),
            js::Expression::RegExpLiteral(r) => {
                // Prefer the raw source slice (`/pattern/flags`) when
                // OXC stamped one — that's the only path the parser
                // exercises. Fall back to reconstructing from
                // pattern + flags for hand-built test ASTs (the
                // `raw: Option` documents this case).
                // OXC always stamps `raw` for parsed source. Reach
                // the Err branch only for hand-built test ASTs.
                let raw = r
                    .raw
                    .as_ref()
                    .ok_or(Error::Unsupported("RegExpLiteral without raw slice"))?;
                Ok(Expr::RegExp(raw.to_string()))
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
                // `-0` source folds to `Number(-0.0)` directly. The JS
                // parser models `-0` as a UnaryExpression over a
                // positive 0 literal; without this fold, `tree_back`
                // (where SSA `Constant::Number(-0.0)` lowers to
                // `Expr::Number(-0.0)`) and `tree_module2` (the
                // re-parsed emit of `(-0)`) would diverge in shape.
                // Folding both sides through the same path converges
                // them. `f64::is_sign_negative` preserves the IEEE
                // -0 bit pattern; using `-n` directly would not, since
                // the type is `f64` and `-0.0_f64` retains the sign
                // bit only when constructed explicitly.
                if matches!(u.operator, js::UnaryOperator::UnaryNegation)
                    && let js::Expression::NumericLiteral(n) = &u.argument
                    && n.value == 0.0
                {
                    return Ok(Expr::Number(-0.0));
                }
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
                        js::Argument::SpreadElement(s) => {
                            args.push(Expr::Spread(Box::new(self.lower_expr(&s.argument)?)));
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
                        js::Argument::SpreadElement(s) => {
                            args.push(Expr::Spread(Box::new(self.lower_expr(&s.argument)?)));
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
                        js::ObjectPropertyKind::SpreadProperty(s) => {
                            let value = self.lower_expr(&s.argument)?;
                            props.push(ObjectProp {
                                key: ObjectKey::Spread,
                                value,
                            });
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
                        js::ArrayExpressionElement::SpreadElement(s) => {
                            elements.push(Expr::Spread(Box::new(self.lower_expr(&s.argument)?)));
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

            js::Expression::TemplateLiteral(tpl) => {
                // Untagged template literal: `` `a${x}b${y}c` ``.
                // quasis and expressions interleave:
                //   quasis[0], exprs[0], quasis[1], exprs[1], ..., quasis[n]
                // with quasis.len() == exprs.len() + 1.
                let quasis: Vec<String> = tpl
                    .quasis
                    .iter()
                    .map(|q| q.value.raw.to_string())
                    .collect();
                let mut exprs = Vec::with_capacity(tpl.expressions.len());
                for e in &tpl.expressions {
                    exprs.push(self.lower_expr(e)?);
                }
                Ok(Expr::Template { quasis, exprs })
            }

            js::Expression::SequenceExpression(s) => {
                let mut exprs = Vec::with_capacity(s.expressions.len());
                for e in &s.expressions {
                    exprs.push(self.lower_expr(e)?);
                }
                Ok(Expr::Sequence { exprs })
            }

            js::Expression::AssignmentExpression(a) => {
                let target = self.lower_assign_target(&a.left)?;
                let value = self.lower_expr(&a.right)?;
                if matches!(a.operator, js::AssignmentOperator::Assign) {
                    return Ok(Expr::Assign {
                        target: Box::new(target),
                        value: Box::new(value),
                    });
                }
                // Compound (`+=`, `-=`, ...). Logical-assign (`||=`,
                // `&&=`, `??=`) is still blocked at the walker; if
                // `compound_to_binop` rejects, the walker is too
                // permissive — surface as Unsupported.
                let op = compound_to_binop(a.operator)?;
                Ok(Expr::CompoundAssign {
                    op,
                    target: Box::new(target),
                    value: Box::new(value),
                })
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
        B::Exponential => BinOp::Pow,
        B::In => BinOp::In,
        B::Instanceof => BinOp::Instanceof,
    })
}

/// If `stmt` is the exact emit shape of a synthetic `__top_init__`
/// IIFE — `(function () { ... })();` (anonymous, no params, no args,
/// not async/generator) — return the inner body's statements so the
/// re-parse path can splice them in place of the wrapper before
/// applying its own wrapping. This is what keeps the structural
/// roundtrip symmetric: emit produces the IIFE, re-parse strips it.
fn synthetic_top_init_iife_body<'a, 'src>(
    stmt: &'a js::Statement<'src>,
) -> Option<&'a [js::Statement<'src>]> {
    let es = match stmt {
        js::Statement::ExpressionStatement(es) => es,
        _ => return None,
    };
    let call = match &es.expression {
        js::Expression::CallExpression(c) => c,
        _ => return None,
    };
    if !call.arguments.is_empty() {
        return None;
    }
    // Callee may be parenthesized.
    let callee = match &call.callee {
        js::Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    let func = match callee {
        js::Expression::FunctionExpression(f) => f,
        _ => return None,
    };
    if func.id.is_some() || func.r#async || func.generator || !func.params.items.is_empty() {
        return None;
    }
    if func.params.rest.is_some() {
        return None;
    }
    let body = func.body.as_ref()?;
    Some(body.statements.as_slice())
}

/// True iff `stmt` is a directive-prologue entry — a top-level
/// expression statement whose expression is a string literal. Per
/// ECMA-262 14.1.1, only string-literal expression statements at the
/// start of a program/function body are directives. We only check the
/// shape; any positional pruning is the caller's responsibility.
fn is_directive_prologue(stmt: &js::Statement) -> bool {
    match stmt {
        js::Statement::ExpressionStatement(es) => {
            matches!(&es.expression, js::Expression::StringLiteral(_))
        }
        _ => false,
    }
}

/// Conservative "is this statement wrap-safe for `__top_init__`?" walker.
///
/// The SSA layer panics on `switch` cases that fall through to the
/// next case (no `break`/`return`/`continue`/`throw` at the case body's
/// tail). Disposition still classifies such functions as Lowered, and
/// inner function literals lowered along with `__top_init__`'s body
/// would hit the panic too. Without a per-function PassThrough fallback
/// for the synthesized wrapper, the only safe move is to detect
/// fallthrough at AST time and refuse to wrap if any sub-tree (nested
/// through expressions, statements, AND inner function bodies) has it.
///
/// We approximate by walking every `SwitchStatement` reachable from
/// `stmt` (including inside function expressions / arrow bodies / inner
/// function declarations) and rejecting if any case body fails to end
/// in a clear terminator. Other constructs disposition would block at
/// the function level (`for-await`, `with`, etc.) are also rejected
/// here for the same reason.
fn stmt_wrap_safe(stmt: &js::Statement) -> bool {
    let mut blocked = false;
    walk_for_wrap_blockers(stmt, &mut blocked);
    !blocked
}

fn walk_for_wrap_blockers(stmt: &js::Statement, blocked: &mut bool) {
    use js::Statement as S;
    if *blocked {
        return;
    }
    match stmt {
        S::SwitchStatement(s) => {
            // Each case body must terminate cleanly. `default` at the
            // end is fine if its last stmt terminates; otherwise it
            // falls through to switch-exit, which the SSA layer can't
            // model yet (panics in `lower_switch`).
            for case in &s.cases {
                let last_terminates = case
                    .consequent
                    .last()
                    .is_some_and(|st| stmt_terminates(st));
                let is_last_case = std::ptr::eq(case, s.cases.last().unwrap());
                if !last_terminates && !is_last_case {
                    *blocked = true;
                    return;
                }
                if !last_terminates && is_last_case && !case.consequent.is_empty() {
                    // Last case without termination falls through to
                    // switch-exit. SSA's `lower_switch` panics here too.
                    *blocked = true;
                    return;
                }
                for st in &case.consequent {
                    walk_for_wrap_blockers(st, blocked);
                }
                if let Some(t) = &case.test {
                    walk_for_wrap_blockers_in_expr_required(t, blocked);
                }
                if *blocked { return; }
            }
            walk_for_wrap_blockers_in_expr_required(&s.discriminant, blocked);
        }
        // Constructs that, while disposition-allowed, can still
        // surface SSA-level panics or invariant violations when the
        // wrapper exposes them to the optimizer (e.g. a nested
        // function whose captures get rewritten by an enclosing
        // `try` / `for-in` / labeled break in ways the SSA invariant
        // checker doesn't expect). The conservative gate is to refuse
        // to wrap files that contain any of these shapes — they keep
        // the un-wrapped PassThroughStmt fallback that worked before
        // issue #32.
        S::TryStatement(_) => { *blocked = true; }
        S::ForInStatement(_) => { *blocked = true; }
        S::DoWhileStatement(_) => { *blocked = true; }
        S::LabeledStatement(_) => { *blocked = true; }
        S::ClassDeclaration(_) => { *blocked = true; }
        S::DebuggerStatement(_) => { *blocked = true; }
        S::WithStatement(_) => { *blocked = true; }
        S::ExpressionStatement(es) => {
            walk_for_wrap_blockers_in_expr_required(&es.expression, blocked);
        }
        S::VariableDeclaration(d) => {
            for decl in &d.declarations {
                if let Some(init) = &decl.init {
                    walk_for_wrap_blockers_in_expr_required(init, blocked);
                }
            }
        }
        S::IfStatement(s) => {
            walk_for_wrap_blockers_in_expr_required(&s.test, blocked);
            walk_for_wrap_blockers(&s.consequent, blocked);
            if let Some(alt) = &s.alternate {
                walk_for_wrap_blockers(alt, blocked);
            }
        }
        S::BlockStatement(b) => {
            for st in &b.body { walk_for_wrap_blockers(st, blocked); }
        }
        S::WhileStatement(s) => {
            walk_for_wrap_blockers_in_expr_required(&s.test, blocked);
            walk_for_wrap_blockers(&s.body, blocked);
        }
        S::ForStatement(s) => {
            if let Some(init) = &s.init {
                if let Some(e) = init.as_expression() {
                    walk_for_wrap_blockers_in_expr_required(e, blocked);
                }
            }
            if let Some(t) = &s.test { walk_for_wrap_blockers_in_expr_required(t, blocked); }
            if let Some(u) = &s.update { walk_for_wrap_blockers_in_expr_required(u, blocked); }
            walk_for_wrap_blockers(&s.body, blocked);
        }
        S::ForOfStatement(s) => {
            walk_for_wrap_blockers_in_expr_required(&s.right, blocked);
            walk_for_wrap_blockers(&s.body, blocked);
        }
        S::ReturnStatement(r) => {
            if let Some(arg) = &r.argument {
                walk_for_wrap_blockers_in_expr_required(arg, blocked);
            }
        }
        S::ThrowStatement(t) => {
            walk_for_wrap_blockers_in_expr_required(&t.argument, blocked);
        }
        S::FunctionDeclaration(f) => {
            if let Some(body) = &f.body {
                for st in &body.statements {
                    walk_for_wrap_blockers(st, blocked);
                }
            }
        }
        S::BreakStatement(_) | S::ContinueStatement(_) | S::EmptyStatement(_) => {}
        _ => { *blocked = true; }
    }
}

fn walk_for_wrap_blockers_in_expr_required(e: &js::Expression, blocked: &mut bool) {
    use js::Expression as E;
    if *blocked { return; }
    match e {
        E::FunctionExpression(f) => {
            if let Some(body) = &f.body {
                for st in &body.statements {
                    walk_for_wrap_blockers(st, blocked);
                }
            }
        }
        E::ArrowFunctionExpression(a) => {
            for st in &a.body.statements {
                walk_for_wrap_blockers(st, blocked);
            }
        }
        E::CallExpression(c) => {
            walk_for_wrap_blockers_in_expr_required(&c.callee, blocked);
            for arg in &c.arguments {
                if let Some(ex) = arg.as_expression() {
                    walk_for_wrap_blockers_in_expr_required(ex, blocked);
                }
            }
        }
        E::NewExpression(n) => {
            walk_for_wrap_blockers_in_expr_required(&n.callee, blocked);
            for arg in &n.arguments {
                if let Some(ex) = arg.as_expression() {
                    walk_for_wrap_blockers_in_expr_required(ex, blocked);
                }
            }
        }
        E::ParenthesizedExpression(p) => {
            walk_for_wrap_blockers_in_expr_required(&p.expression, blocked);
        }
        E::SequenceExpression(s) => {
            for ex in &s.expressions {
                walk_for_wrap_blockers_in_expr_required(ex, blocked);
            }
        }
        E::AssignmentExpression(a) => {
            walk_for_wrap_blockers_in_expr_required(&a.right, blocked);
        }
        E::BinaryExpression(b) => {
            walk_for_wrap_blockers_in_expr_required(&b.left, blocked);
            walk_for_wrap_blockers_in_expr_required(&b.right, blocked);
        }
        E::LogicalExpression(l) => {
            walk_for_wrap_blockers_in_expr_required(&l.left, blocked);
            walk_for_wrap_blockers_in_expr_required(&l.right, blocked);
        }
        E::ConditionalExpression(c) => {
            walk_for_wrap_blockers_in_expr_required(&c.test, blocked);
            walk_for_wrap_blockers_in_expr_required(&c.consequent, blocked);
            walk_for_wrap_blockers_in_expr_required(&c.alternate, blocked);
        }
        E::UnaryExpression(u) => walk_for_wrap_blockers_in_expr_required(&u.argument, blocked),
        E::UpdateExpression(_)
        | E::Identifier(_)
        | E::NumericLiteral(_)
        | E::StringLiteral(_)
        | E::BooleanLiteral(_)
        | E::NullLiteral(_)
        | E::ThisExpression(_)
        | E::RegExpLiteral(_)
        | E::TemplateLiteral(_)
        | E::BigIntLiteral(_) => {}
        E::ArrayExpression(arr) => {
            for el in &arr.elements {
                if let Some(ex) = el.as_expression() {
                    walk_for_wrap_blockers_in_expr_required(ex, blocked);
                }
            }
        }
        E::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let js::ObjectPropertyKind::ObjectProperty(p) = prop {
                    walk_for_wrap_blockers_in_expr_required(&p.value, blocked);
                }
            }
        }
        E::StaticMemberExpression(m) => walk_for_wrap_blockers_in_expr_required(&m.object, blocked),
        E::ComputedMemberExpression(m) => {
            walk_for_wrap_blockers_in_expr_required(&m.object, blocked);
            walk_for_wrap_blockers_in_expr_required(&m.expression, blocked);
        }
        // Anything we haven't enumerated: be conservative.
        _ => { *blocked = true; }
    }
}

/// True iff `stmt`'s control flow always terminates the enclosing
/// case-body without falling through. Used by the wrap-safety check
/// to rule out switches whose case bodies fall through to the next
/// case (or to switch-exit).
fn stmt_terminates(stmt: &js::Statement) -> bool {
    use js::Statement as S;
    match stmt {
        S::ReturnStatement(_)
        | S::BreakStatement(_)
        | S::ContinueStatement(_)
        | S::ThrowStatement(_) => true,
        S::BlockStatement(b) => b.body.last().is_some_and(stmt_terminates),
        S::IfStatement(s) => {
            s.alternate.as_ref().is_some_and(|alt| {
                stmt_terminates(&s.consequent) && stmt_terminates(alt)
            })
        }
        _ => false,
    }
}

fn stmt_kind(s: &js::Statement) -> &'static str {
    match s {
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
