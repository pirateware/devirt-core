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
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_traverse::{traverse_mut, Ancestor, Traverse, TraverseCtx};

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
        //    Exception: an *expression-bodied* arrow (`() => 7`) stores its body
        //    as a one-statement list whose single `ExpressionStatement` is the
        //    implicit return — dropping it as a "dead literal" would empty the
        //    arrow to the invalid `() =>`. Skip the prune for that list; the value
        //    is returned, not discarded.
        if !is_expression_arrow_body(ctx) {
            stmts.retain(|s| keep_statement(s, scoping));
        }

        if stmts.len() != before {
            self.changed = true;
        }
    }
}

/// True when the statement list currently being visited is the body of an
/// expression-bodied arrow (`() => expr`), which oxc represents as a single-
/// statement `FunctionBody` whose lone `ExpressionStatement` is the implicit
/// return. The list's parent is `FunctionBodyStatements`; its grandparent is the
/// arrow, carrying the `expression` flag.
fn is_expression_arrow_body(ctx: &TraverseCtx<'_, ()>) -> bool {
    matches!(ctx.ancestor(1), Ancestor::ArrowFunctionExpressionBody(a) if *a.expression())
}

fn keep_statement(s: &Statement<'_>, scoping: &Scoping) -> bool {
    match s {
        Statement::EmptyStatement(_) => false,
        Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
        Statement::FunctionDeclaration(f) => {
            f.id.as_ref()
                .is_none_or(|id| !fn_decl_is_dead(id.symbol_id(), f, scoping))
        }
        // A bare literal statement (`80214130;`, `true;`, `null;`, `"h";`)
        // computes a value that is immediately discarded with no side effect —
        // pure obfuscation noise (a spent opaque-predicate constant, or the
        // pure-RHS debris dead-store leaves when it strips `s = "h"` down to its
        // value). Drop it.
        //
        // String literals are safe to drop here despite the directive-prologue
        // rule: oxc parses a body's directive prologue into a *separate*
        // `directives` field, never into the statement list. So a `StringLiteral`
        // expression statement that reaches this list is provably not a directive
        // (e.g. `"use strict"` survives because it lives in `body.directives`,
        // which DCE never touches) — it is just inert noise.
        Statement::ExpressionStatement(es) => !is_dead_literal(&es.expression),
        _ => true,
    }
}

/// A literal expression whose evaluation has no side effect, so using it as a
/// statement does nothing. String literals are included: a directive prologue is
/// stored in the body's separate `directives` field, so any string-literal
/// statement in the statement list is non-directive dead code (see
/// `keep_statement`).
fn is_dead_literal(expr: &oxc_ast::ast::Expression<'_>) -> bool {
    use oxc_ast::ast::Expression as E;
    matches!(
        expr,
        E::NumericLiteral(_)
            | E::BooleanLiteral(_)
            | E::NullLiteral(_)
            | E::BigIntLiteral(_)
            | E::StringLiteral(_)
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

/// A function declaration is dead if its name is never *read* from outside its
/// own body — i.e. no live code calls or otherwise observes it. Defining a
/// function declaration has no side effect, so when nothing external reads the
/// binding, the whole statement is inert: its body only runs when called, and a
/// call can only come through an external read.
///
/// Two gaps over plain `symbol_is_unused` (which counts *any* reference — read or
/// write — as a use) are closed here, both exploited by obfuscator string-array
/// accessors left dead after every call site is inlined:
///
///   * **Self-reassignment.** `function f(){ … f = function(){ return a; } … }`
///     leaves a lone *write* reference that `symbol_is_unused` treats as live.
///     Keying on *reads* drops it.
///   * **Self-reads.** The javascript-obfuscator base64/RC4 accessor memoizes
///     through its own name: `function f(i){ … if (f.X === undefined){ f.Y = …;
///     f.Z = {}; f.X = true; } … f.Z[k] … }`. Every surviving reference is a
///     *read* of `f`, but all are lexically inside `f`'s own body, so they only
///     execute if `f` is called from outside — which never happens once the call
///     sites are lifted. Excluding self-body reads lets it be collected.
///
/// Self-body reads are identified by counting, inside the function's own body,
/// the read references that resolve to its symbol, and comparing against the
/// symbol's *total* resolved read count. When they match, every read is in-body
/// and the declaration is dead; an external read makes the in-body count smaller
/// and keeps it live.
fn fn_decl_is_dead(
    symbol: oxc_syntax::symbol::SymbolId,
    func: &oxc_ast::ast::Function<'_>,
    scoping: &Scoping,
) -> bool {
    let total_reads = scoping
        .get_resolved_references(symbol)
        .filter(|r| r.is_read())
        .count();
    if total_reads == 0 {
        return true;
    }
    // Count the read references to `symbol` that occur within the function's own
    // body (its self-memoizing `f.cache`/`f = …` references). Resolution is by
    // `reference_id`, so a parameter or local that shadows the name is *not*
    // miscounted — only references that actually bind to `symbol` are tallied.
    let mut scan = SelfReadScan {
        symbol,
        scoping,
        in_body_reads: 0,
    };
    if let Some(body) = &func.body {
        for stmt in &body.statements {
            scan.visit_statement(stmt);
        }
    }
    total_reads == scan.in_body_reads
}

/// Counts read references resolving to `symbol` within the subtree it visits.
struct SelfReadScan<'r> {
    symbol: oxc_syntax::symbol::SymbolId,
    scoping: &'r Scoping,
    in_body_reads: usize,
}

impl<'a, 'r> Visit<'a> for SelfReadScan<'r> {
    fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
        if let Some(rid) = it.reference_id.get() {
            let r = self.scoping.get_reference(rid);
            if r.symbol_id() == Some(self.symbol) && r.is_read() {
                self.in_body_reads += 1;
            }
        }
    }
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
