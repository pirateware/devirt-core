//! CFG-based control-flow recovery for flattened dispatchers (Plan P2).
//!
//! The linear `unflatten` collapses *acyclic, deterministic* `while`/`switch`
//! dispatchers. It declines (soundly) on **looping** dispatchers — a state
//! revisited via a conditional transition. This pass recovers the simplest such
//! loop: a single state that conditionally transitions back to itself or exits,
//! which is a `do { … } while (cond)`.
//!
//! It engages only where the linear form bailed (the recovered graph has a back
//! edge), so the two passes never double-transform. The recovery is exact by
//! construction (see `src/ir/cff.rs`): the recovered graph *is* the state machine,
//! and the structuring re-emits the same statements under the same condition.
//! Anything not matching the supported shape is left untouched (sound no-op).
//!
//! Like `unflatten_param`, the rewrite is a **hand-written recursion**, not
//! `traverse_mut`: the structured `do/while` is a freshly-built node with no
//! `scope_id`, and oxc's walker panics if it descends into one. The recursion
//! descends into each list's *existing* children first, then performs the
//! replacement, which it never re-visits.

use oxc_allocator::{Allocator, Vec as ArenaVec};
use oxc_ast::ast::{Expression, Program, Statement};
use oxc_ast::AstBuilder;

use super::unflatten::{detect_loop, mentions, state_init};
use crate::ir::cff;
use crate::pass::Pass;

pub struct UnflattenCfg;

impl Pass for UnflattenCfg {
    fn name(&self) -> &'static str {
        "unflatten-cfg"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let ast = AstBuilder::new(allocator);
        let mut w = Walker {
            ast,
            changed: false,
        };
        w.lists(&mut program.body);
        w.changed
    }
}

struct Walker<'a> {
    ast: AstBuilder<'a>,
    changed: bool,
}

impl<'a> Walker<'a> {
    /// Recurse a statement list: descend into existing children first, then try to
    /// recover a looping dispatcher in this list. The replacement (a freshly-built
    /// `do/while`) is never re-visited.
    fn lists(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>) {
        for stmt in stmts.iter_mut() {
            self.recurse_stmt(stmt);
        }
        self.try_recover(stmts);
    }

    fn try_recover(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>) {
        let mut hit: Option<(usize, usize, Vec<Statement<'a>>)> = None;
        for i in 0..stmts.len() {
            let Some((s_name, entry)) = state_init(&stmts[i]) else {
                continue;
            };
            // Dispatch-only: `s` referenced in exactly one OTHER statement (the loop).
            let others: Vec<usize> = (0..stmts.len())
                .filter(|&k| k != i && mentions(&stmts[k], &s_name))
                .collect();
            if others.len() != 1 {
                continue;
            }
            let j = others[0];
            if j <= i {
                continue;
            }
            let Some((switch, _kind, test)) = detect_loop(&stmts[j], &s_name) else {
                continue;
            };
            // Recover the state graph, using the loop test to classify exits.
            let continues = |state: i64| test.continues(state);
            let Some(cfg) = cff::build(switch, &s_name, entry, &continues) else {
                continue;
            };
            // Try the specialized self-loop → do/while first (it reads nicest), then
            // fall back to the general relooper for arbitrary reducible branch/loop
            // graphs the linear form declined on. Either declines soundly (→ leave
            // the dispatcher) on shapes it cannot place.
            let seq = cff::structure(switch, &s_name, &cfg, self.ast)
                .or_else(|| cff::structure_general(switch, &s_name, &cfg, self.ast));
            let Some(seq) = seq else {
                continue;
            };
            hit = Some((i, j, seq));
            break;
        }

        let Some((i, j, seq)) = hit else {
            return;
        };
        // Splice: drop the state initializer (i) and replace the dispatcher (j)
        // with the structured sequence; keep everything else in order.
        let old = std::mem::replace(stmts, self.ast.vec());
        let mut seq = Some(seq);
        for (k, s) in old.into_iter().enumerate() {
            if k == i {
                continue;
            }
            if k == j {
                for st in seq.take().unwrap() {
                    stmts.push(st);
                }
                continue;
            }
            stmts.push(s);
        }
        self.changed = true;
    }

    /// Descend into every statement list reachable from `stmt` (best-effort; a
    /// container not handled here simply isn't searched — sound, just no transform
    /// there). Mirrors `unflatten_param`'s recursion.
    fn recurse_stmt(&mut self, stmt: &mut Statement<'a>) {
        match stmt {
            Statement::BlockStatement(b) => self.lists(&mut b.body),
            Statement::FunctionDeclaration(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements);
                }
            }
            Statement::VariableDeclaration(vd) => {
                for d in &mut vd.declarations {
                    if let Some(init) = &mut d.init {
                        self.recurse_expr_fn(init);
                    }
                }
            }
            Statement::ExpressionStatement(es) => self.recurse_expr_fn(&mut es.expression),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &mut r.argument {
                    self.recurse_expr_fn(arg);
                }
            }
            Statement::IfStatement(i) => {
                self.recurse_stmt(&mut i.consequent);
                if let Some(alt) = &mut i.alternate {
                    self.recurse_stmt(alt);
                }
            }
            Statement::ForStatement(f) => self.recurse_stmt(&mut f.body),
            Statement::ForInStatement(f) => self.recurse_stmt(&mut f.body),
            Statement::ForOfStatement(f) => self.recurse_stmt(&mut f.body),
            Statement::WhileStatement(w) => self.recurse_stmt(&mut w.body),
            Statement::DoWhileStatement(d) => self.recurse_stmt(&mut d.body),
            Statement::LabeledStatement(l) => self.recurse_stmt(&mut l.body),
            Statement::TryStatement(t) => {
                self.lists(&mut t.block.body);
                if let Some(h) = &mut t.handler {
                    self.lists(&mut h.body.body);
                }
                if let Some(f) = &mut t.finalizer {
                    self.lists(&mut f.body);
                }
            }
            Statement::SwitchStatement(sw) => {
                for case in &mut sw.cases {
                    self.lists(&mut case.consequent);
                }
            }
            _ => {}
        }
    }

    fn recurse_expr_fn(&mut self, expr: &mut Expression<'a>) {
        match expr {
            Expression::FunctionExpression(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements);
                }
            }
            Expression::ArrowFunctionExpression(f) => {
                self.lists(&mut f.body.statements);
            }
            Expression::CallExpression(c) => {
                self.recurse_expr_fn(&mut c.callee);
                for arg in &mut c.arguments {
                    if let Some(e) = arg.as_expression_mut() {
                        self.recurse_expr_fn(e);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => self.recurse_expr_fn(&mut p.expression),
            Expression::UnaryExpression(u) => self.recurse_expr_fn(&mut u.argument),
            Expression::SequenceExpression(s) => {
                for e in &mut s.expressions {
                    self.recurse_expr_fn(e);
                }
            }
            Expression::AssignmentExpression(a) => self.recurse_expr_fn(&mut a.right),
            _ => {}
        }
    }
}
