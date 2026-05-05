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

use crate::ssa::FuncId;

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
    /// Top-level non-function statement preserved as a verbatim source
    /// slice. The SSA pipeline is function-centric — top-level
    /// statements have no per-instruction representation in SSA — so
    /// any such statement (IIFE, top-level expression, top-level `for`
    /// / `class`, etc.) round-trips as opaque text, mirroring how
    /// un-lowerable function bodies use `FuncBody::PassThrough`. This
    /// is what `lower.rs::lower_top_stmt` produces for everything that
    /// is not a `FunctionDeclaration`; the structured `Item::Stmt`
    /// variant is retained for hand-built tests but not produced by
    /// the OXC frontend.
    PassThroughStmt(String),
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
    /// Verbatim body source slice (between `{` and `}`, exclusive).
    /// Populated at AST→tree-IR time. Used by the SSA layer to fall
    /// back to a PassThrough function when the body contains a
    /// construct that cannot be expressed in SSA form (currently
    /// `Stmt::ForIn` and `Stmt::Try`). `None` for hand-built tree-IR
    /// in tests; the SSA fallback path is then unavailable.
    pub source: Option<String>,
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
    /// C-style `for (init; cond; step) body`. Each header part is
    /// optional in JS source; an absent `cond` is treated as the
    /// constant `true`. SSA lowering routes `Stmt::Continue` inside
    /// `body` to a synthesized step block that runs `step` before
    /// branching back to the loop header — distinct from `While`, where
    /// continue jumps directly to the header.
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Vec<Stmt>,
    },
    /// `for (kind binding of iterable) body`. Desugared to the iterator
    /// protocol in the SSA layer; reconstructed as a `while` loop by
    /// `lower_tree`. Emit is direct so the tree-IR round-trip preserves
    /// the for-of form when no SSA pass is run.
    ForOf {
        /// `const`, `let`, or `var` — stored as a string for simplicity.
        kind: String,
        /// Plain identifier binding (e.g. `x` in `for (const x of arr)`).
        binding: String,
        /// The iterable expression.
        iterable: Box<Expr>,
        /// The loop body.
        body: Vec<Stmt>,
    },
    /// `for (kind binding in obj) body`. Iterates string-named
    /// enumerable properties of `obj`, walking the prototype chain.
    /// Preserved syntactically rather than desugared: the spec
    /// iteration semantics are too gnarly to reimplement faithfully
    /// (prototype walk + shadowed-by-non-enumerable + numeric-index
    /// ordering). Functions containing this statement do not enter
    /// SSA optimization — `lower_ssa::lower_module` detects ForIn in
    /// the body and routes the function to PassThrough using
    /// `IrFunction.source`, mirroring the precedent for `Stmt::Try`.
    ForIn {
        /// `const`, `let`, or `var` — stored as a string for parity
        /// with `ForOf`.
        kind: String,
        /// Plain identifier binding.
        binding: String,
        /// The object being enumerated.
        obj: Box<Expr>,
        /// Loop body.
        body: Vec<Stmt>,
    },
    /// `continue` inside the nearest enclosing loop. Lowered as a jump
    /// to the loop's continue-target (header for `While`, step block
    /// for `For`). `break` is not supported (see docs/DECISIONS.md).
    Continue,
    Block(Vec<Stmt>),
    /// Nested function declaration (`function name(...) { ... }` inside
    /// another function body). Top-level declarations stay as
    /// `Item::Function`; this variant is reserved for inner ones, which
    /// the lowering emits at their textual position (we do not hoist
    /// — see project memory: closures-disposition-rule).
    FuncDecl(FuncLiteral),
    /// `try { body } catch (param) { catch_body } finally { finally_body }`.
    /// All three forms are supported:
    /// - try/catch: `catch` is `Some(...)`, `finally` is `None`
    /// - try/finally: `catch` is `None`, `finally` is `Some(...)`
    /// - try/catch/finally: both present
    /// The catch param is `None` for `catch { }` (no binding) or
    /// `Some(name)` for `catch (e) { }`.
    ///
    /// Design decision: `Stmt::Try` does NOT enter SSA. Functions containing
    /// try/catch are lowered through tree-IR only; `lower_ssa` panics if it
    /// reaches a Try node. This means SSA-level passes (dead code, constant
    /// prop, escape analysis) cannot reason across try boundaries or see the
    /// catch binding. For a deobfuscator with local-only analysis this is
    /// acceptable, but any future global pass will need exception edges in the
    /// SSA CFG — a substantial retrofit.
    ///
    /// Catch binding scope: the catch param (if present) is block-scoped to
    /// the catch clause per ES2015+. SSA passes don't see the try at all, so
    /// they also don't see the catch binding. This is safe as long as no pass
    /// assumes flat lexical scope across the whole function body — if such a
    /// pass is added, it must be made Try-aware or restricted to non-try
    /// functions.
    Try {
        body: Vec<Stmt>,
        /// Optional catch clause: `(Option<catch_param_name>, catch_body)`.
        catch: Option<(Option<String>, Vec<Stmt>)>,
        /// Optional finally block.
        finally: Option<Vec<Stmt>>,
    },
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
    /// Construct kinds that prevented this function from being lowered.
    /// Non-empty only when `body` is `FuncBody::PassThrough` and the
    /// PassThrough was caused by AST-level disposition (not SSA-level
    /// capture reclassification via `force_passthrough`). Populated at
    /// initial-lowering time; SSA and tree-reconstruction layers leave
    /// this empty.
    pub blockers: std::collections::BTreeSet<crate::blocker::BlockerKind>,
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
    Array {
        elements: Vec<Expr>,
    },
    Object {
        props: Vec<ObjectProp>,
    },
    /// Spread element `...expr`. Valid only inside `Expr::Call` args,
    /// `Expr::Array` elements, and `Expr::Object` props (as a spread
    /// property represented via a special `ObjectProp` with a
    /// `ObjectKey::Spread` key). The emitter renders it as `...inner`.
    Spread(Box<Expr>),
    New {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    /// `cond ? then : else`. SSA lowering produces a control-flow
    /// diamond: writes the value into a synthesized symbol on each side
    /// and reads it after the join, so Braun's algorithm inserts the
    /// phi automatically (matching the existing if/else convention).
    Ternary {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    /// `++x`, `x++`, `--x`, `x--`. Carries its target and prefix flag.
    /// SSA lowering evaluates the receiver/key of property targets
    /// exactly once before the read+write, then yields the old or new
    /// value depending on `prefix`. The read-modify-write is `x = x +
    /// 1` (or `- 1`) — JS's `ToNumber` coercion is approximated as the
    /// `+`/`-` operator's coercion. Numeric inputs match strict
    /// semantics; non-numeric inputs diverge (documented in
    /// docs/DECISIONS.md).
    Update {
        op: UpdateOp,
        target: Box<Expr>,
        prefix: bool,
    },
    /// Template literal: `` `quasi0${expr0}quasi1${expr1}quasi2` ``.
    /// `quasis` holds the raw string parts between `${...}` delimiters;
    /// there is always one more quasi than expressions
    /// (`quasis.len() == exprs.len() + 1`).
    Template {
        quasis: Vec<String>,
        exprs: Vec<Expr>,
    },
    /// Comma operator: `(a, b, c)`. Evaluates each operand left-to-right
    /// for side effects; the value of the expression is the last
    /// operand. Preserved as syntax through the IR rather than
    /// decomposed into separate statements, because doing so collapses
    /// the bundler-interop pattern `(0, obj.method)(args)` (which
    /// strips the receiver and binds `this` to undefined) into a plain
    /// method call. The SSA `Expr::Call` arm only specializes
    /// `Expr::Member` callees into `CallMethod`; a `Sequence` callee
    /// falls through to the plain `Call` path, preserving
    /// `this === undefined`.
    Sequence {
        exprs: Vec<Expr>,
    },
    /// Plain `=` assignment used as an expression (`if (x = foo())`,
    /// `a = b = c`, `arr[i++] = expr`). Statement-level `=` continues
    /// to lower to `Stmt::Assign`; this variant is reserved for
    /// assignments in expression position. Compound (`+=` etc.) and
    /// logical (`||=` etc.) operators are NOT represented here and
    /// remain blocked. Rendering syntactically as `target = value`
    /// preserves ECMA-262 13.15 spec semantics — e.g. setter-LHS
    /// behavior — without statically reasoning about the target.
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    /// Compound assignment in expression position: `x op= rhs`,
    /// `obj.x op= rhs`, `obj[k] op= rhs`. Like `Expr::Assign`, this
    /// preserves spec semantics (single LHS evaluation, getter/setter
    /// asymmetry, exception ordering) by rendering syntactically rather
    /// than statically reasoning about the target. Statement-position
    /// compound assignment is decomposed at lower-time into temps +
    /// `Stmt::Assign`; this variant is for cases where the assignment's
    /// value flows into a surrounding expression. `op` is the binary
    /// operator that combines with `=`: e.g. `BinOp::Add` for `+=`.
    /// Logical-assign (`||=`, `&&=`, `??=`) is NOT represented here —
    /// short-circuit semantics need a different lowering shape.
    CompoundAssign {
        op: BinOp,
        target: Box<Expr>,
        value: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateOp {
    Inc,
    Dec,
}

#[derive(Debug, Clone)]
pub enum ObjectKey {
    /// Identifier or string-literal key. Stored as the string value;
    /// emit decides quoting.
    Static(String),
    /// `{[expr]: v}` — key computed at runtime.
    Computed(Box<Expr>),
    /// Spread property marker: `{...expr}`. When this key is used, the
    /// associated `ObjectProp.value` holds the expression being spread.
    Spread,
}

#[derive(Debug, Clone)]
pub struct ObjectProp {
    pub key: ObjectKey,
    pub value: Expr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
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
        BinOp::Pow => "**",
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
            Item::PassThroughStmt(src) => {
                out.push_str("passthrough_stmt {\n");
                for line in src.lines() {
                    writeln!(&mut out, "  {line}").unwrap();
                }
                out.push_str("}\n\n");
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
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            let cond_s = cond.as_ref().map(dump_expr).unwrap_or_else(|| "_".into());
            let step_s = step.as_ref().map(dump_expr).unwrap_or_else(|| "_".into());
            writeln!(out, "{pad}for (_; {}; {}) {{", cond_s, step_s).unwrap();
            if let Some(init) = init {
                dump_stmt(out, init, depth + 1);
            }
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        Stmt::ForOf {
            kind,
            binding,
            iterable,
            body,
        } => {
            writeln!(
                out,
                "{pad}for ({kind} {binding} of {}) {{",
                dump_expr(iterable)
            )
            .unwrap();
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        Stmt::ForIn {
            kind,
            binding,
            obj,
            body,
        } => {
            writeln!(
                out,
                "{pad}for ({kind} {binding} in {}) {{",
                dump_expr(obj)
            )
            .unwrap();
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            writeln!(out, "{pad}}}").unwrap();
        }
        Stmt::Continue => writeln!(out, "{pad}continue").unwrap(),
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
        Stmt::Try {
            body,
            catch,
            finally,
        } => {
            writeln!(out, "{pad}try {{").unwrap();
            for s in body {
                dump_stmt(out, s, depth + 1);
            }
            if let Some((param, catch_body)) = catch {
                match param {
                    Some(p) => writeln!(out, "{pad}}} catch ({p}) {{").unwrap(),
                    None => writeln!(out, "{pad}}} catch {{").unwrap(),
                }
                for s in catch_body {
                    dump_stmt(out, s, depth + 1);
                }
            }
            if let Some(finally_body) = finally {
                writeln!(out, "{pad}}} finally {{").unwrap();
                for s in finally_body {
                    dump_stmt(out, s, depth + 1);
                }
            }
            writeln!(out, "{pad}}}").unwrap();
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
        Expr::Array { elements } => format!(
            "[{}]",
            elements.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::Object { props } => {
            let parts: Vec<String> = props
                .iter()
                .map(|p| {
                    match &p.key {
                        ObjectKey::Spread => format!("...{}", dump_expr(&p.value)),
                        ObjectKey::Static(s) => format!("{s}: {}", dump_expr(&p.value)),
                        ObjectKey::Computed(e) => format!("[{}]: {}", dump_expr(e), dump_expr(&p.value)),
                    }
                })
                .collect();
            format!("{{{}}}", parts.join(", "))
        }
        Expr::New { callee, args } => format!(
            "new {}({})",
            dump_expr(callee),
            args.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::Ternary {
            cond,
            then_branch,
            else_branch,
        } => format!(
            "({} ? {} : {})",
            dump_expr(cond),
            dump_expr(then_branch),
            dump_expr(else_branch)
        ),
        Expr::Spread(inner) => format!("...{}", dump_expr(inner)),
        Expr::Update { op, target, prefix } => {
            let s = match op {
                UpdateOp::Inc => "++",
                UpdateOp::Dec => "--",
            };
            if *prefix {
                format!("({}{})", s, dump_expr(target))
            } else {
                format!("({}{})", dump_expr(target), s)
            }
        }
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
        Expr::Template { quasis, exprs } => {
            let mut s = String::from("`");
            for (i, q) in quasis.iter().enumerate() {
                s.push_str(q);
                if i < exprs.len() {
                    s.push_str("${");
                    s.push_str(&dump_expr(&exprs[i]));
                    s.push('}');
                }
            }
            s.push('`');
            s
        }
        Expr::Sequence { exprs } => format!(
            "({})",
            exprs.iter().map(dump_expr).collect::<Vec<_>>().join(", ")
        ),
        Expr::Assign { target, value } => {
            format!("({} = {})", dump_expr(target), dump_expr(value))
        }
        Expr::CompoundAssign { op, target, value } => {
            format!(
                "({} {}= {})",
                dump_expr(target),
                binop_str(*op),
                dump_expr(value)
            )
        }
    }
}
