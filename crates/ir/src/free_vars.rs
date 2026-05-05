//! Free-variable analysis on tree-IR `FuncLiteral`s.
//!
//! Computes the alphabetically-sorted list of identifier names a
//! function literal needs to capture from its enclosing scope. Used by
//! the SSA lowering's pass 1 to plan `AllocClosure` capture lists for
//! every nested function, and to seed the inner builder with the
//! correct `capture_count`.
//!
//! Bottom-up by design: when analyzing a function `A` that contains a
//! nested function `B`, `A`'s captures must include any name that `B`
//! captures but `A` does not bind itself — otherwise `A` could not
//! satisfy `B`'s `AllocClosure` operands at the textual position where
//! `B` is allocated. Pass 1 walks function literals in post-order so
//! `inner_captures` is already populated for every nested literal by
//! the time we analyze the enclosing one.
//!
//! `FuncBody::PassThrough` bodies are opaque to *passes*, not to
//! *scope analysis*. The free-variable set of a PassThrough body must
//! still be computed correctly so captures flow into the surrounding
//! `AllocClosure` and the outer's bindings stay alive (otherwise DCE
//! drops them and the renamer never receives a pin signal — the
//! emitted PassThrough text then references names that no longer
//! exist, producing `ReferenceError` at runtime). This module
//! delegates PassThrough analysis to `pt_free_vars`, which parses the
//! source slice with OXC and walks the AST scope-aware. See that
//! module's header for the rationale.

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::pt_free_vars;
use crate::ssa::FuncId;
use crate::tree::{Expr, FuncBody, FuncLiteral, Stmt};

/// Free names of `lit`, sorted alphabetically. The list is empty for a
/// function whose body references nothing from outside its own
/// parameters, name, and locals.
pub fn analyze(lit: &FuncLiteral, inner_captures: &HashMap<FuncId, Vec<String>>) -> Vec<String> {
    // PassThrough bodies need a parser-aware walk over the source
    // slice (see module doc). Tree-IR walk is right for Stmts /
    // Expression bodies.
    if let FuncBody::PassThrough(src) = &lit.body {
        return pt_free_vars::analyze_passthrough(
            src,
            &lit.params,
            lit.name.as_deref(),
            lit.is_async,
            lit.is_generator,
        );
    }

    let mut ctx = Ctx {
        free: BTreeSet::new(),
        scopes: Vec::new(),
        inner_captures,
    };
    ctx.enter_function_scope(lit);
    ctx.walk_body(&lit.body);
    ctx.leave_scope();
    ctx.free.into_iter().collect()
}

struct Ctx<'a> {
    free: BTreeSet<String>,
    scopes: Vec<HashSet<String>>,
    inner_captures: &'a HashMap<FuncId, Vec<String>>,
}

impl Ctx<'_> {
    fn enter_function_scope(&mut self, lit: &FuncLiteral) {
        let mut s = HashSet::new();
        // Named function expressions bind their own name within the body.
        if let Some(n) = &lit.name {
            s.insert(n.clone());
        }
        for p in &lit.params {
            s.insert(p.clone());
        }
        self.scopes.push(s);
    }
    fn enter_block_scope(&mut self) {
        self.scopes.push(HashSet::new());
    }
    fn leave_scope(&mut self) {
        self.scopes.pop();
    }
    fn bind(&mut self, name: String) {
        self.scopes
            .last_mut()
            .expect("no active scope")
            .insert(name);
    }
    fn is_bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }
    fn use_name(&mut self, name: &str) {
        if !self.is_bound(name) {
            self.free.insert(name.to_string());
        }
    }

    fn walk_body(&mut self, body: &FuncBody) {
        match body {
            FuncBody::Stmts(stmts) => self.walk_stmts(stmts),
            FuncBody::Expression(e) => self.walk_expr(e),
            // Opaque to this analysis (see module doc).
            FuncBody::PassThrough(_) => {}
        }
    }

    fn walk_stmts(&mut self, stmts: &[Stmt]) {
        // Hoist nested FuncDecl names into the enclosing scope so a
        // statement before the decl can still reference the function by
        // name without it counting as free.
        for s in stmts {
            if let Stmt::FuncDecl(lit) = s
                && let Some(name) = &lit.name
            {
                self.bind(name.clone());
            }
        }
        for s in stmts {
            self.walk_stmt(s);
        }
    }

    fn walk_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, init } => {
                if let Some(e) = init {
                    self.walk_expr(e);
                }
                self.bind(name.clone());
            }
            Stmt::Const { name, init } => {
                self.walk_expr(init);
                self.bind(name.clone());
            }
            Stmt::Assign { target, value } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
            Stmt::ExprStmt(e) => self.walk_expr(e),
            Stmt::Return(e) => {
                if let Some(e) = e {
                    self.walk_expr(e);
                }
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                self.walk_expr(cond);
                self.enter_block_scope();
                self.walk_stmts(then_body);
                self.leave_scope();
                if let Some(eb) = else_body {
                    self.enter_block_scope();
                    self.walk_stmts(eb);
                    self.leave_scope();
                }
            }
            Stmt::While { cond, body } => {
                self.walk_expr(cond);
                self.enter_block_scope();
                self.walk_stmts(body);
                self.leave_scope();
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                // Init's bindings (e.g. `let i = 0`) live for the whole
                // for-loop region and are visible to cond/step/body.
                self.enter_block_scope();
                if let Some(init) = init {
                    self.walk_stmt(init);
                }
                if let Some(cond) = cond {
                    self.walk_expr(cond);
                }
                self.walk_stmts(body);
                if let Some(step) = step {
                    self.walk_expr(step);
                }
                self.leave_scope();
            }
            Stmt::ForOf {
                binding,
                iterable,
                body,
                ..
            } => {
                // Iterable is evaluated in the enclosing scope.
                self.walk_expr(iterable);
                // The binding variable is introduced in a fresh scope
                // that covers the body.
                self.enter_block_scope();
                self.bind(binding.clone());
                self.walk_stmts(body);
                self.leave_scope();
            }
            Stmt::ForIn {
                binding,
                obj,
                body,
                ..
            } => {
                self.walk_expr(obj);
                self.enter_block_scope();
                self.bind(binding.clone());
                self.walk_stmts(body);
                self.leave_scope();
            }
            Stmt::Continue => {}
            Stmt::Block(stmts) => {
                self.enter_block_scope();
                self.walk_stmts(stmts);
                self.leave_scope();
            }
            Stmt::FuncDecl(lit) => {
                // Name is already hoisted by the surrounding `walk_stmts`.
                // Don't descend — the inner literal has its own free-var
                // list. Instead, propagate its captures: anything it
                // captures that isn't bound in the enclosing scopes
                // becomes a free var of the current function.
                self.absorb_inner_captures(lit);
            }
            Stmt::Try {
                body,
                catch,
                finally,
            } => {
                self.enter_block_scope();
                self.walk_stmts(body);
                self.leave_scope();
                if let Some((param, catch_body)) = catch {
                    self.enter_block_scope();
                    if let Some(name) = param {
                        self.bind(name.clone());
                    }
                    self.walk_stmts(catch_body);
                    self.leave_scope();
                }
                if let Some(finally_body) = finally {
                    self.enter_block_scope();
                    self.walk_stmts(finally_body);
                    self.leave_scope();
                }
            }
        }
    }

    fn walk_expr(&mut self, e: &Expr) {
        match e {
            Expr::Number(_) | Expr::String(_) | Expr::Bool(_) | Expr::Null | Expr::Undefined => {}
            Expr::Ident(name) => self.use_name(name),
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            Expr::Unary { operand, .. } => self.walk_expr(operand),
            Expr::Call { callee, args } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Member { obj, .. } => self.walk_expr(obj),
            Expr::Index { obj, index } => {
                self.walk_expr(obj);
                self.walk_expr(index);
            }
            Expr::Func(lit) => self.absorb_inner_captures(lit),
            Expr::Array { elements } => {
                for el in elements {
                    self.walk_expr(el);
                }
            }
            Expr::Object { props } => {
                for p in props {
                    if let crate::tree::ObjectKey::Computed(e) = &p.key {
                        self.walk_expr(e);
                    }
                    self.walk_expr(&p.value);
                }
            }
            Expr::New { callee, args } => {
                self.walk_expr(callee);
                for a in args {
                    self.walk_expr(a);
                }
            }
            Expr::Ternary {
                cond,
                then_branch,
                else_branch,
            } => {
                self.walk_expr(cond);
                self.walk_expr(then_branch);
                self.walk_expr(else_branch);
            }
            Expr::Update { target, .. } => self.walk_expr(target),
            Expr::Spread(inner) => self.walk_expr(inner),
            Expr::Template { exprs, .. } => {
                for e in exprs {
                    self.walk_expr(e);
                }
            }
            Expr::Sequence { exprs } => {
                for e in exprs {
                    self.walk_expr(e);
                }
            }
            Expr::Assign { target, value } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
        }
    }

    fn absorb_inner_captures(&mut self, lit: &FuncLiteral) {
        let Some(fid) = lit.func_id.get() else {
            // Pass 1 has not yet stamped this literal — caller is
            // analyzing in the wrong order. Treat as opaque (no
            // propagation) to avoid panicking, but this is a bug.
            debug_assert!(
                false,
                "free_vars::analyze called before pass 1 stamped FuncId"
            );
            return;
        };
        if let Some(caps) = self.inner_captures.get(&fid) {
            for name in caps {
                self.use_name(name);
            }
        }
    }
}
