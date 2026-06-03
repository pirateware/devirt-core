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
        Statement::FunctionDeclaration(f) => {
            f.id.as_ref()
                .is_none_or(|id| !is_removable_symbol(id.symbol_id(), scoping))
        }
        // A bare literal statement (`80214130;`, `true;`, `null;`) computes a
        // value that is immediately discarded with no side effect — pure
        // obfuscation noise (often a spent opaque-predicate constant). Drop it.
        // Strings are excluded: a leading string can be a directive-like marker
        // and removing it is both low-value and potentially surprising.
        Statement::ExpressionStatement(es) => !is_dead_literal(&es.expression),
        _ => true,
    }
}

/// A literal expression whose evaluation has no side effect, so using it as a
/// statement does nothing. Excludes string literals (directive concern).
fn is_dead_literal(expr: &oxc_ast::ast::Expression<'_>) -> bool {
    use oxc_ast::ast::Expression as E;
    matches!(
        expr,
        E::NumericLiteral(_) | E::BooleanLiteral(_) | E::NullLiteral(_) | E::BigIntLiteral(_)
    )
}

/// A binding may be removed if it is unused anywhere in the program. We delete
/// root-scope bindings too: the deobfuscator's scope is the file it is given, so
/// a binding unreferenced *within that file* is dead. A top-level `var` in a
/// script does become a global property, but whether some *other* script reads
/// that property is the input's concern, not ours — protecting it here only
/// keeps spent obfuscator scaffolding (string arrays, decoder fns) alive.
/// Soundness for the value itself is preserved separately: a declarator is only
/// dropped when its initializer is pure (see `keep_declarator`).
fn is_removable_symbol(symbol: oxc_syntax::symbol::SymbolId, scoping: &Scoping) -> bool {
    scoping.symbol_is_unused(symbol)
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
    let pure = d.init.as_ref().is_none_or(|e| is_pure_simple(e));
    !(is_removable_symbol(bid.symbol_id(), scoping) && pure)
}
