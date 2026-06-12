//! Unwrap a single-iteration loop wrapper.
//!
//! Obfuscators wrap straight-line code in `while (true) { … ; break; }` (or
//! `for (;;) { … ; break; }`) to look like control flow. When the loop's condition
//! is a constant truthy value and the body's only exit *for this loop* is a single
//! trailing unconditional `break`, the loop runs its body exactly once — so it is
//! equivalent to the body itself. We replace the loop with `{ … }` (the body minus
//! the trailing break); a transparent block is then dissolved by `block-flatten`.
//!
//! Soundness — the rewrite preserves behavior only when the body cannot loop back
//! or exit early *through this loop*:
//!   * the test is a constant truthy value (or absent, `for (;;)`), so the loop is
//!     entered and never re-tested into an exit;
//!   * the last body statement is an unlabeled `break` (the lone, unconditional
//!     exit);
//!   * nowhere else in the body is there an unlabeled `break` not enclosed by a
//!     nested loop/`switch`, an unlabeled `continue` not enclosed by a nested loop,
//!     or any labeled `break`/`continue` (which could target this loop). Those would
//!     mean the body can restart or exit at a different point.
//!
//! Anything else is left untouched. The body is kept wrapped in a block so a
//! block-scoped `let`/`const`/`class` cannot leak into the parent scope.

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{Expression, ForStatement, Program, Statement, WhileStatement};
use oxc_ast::AstBuilder;
use oxc_ast_visit::{walk, Visit};
use oxc_span::SPAN;

use crate::pass::Pass;

#[derive(Default)]
pub struct SingleIterLoop {
    changed: bool,
}

impl Pass for SingleIterLoop {
    fn name(&self) -> &'static str {
        "single-iter-loop"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let ast = AstBuilder::new(allocator);
        self.walk_list(&mut program.body, ast);
        self.changed
    }
}

impl SingleIterLoop {
    fn walk_list<'a>(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>, ast: AstBuilder<'a>) {
        for stmt in stmts.iter_mut() {
            self.walk_stmt(stmt, ast);
        }
    }

    fn walk_stmt<'a>(&mut self, stmt: &mut Statement<'a>, ast: AstBuilder<'a>) {
        // Recurse first so nested loops are simplified bottom-up.
        match stmt {
            Statement::BlockStatement(b) => self.walk_list(&mut b.body, ast),
            Statement::FunctionDeclaration(f) => {
                if let Some(body) = &mut f.body {
                    self.walk_list(&mut body.statements, ast);
                }
            }
            Statement::IfStatement(i) => {
                self.walk_stmt(&mut i.consequent, ast);
                if let Some(alt) = &mut i.alternate {
                    self.walk_stmt(alt, ast);
                }
            }
            Statement::ForStatement(f) => self.walk_stmt(&mut f.body, ast),
            Statement::ForInStatement(f) => self.walk_stmt(&mut f.body, ast),
            Statement::ForOfStatement(f) => self.walk_stmt(&mut f.body, ast),
            Statement::WhileStatement(w) => self.walk_stmt(&mut w.body, ast),
            Statement::DoWhileStatement(d) => self.walk_stmt(&mut d.body, ast),
            Statement::LabeledStatement(l) => self.walk_stmt(&mut l.body, ast),
            Statement::TryStatement(t) => {
                self.walk_list(&mut t.block.body, ast);
                if let Some(h) = &mut t.handler {
                    self.walk_list(&mut h.body.body, ast);
                }
                if let Some(f) = &mut t.finalizer {
                    self.walk_list(&mut f.body, ast);
                }
            }
            Statement::SwitchStatement(sw) => {
                for case in &mut sw.cases {
                    self.walk_list(&mut case.consequent, ast);
                }
            }
            _ => {}
        }
        // Then try to unwrap this statement if it is a single-iteration loop.
        if let Some(block) = self.try_unwrap(stmt, ast) {
            *stmt = block;
            self.changed = true;
        }
    }

    /// If `stmt` is a single-iteration loop, return the replacement block.
    fn try_unwrap<'a>(
        &self,
        stmt: &Statement<'a>,
        ast: AstBuilder<'a>,
    ) -> Option<Statement<'a>> {
        let body = match stmt {
            Statement::WhileStatement(w) if truthy_const(Some(&w.test)) => loop_block(w),
            Statement::ForStatement(f) if for_is_infinite(f) => for_block(f),
            _ => return None,
        }?;
        // The body must *definitely* exit on the first iteration: its last statement
        // is a bare `break;` (which we strip) or a `return`/`throw` (which we keep).
        // And nothing else in the body may jump out of (or restart) this loop.
        let (last, head) = body.split_last()?;
        let strip_last = is_bare_break(last);
        if !strip_last && !is_terminator(last) {
            return None;
        }
        if body_has_loop_jump(head) {
            return None;
        }
        let mut kept = ast.vec();
        for s in head {
            kept.push(s.clone_in(ast.allocator));
        }
        if !strip_last {
            kept.push(last.clone_in(ast.allocator)); // keep the return/throw
        }
        Some(ast.statement_block(SPAN, kept))
    }
}

// --- shape helpers ---------------------------------------------------------

fn truthy_const(test: Option<&Expression<'_>>) -> bool {
    match test {
        None => true,
        Some(Expression::BooleanLiteral(b)) => b.value,
        Some(Expression::NumericLiteral(n)) => n.value != 0.0,
        Some(Expression::StringLiteral(s)) => !s.value.is_empty(),
        _ => false,
    }
}

fn for_is_infinite(f: &ForStatement<'_>) -> bool {
    // `for (init?; <truthy?>; update?)` with no state we model is single-iteration
    // only when nothing re-tests into an exit and there is no update that matters;
    // we conservatively require no init and no update (a bare `for (;;)`/`for(;1;)`).
    f.init.is_none() && f.update.is_none() && truthy_const(f.test.as_ref())
}

fn loop_block<'b, 'a>(w: &'b WhileStatement<'a>) -> Option<&'b [Statement<'a>]> {
    match &w.body {
        Statement::BlockStatement(b) => Some(&b.body),
        _ => None,
    }
}

fn for_block<'b, 'a>(f: &'b ForStatement<'a>) -> Option<&'b [Statement<'a>]> {
    match &f.body {
        Statement::BlockStatement(b) => Some(&b.body),
        _ => None,
    }
}

fn is_bare_break(stmt: &Statement<'_>) -> bool {
    matches!(stmt, Statement::BreakStatement(b) if b.label.is_none())
}

fn is_terminator(stmt: &Statement<'_>) -> bool {
    matches!(
        stmt,
        Statement::ReturnStatement(_) | Statement::ThrowStatement(_)
    )
}

/// Does `stmts` contain a jump that targets *this* loop — an unlabeled `break` not
/// inside a nested loop/`switch`, an unlabeled `continue` not inside a nested loop,
/// or any labeled `break`/`continue` (conservatively assumed to target this loop)?
fn body_has_loop_jump(stmts: &[Statement<'_>]) -> bool {
    let mut s = JumpScan {
        loop_depth: 0,
        switch_depth: 0,
        found: false,
    };
    for st in stmts {
        s.visit_statement(st);
    }
    s.found
}

struct JumpScan {
    loop_depth: u32,
    switch_depth: u32,
    found: bool,
}

impl<'a> Visit<'a> for JumpScan {
    fn visit_for_statement(&mut self, it: &ForStatement<'a>) {
        self.loop_depth += 1;
        self.switch_depth += 1;
        walk::walk_for_statement(self, it);
        self.loop_depth -= 1;
        self.switch_depth -= 1;
    }
    fn visit_for_in_statement(&mut self, it: &oxc_ast::ast::ForInStatement<'a>) {
        self.loop_depth += 1;
        self.switch_depth += 1;
        walk::walk_for_in_statement(self, it);
        self.loop_depth -= 1;
        self.switch_depth -= 1;
    }
    fn visit_for_of_statement(&mut self, it: &oxc_ast::ast::ForOfStatement<'a>) {
        self.loop_depth += 1;
        self.switch_depth += 1;
        walk::walk_for_of_statement(self, it);
        self.loop_depth -= 1;
        self.switch_depth -= 1;
    }
    fn visit_while_statement(&mut self, it: &WhileStatement<'a>) {
        self.loop_depth += 1;
        self.switch_depth += 1;
        walk::walk_while_statement(self, it);
        self.loop_depth -= 1;
        self.switch_depth -= 1;
    }
    fn visit_do_while_statement(&mut self, it: &oxc_ast::ast::DoWhileStatement<'a>) {
        self.loop_depth += 1;
        self.switch_depth += 1;
        walk::walk_do_while_statement(self, it);
        self.loop_depth -= 1;
        self.switch_depth -= 1;
    }
    fn visit_switch_statement(&mut self, it: &oxc_ast::ast::SwitchStatement<'a>) {
        // A `switch` only catches `break`, not `continue`.
        self.switch_depth += 1;
        walk::walk_switch_statement(self, it);
        self.switch_depth -= 1;
    }
    // A nested function is its own jump scope; `break`/`continue` cannot cross it.
    fn visit_function(&mut self, _it: &oxc_ast::ast::Function<'a>, _flags: oxc_semantic::ScopeFlags) {
    }
    fn visit_arrow_function_expression(&mut self, _it: &oxc_ast::ast::ArrowFunctionExpression<'a>) {}

    fn visit_break_statement(&mut self, it: &oxc_ast::ast::BreakStatement<'a>) {
        if it.label.is_some() || self.switch_depth == 0 {
            self.found = true;
        }
    }
    fn visit_continue_statement(&mut self, it: &oxc_ast::ast::ContinueStatement<'a>) {
        if it.label.is_some() || self.loop_depth == 0 {
            self.found = true;
        }
    }
}
