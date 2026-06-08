//! Flatten a transparent bare block into its parent statement list.
//!
//! A standalone `{ … }` block is a no-op wrapper unless it introduces a
//! block-scoped binding (`let`/`const`/`class`, or a `function` declaration whose
//! scope a block changes). Such blocks appear as debris after a constant-condition
//! `if` collapses to its taken branch (`const-fold` replaces the whole `if` with
//! the branch's block) and after similar branch rewrites. Splicing a transparent
//! block's statements straight into the enclosing list reads better and — the real
//! win — lets downstream passes see through it: once `{ return x; }` becomes a bare
//! `return x;`, DCE's unreachable-tail removal can drop the dead code that followed
//! the block.
//!
//! Only statement-*list* members are unwrapped. The bodies of `if`/loops/functions
//! are single `Statement`s, not list elements, so they are never touched (we do not
//! want to dissolve `if (c) { … }` into the parent). A labeled block (`L: { … }`)
//! is the body of a `LabeledStatement`, also not a list member, so it stays intact.
//!
//! Like `unflatten_cfg`, this is a hand-written recursion rather than `traverse_mut`:
//! it only ever moves existing nodes up a level, and the manual walk keeps it
//! independent of the traverser's scope bookkeeping.

use oxc_allocator::{Allocator, Vec as ArenaVec};
use oxc_ast::ast::{Expression, Program, Statement, VariableDeclarationKind};

use crate::pass::Pass;

#[derive(Default)]
pub struct BlockFlatten {
    changed: bool,
}

impl Pass for BlockFlatten {
    fn name(&self) -> &'static str {
        "block-flatten"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        self.lists(&mut program.body, allocator);
        self.changed
    }
}

impl BlockFlatten {
    /// Recurse a statement list: descend into existing children first, then unwrap
    /// any transparent bare block in this list (moving its statements up in place).
    fn lists<'a>(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>, alloc: &'a Allocator) {
        for stmt in stmts.iter_mut() {
            self.recurse_stmt(stmt, alloc);
        }
        if !stmts.iter().any(is_transparent_block) {
            return;
        }
        let old = std::mem::replace(stmts, ArenaVec::new_in(alloc));
        for s in old {
            match s {
                Statement::BlockStatement(b) if block_is_transparent(&b.body) => {
                    for inner in b.unbox().body {
                        stmts.push(inner);
                    }
                    self.changed = true;
                }
                other => stmts.push(other),
            }
        }
    }

    /// Descend into every statement list reachable from `stmt`. Mirrors
    /// `unflatten_cfg`'s recursion; a container not handled here is simply not
    /// searched (sound, just no transform there).
    fn recurse_stmt<'a>(&mut self, stmt: &mut Statement<'a>, alloc: &'a Allocator) {
        match stmt {
            Statement::BlockStatement(b) => self.lists(&mut b.body, alloc),
            Statement::FunctionDeclaration(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements, alloc);
                }
            }
            Statement::VariableDeclaration(vd) => {
                for d in &mut vd.declarations {
                    if let Some(init) = &mut d.init {
                        self.recurse_expr_fn(init, alloc);
                    }
                }
            }
            Statement::ExpressionStatement(es) => self.recurse_expr_fn(&mut es.expression, alloc),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &mut r.argument {
                    self.recurse_expr_fn(arg, alloc);
                }
            }
            Statement::IfStatement(i) => {
                self.recurse_stmt(&mut i.consequent, alloc);
                if let Some(alt) = &mut i.alternate {
                    self.recurse_stmt(alt, alloc);
                }
            }
            Statement::ForStatement(f) => self.recurse_stmt(&mut f.body, alloc),
            Statement::ForInStatement(f) => self.recurse_stmt(&mut f.body, alloc),
            Statement::ForOfStatement(f) => self.recurse_stmt(&mut f.body, alloc),
            Statement::WhileStatement(w) => self.recurse_stmt(&mut w.body, alloc),
            Statement::DoWhileStatement(d) => self.recurse_stmt(&mut d.body, alloc),
            Statement::LabeledStatement(l) => self.recurse_stmt(&mut l.body, alloc),
            Statement::TryStatement(t) => {
                self.lists(&mut t.block.body, alloc);
                if let Some(h) = &mut t.handler {
                    self.lists(&mut h.body.body, alloc);
                }
                if let Some(f) = &mut t.finalizer {
                    self.lists(&mut f.body, alloc);
                }
            }
            Statement::SwitchStatement(sw) => {
                for case in &mut sw.cases {
                    self.lists(&mut case.consequent, alloc);
                }
            }
            _ => {}
        }
    }

    fn recurse_expr_fn<'a>(&mut self, expr: &mut Expression<'a>, alloc: &'a Allocator) {
        match expr {
            Expression::FunctionExpression(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements, alloc);
                }
            }
            Expression::ArrowFunctionExpression(f) => {
                self.lists(&mut f.body.statements, alloc);
            }
            Expression::CallExpression(c) => {
                self.recurse_expr_fn(&mut c.callee, alloc);
                for arg in &mut c.arguments {
                    if let Some(e) = arg.as_expression_mut() {
                        self.recurse_expr_fn(e, alloc);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => self.recurse_expr_fn(&mut p.expression, alloc),
            Expression::UnaryExpression(u) => self.recurse_expr_fn(&mut u.argument, alloc),
            Expression::SequenceExpression(s) => {
                for e in &mut s.expressions {
                    self.recurse_expr_fn(e, alloc);
                }
            }
            Expression::AssignmentExpression(a) => self.recurse_expr_fn(&mut a.right, alloc),
            _ => {}
        }
    }
}

/// A `{ … }` block statement with no block-scoped declarations.
fn is_transparent_block(s: &Statement<'_>) -> bool {
    matches!(s, Statement::BlockStatement(b) if block_is_transparent(&b.body))
}

/// True when a block's body introduces no binding whose scope is the block itself
/// (`let`/`const`/`class`, or a `function` declaration). `var` hoists past the
/// block regardless, so a `var`-only block is safe to dissolve.
fn block_is_transparent(body: &[Statement<'_>]) -> bool {
    !body.iter().any(|st| {
        matches!(
            st,
            Statement::ClassDeclaration(_) | Statement::FunctionDeclaration(_)
        ) || matches!(st, Statement::VariableDeclaration(vd)
            if vd.kind != VariableDeclarationKind::Var)
    })
}
