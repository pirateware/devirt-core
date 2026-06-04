//! Cross-function write-once constant inlining (materialization).
//!
//! Control-flow-flattened VMs hide their opcode/case labels as a table of
//! write-once integers, assigned in an *init* function and read as switch labels
//! in the dispatchers:
//!
//! ```text
//! var L1, L2;                              // declared once at IIFE scope, no init
//! function init(){ L1 = 19; L2 = 187; }    // each written exactly once, a literal
//! init();                                   // run unconditionally, once, up top
//! var run = function(s){ while(s){ switch(s){ case L1: …; case L2: …; } } };
//! ```
//!
//! Replacing every read of `L1` with `19` turns the dispatcher into a plain
//! numeric switch (which `unflatten` and const-fold can then act on) and lets DCE
//! delete the now-dead table and the emptied init function.
//!
//! The soundness reasoning — the single-write census and the interprocedural
//! happens-before proof — lives in [`crate::ir::ConstResolver`] (engine) and
//! [`crate::ir::ordering`] (the ordering oracle). This pass is now a thin
//! **materialization** client: it asks the resolver which symbols are provably
//! constant and rewrites their reads to the literal. Keeping resolution separate
//! lets the unflatteners consult the same facts without mutating the AST first.

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::*;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use std::collections::HashMap;

use crate::ir::ConstResolver;
use crate::pass::Pass;
use crate::util::build_scoping;

pub struct GlobalConstInline;

impl Pass for GlobalConstInline {
    fn name(&self) -> &'static str {
        "global-const-inline"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);
        let resolver = ConstResolver::build(program, &scoping, allocator);

        if std::env::var("GCI_DEBUG").is_ok() {
            // Coverage, so "didn't fire" is never mistaken for "covered everything":
            // how many single-literal-write candidates were inlinable vs. left
            // (writer not in a top-level init function, or an unsafe read ordering).
            eprintln!(
                "[gci] inlined={} of single-literal-write candidates={} (rest: nested/non-top init or unprovable order)",
                resolver.accepted().len(),
                resolver.single_lit_candidates(),
            );
        }

        if resolver.accepted().is_empty() {
            return false;
        }

        let mut replacer = Replacer {
            accepted: resolver.accepted(),
            changed: false,
        };
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        replacer.changed
    }
}

// ---------------------------------------------------------------------------
// Replacement (materialization)
// ---------------------------------------------------------------------------

struct Replacer<'a, 'm> {
    accepted: &'m HashMap<SymbolId, Expression<'a>>,
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
        if let Some(val) = self.accepted.get(&sym) {
            *expr = val.clone_in(ctx.ast.allocator);
            self.changed = true;
        }
    }
}
