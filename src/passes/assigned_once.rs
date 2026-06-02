//! Assigned-once inlining.
//!
//! For every binding that (a) is initialized with a duplicable literal and (b)
//! is never re-assigned (`symbol_is_mutated == false`), replace each *read* of
//! that binding with a clone of the literal. The now-unused declaration is left
//! for the DCE pass to remove.
//!
//! This is the first pass that uses the symbol table. It runs two traversals:
//! a collector (binding → literal value) and a replacer (read site → value).
//! Two passes are needed because a read may textually precede its declaration.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::{BindingPattern, Expression, Program, VariableDeclarator};
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

struct Collector<'a> {
    values: HashMap<SymbolId, Expression<'a>>,
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
        // Two declarations writing the same symbol → value is ambiguous; poison.
        if self.values.contains_key(&sym) {
            self.poisoned.insert(sym);
            return;
        }
        eprintln!("[collect] sym={:?} init_is_lit={}", sym, is_inlineable_literal(init));
        let cloned = init.clone_in(ctx.ast.allocator);
        self.values.insert(sym, cloned);
    }
}

struct Replacer<'a, 'm> {
    values: &'m HashMap<SymbolId, Expression<'a>>,
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
        if let Some(value) = self.values.get(&sym) {
            *expr = value.clone_in(ctx.ast.allocator);
            self.changed = true;
        }
    }
}
