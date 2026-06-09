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
use oxc_traverse::{traverse_mut, Ancestor, Traverse, TraverseCtx};

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

        let mut collector = Collector {
            values: HashMap::new(),
            poisoned: HashSet::new(),
        };
        let scoping = traverse_mut(&mut collector, allocator, program, scoping, ());

        for sym in &collector.poisoned {
            collector.values.remove(sym);
        }
        if collector.values.is_empty() {
            return false;
        }

        let mut replacer = Replacer {
            values: &collector.values,
            changed: false,
        };
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
    /// Source offset past which a read is no longer guaranteed to see the
    /// initialized value. `u32::MAX` for a declarator that runs unconditionally
    /// (directly in the function/root). For one sitting directly in a `try {}`
    /// block it is the block's end: a read in the `catch`/`finally` or after the
    /// `try` could observe the pre-init `undefined` (an earlier try statement may
    /// have thrown before the declarator ran), so it is excluded.
    region_end: u32,
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
        // Execution gate. The cheap, sound case is a declarator directly in its
        // function/root scope: it runs unconditionally, exactly once, before any
        // later statement — so every later read (in that function) is safe.
        // We additionally admit a declarator sitting *directly* in a `try {}`
        // block; there the safe region is bounded to the block (see `region_end`).
        // Anything else (an `if`/loop/switch body, a `catch`, a nested block) might
        // be skipped at runtime, so it stays poisoned.
        let scope = ctx.current_scope_id();
        let func_scope = enclosing_function(ctx.scoping(), scope);
        let region_end = if func_scope == scope {
            u32::MAX
        } else if let Some(end) = direct_try_block_region(ctx) {
            end
        } else {
            self.poisoned.insert(sym);
            return;
        };
        // Two declarations writing the same symbol → value is ambiguous; poison.
        if self.values.contains_key(&sym) {
            self.poisoned.insert(sym);
            return;
        }
        let value = init.clone_in(ctx.ast.allocator);
        self.values.insert(
            sym,
            Const {
                value,
                func_scope,
                decl_end: node.span.end,
                region_end,
            },
        );
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

/// If the declarator sits *directly* in a `try { … }` block — a top-level
/// statement of the block, not nested in an inner `if`/loop/block — return the
/// block's end offset. Within `[decl_end, block_end)` a read is guaranteed to see
/// the initialized value (control reached it, so it passed the unconditional
/// declarator). Past it (`catch`/`finally`, or after the `try`) a read could see
/// the pre-init `undefined`, so it is excluded. Returns `None` for any other
/// nesting, which the caller poisons.
fn direct_try_block_region(ctx: &TraverseCtx<'_, ()>) -> Option<u32> {
    let mut ancestors = ctx.ancestors();
    // The first statement-list owner above the declarator (skipping the
    // intervening `VariableDeclaration`). If it is the function/program body the
    // unconditional case already applied; here we only care about a block.
    let block_end = loop {
        match ancestors.next()? {
            Ancestor::BlockStatementBody(b) => break b.span().end,
            Ancestor::FunctionBodyStatements(_) | Ancestor::ProgramBody(_) => return None,
            _ => continue,
        }
    };
    // That block must be a `try`'s `block` (not a `catch`/`finally` body, an
    // `if`/loop body, or a bare nested block).
    match ancestors.next()? {
        Ancestor::TryStatementBlock(_) => Some(block_end),
        _ => None,
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
        if idref.span.start < konst.decl_end || idref.span.start >= konst.region_end {
            return;
        }
        *expr = konst.value.clone_in(ctx.ast.allocator);
        self.changed = true;
    }
}
