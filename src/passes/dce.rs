//! Dead-code elimination.
//!
//! Operates on statement lists (`enter_statements`) and removes:
//!   1. unreachable statements after a terminator (return/throw/break/continue),
//!      while *preserving* hoisted `var`/function declarations from the dead tail;
//!   2. variable declarators whose binding is unused and whose initializer is
//!      pure (so dropping it loses nothing);
//!   3. now-empty `VariableDeclaration`s and stray `EmptyStatement`s.

use oxc_allocator::{Allocator, Vec as ArenaVec};
use oxc_ast::ast::{BindingPattern, Program, Statement, VariableDeclarator};
use oxc_semantic::Scoping;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::{build_scoping, is_pure_simple};

#[derive(Default)]
pub struct Dce {
    changed: bool,
}

impl Pass for Dce {
    fn name(&self) -> &'static str {
        "dce"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let scoping = build_scoping(program);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

impl<'a> Traverse<'a, ()> for Dce {
    fn enter_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let before = stmts.len();

        // 1. Drop unreachable tail, keeping hoisted declarations.
        if let Some(term) = stmts.iter().position(is_terminator) {
            if term + 1 < stmts.len() {
                let mut idx = 0usize;
                stmts.retain(|s| {
                    let keep = idx <= term || is_hoisted(s);
                    idx += 1;
                    keep
                });
            }
        }

        // 2. Prune unused, pure declarators.
        let scoping: &Scoping = ctx.scoping();
        for s in stmts.iter_mut() {
            if let Statement::VariableDeclaration(vd) = s {
                vd.declarations.retain(|d| keep_declarator(d, scoping));
            }
        }

        // 3. Remove emptied declarations, bare empty statements, and unused
        //    function declarations (e.g. decoders left dead after lifting).
        stmts.retain(|s| keep_statement(s, scoping));

        if stmts.len() != before {
            self.changed = true;
        }
    }
}

fn keep_statement(s: &Statement<'_>, scoping: &Scoping) -> bool {
    match s {
        Statement::EmptyStatement(_) => false,
        Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
        Statement::FunctionDeclaration(f) => f
            .id
            .as_ref()
            .map_or(true, |id| !is_removable_symbol(id.symbol_id(), scoping)),
        _ => true,
    }
}

/// A binding may be removed only if it is unused AND not declared in the root
/// (program/global) scope. Top-level bindings in a script become global
/// properties — observable output we must never delete, even if nothing in this
/// file references them (they may be the program's public entry point).
fn is_removable_symbol(symbol: oxc_syntax::symbol::SymbolId, scoping: &Scoping) -> bool {
    scoping.symbol_is_unused(symbol) && scoping.symbol_scope_id(symbol) != scoping.root_scope_id()
}

fn is_terminator(s: &Statement<'_>) -> bool {
    matches!(
        s,
        Statement::ReturnStatement(_)
            | Statement::ThrowStatement(_)
            | Statement::BreakStatement(_)
            | Statement::ContinueStatement(_)
    )
}

fn is_hoisted(s: &Statement<'_>) -> bool {
    matches!(
        s,
        Statement::VariableDeclaration(_) | Statement::FunctionDeclaration(_)
    )
}

fn keep_declarator(d: &VariableDeclarator<'_>, scoping: &Scoping) -> bool {
    let BindingPattern::BindingIdentifier(bid) = &d.id else {
        return true; // patterns: keep (destructuring may have effects)
    };
    let pure = d.init.as_ref().map_or(true, |e| is_pure_simple(e));
    !(is_removable_symbol(bid.symbol_id(), scoping) && pure)
}

