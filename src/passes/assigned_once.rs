//! Assigned-once inlining.
//!
//! For every binding that (a) is initialized with a duplicable literal and (b)
//! is never re-assigned (`symbol_is_mutated == false`), replace each *read* of
//! that binding with a clone of the literal. The now-unused declaration is left
//! for the DCE pass to remove.
//!
//! Soundness — a read may only be replaced when the initializer is guaranteed to
//! have run before it; otherwise the read could observe the binding's pre-init
//! `undefined`. Proving that in general is dominance analysis, but a cheap,
//! conservative sufficient condition holds here. All three must hold:
//!
//! - the declarator sits **directly** in its function (or the program) — not
//!   inside any block, `if`, loop, `switch`, `try`, or `with`. Each of those
//!   introduces its own scope, so the test is simply "the declarator's lexical
//!   scope *is* its enclosing function/root scope". Such a declarator runs
//!   unconditionally, exactly once, before any later statement of the function.
//! - the read is in that **same** function (no closure could be invoked before
//!   the initializer ran).
//! - the read appears **after** the declarator in source order.
//!
//! Reads that fail these — captured by a nested function, or textually before the
//! declaration — are simply left untouched (the declaration then survives DCE).

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::{BindingPattern, Expression, Program, VariableDeclarator};
use oxc_semantic::Scoping;
use oxc_syntax::scope::{ScopeFlags, ScopeId};
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::{build_scoping, is_inlineable_literal};

#[derive(Default)]
pub struct AssignedOnceInline;

impl Pass for AssignedOnceInline {
    fn name(&self) -> &'static str {
        "assigned-once-inline"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        let mut collector = Collector { values: HashMap::new(), poisoned: HashSet::new() };
        let scoping = traverse_mut(&mut collector, allocator, program, scoping, ());

        for sym in &collector.poisoned {
            collector.values.remove(sym);
        }
        if collector.values.is_empty() {
            return false;
        }

        let mut replacer = Replacer { values: &collector.values, changed: false };
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        replacer.changed
    }
}

/// The nearest enclosing function scope of `scope`, or the root if there is none.
fn enclosing_function(scoping: &Scoping, mut scope: ScopeId) -> ScopeId {
    loop {
        if scoping.scope_flags(scope).contains(ScopeFlags::Function) {
            return scope;
        }
        match scoping.scope_parent_id(scope) {
            Some(parent) => scope = parent,
            None => return scope, // root
        }
    }
}

/// A recorded constant: the literal value plus the dominance facts needed to
/// decide, at each read, whether the initializer is guaranteed to have run.
struct Const<'a> {
    value: Expression<'a>,
    /// The function (or root) scope the declarator runs in.
    func_scope: ScopeId,
    /// Source offset just past the declarator; a read must start at or after it.
    decl_end: u32,
}

struct Collector<'a> {
    values: HashMap<SymbolId, Const<'a>>,
    poisoned: HashSet<SymbolId>,
}

impl<'a> Traverse<'a, ()> for Collector<'a> {
    fn enter_variable_declarator(
        &mut self,
        node: &mut VariableDeclarator<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let BindingPattern::BindingIdentifier(bid) = &node.id else {
            return;
        };
        let Some(init) = &node.init else {
            return;
        };
        if !is_inlineable_literal(init) {
            return;
        }
        let sym = bid.symbol_id();
        if ctx.scoping().symbol_is_mutated(sym) {
            return;
        }
        // Unconditional-execution gate: the declarator must sit directly in its
        // function/root scope. Any wrapping block/if/loop/switch/try introduces
        // an intermediate scope, so a nested declarator fails this check and is
        // never recorded (its init might be skipped at runtime).
        let scope = ctx.current_scope_id();
        if enclosing_function(ctx.scoping(), scope) != scope {
            self.poisoned.insert(sym);
            return;
        }
        // Two declarations writing the same symbol → value is ambiguous; poison.
        if self.values.contains_key(&sym) {
            self.poisoned.insert(sym);
            return;
        }
        let value = init.clone_in(ctx.ast.allocator);
        self.values.insert(sym, Const { value, func_scope: scope, decl_end: node.span.end });
    }

    // A `for (var x of/in …)` head rebinds `x` on every iteration, but that write
    // is a binding, not an `is_write` reference, so `symbol_is_mutated` misses it.
    // Poison every symbol bound in such a head so it is never treated as a const.
    fn enter_for_of_statement(
        &mut self,
        node: &mut oxc_ast::ast::ForOfStatement<'a>,
        _ctx: &mut TraverseCtx<'a, ()>,
    ) {
        poison_for_head(&node.left, &mut self.poisoned);
    }

    fn enter_for_in_statement(
        &mut self,
        node: &mut oxc_ast::ast::ForInStatement<'a>,
        _ctx: &mut TraverseCtx<'a, ()>,
    ) {
        poison_for_head(&node.left, &mut self.poisoned);
    }
}

/// Poison every symbol bound by a `for`-head variable declaration.
fn poison_for_head(left: &oxc_ast::ast::ForStatementLeft<'_>, poisoned: &mut HashSet<SymbolId>) {
    use oxc_ast::ast::ForStatementLeft;
    let ForStatementLeft::VariableDeclaration(d) = left else {
        return; // bare/destructuring assignment targets are `is_write` references
    };
    struct Collect<'p>(&'p mut HashSet<SymbolId>);
    impl<'a, 'p> oxc_ast_visit::Visit<'a> for Collect<'p> {
        fn visit_binding_identifier(&mut self, id: &oxc_ast::ast::BindingIdentifier<'a>) {
            if let Some(sym) = id.symbol_id.get() {
                self.0.insert(sym);
            }
        }
    }
    oxc_ast_visit::Visit::visit_variable_declaration(&mut Collect(poisoned), d);
}

struct Replacer<'a, 'm> {
    values: &'m HashMap<SymbolId, Const<'a>>,
    changed: bool,
}

impl<'a, 'm> Traverse<'a, ()> for Replacer<'a, 'm> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Expression::Identifier(idref) = expr else {
            return;
        };
        let Some(rid) = idref.reference_id.get() else {
            return;
        };
        let reference = ctx.scoping().get_reference(rid);
        if !reference.is_read() {
            return;
        }
        let Some(sym) = reference.symbol_id() else {
            return;
        };
        let Some(konst) = self.values.get(&sym) else {
            return;
        };
        // The read must be in the same function as the declarator (no closure
        // could run before the init) and lexically after it.
        if enclosing_function(ctx.scoping(), ctx.current_scope_id()) != konst.func_scope {
            return;
        }
        if idref.span.start < konst.decl_end {
            return;
        }
        *expr = konst.value.clone_in(ctx.ast.allocator);
        self.changed = true;
    }
}
