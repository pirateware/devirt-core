//! Flatten the `else` of an `if` whose consequent always exits.
//!
//! Obfuscators (and minifiers) nest deeply: `if (c) { return a; } else { B }`.
//! When the consequent unconditionally terminates (its last statement is a
//! `return`/`throw`/`break`/`continue`), the `else` is redundant — control can
//! only reach `B` when `c` was false, exactly as if `B` followed the `if`:
//!
//! ```js
//! if (c) { return a; }
//! B
//! ```
//!
//! This peels one nesting level per round and compounds: `else if` ladders
//! unwind into flat guard sequences.
//!
//! Soundness: moving the else body into the parent statement list widens the
//! scope of any *block-scoped* declaration it contains (`let`/`const`/`class`/
//! block `function`). We therefore only flatten when the else body has no such
//! declaration — `var`/expressions/control flow are function-scoped or
//! scopeless and move freely.

use oxc_allocator::{Allocator, Vec as ArenaVec};
use oxc_ast::ast::{Program, Statement};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct FlattenElse {
    changed: bool,
}

impl Pass for FlattenElse {
    fn name(&self) -> &'static str {
        "flatten-else"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let scoping = build_scoping(program);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

/// Does this statement complete abruptly for sure — i.e. nothing after it in the
/// same block is reachable? A bare terminator, or a block ending in one.
fn always_exits(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::ReturnStatement(_)
        | Statement::ThrowStatement(_)
        | Statement::BreakStatement(_)
        | Statement::ContinueStatement(_) => true,
        Statement::BlockStatement(b) => b.body.last().is_some_and(always_exits),
        _ => false,
    }
}

/// Whether a statement is a block-scoped declaration whose scope would change if
/// hoisted out of its block into the parent statement list.
fn is_block_scoped_decl(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::FunctionDeclaration(_) | Statement::ClassDeclaration(_) => true,
        Statement::VariableDeclaration(v) => !v.kind.is_var(),
        _ => false,
    }
}

/// The else body is safe to hoist iff none of its top-level statements is a
/// block-scoped declaration. (A single non-block else statement that is such a
/// declaration is also rejected.)
fn else_body_hoistable(alt: &Statement<'_>) -> bool {
    match alt {
        Statement::BlockStatement(b) => !b.body.iter().any(is_block_scoped_decl),
        other => !is_block_scoped_decl(other),
    }
}

impl<'a> Traverse<'a, ()> for FlattenElse {
    fn enter_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let any = stmts.iter().any(|s| match s {
            Statement::IfStatement(i) => {
                i.alternate.is_some()
                    && always_exits(&i.consequent)
                    && i.alternate.as_ref().is_some_and(else_body_hoistable)
            }
            _ => false,
        });
        if !any {
            return;
        }

        let old = std::mem::replace(stmts, ctx.ast.vec_with_capacity(stmts.len()));
        for stmt in old {
            match stmt {
                Statement::IfStatement(mut ifs)
                    if ifs.alternate.is_some()
                        && always_exits(&ifs.consequent)
                        && ifs.alternate.as_ref().is_some_and(else_body_hoistable) =>
                {
                    // Detach the else, keep the now-else-less `if`, then append
                    // the else body (its statements, if a block) after it.
                    let alt = ifs.alternate.take().unwrap();
                    stmts.push(Statement::IfStatement(ifs));
                    match alt {
                        Statement::BlockStatement(mut b) => {
                            let body = std::mem::replace(&mut b.body, ctx.ast.vec());
                            for s in body {
                                stmts.push(s);
                            }
                        }
                        other => stmts.push(other),
                    }
                    self.changed = true;
                }
                other => stmts.push(other),
            }
        }
    }
}
