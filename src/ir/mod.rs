//! A CFG / SSA dataflow layer for sound intraprocedural flow transforms.
//!
//! See `SSA_IR_PLAN.md`. The model is: per function body, lower the AST to a
//! minimal CFG that tracks control flow plus the def/use events of *versionable*
//! (non-escaping, non-captured) local symbols; build dominators + SSA over it;
//! and use the resulting reaching-definition facts to drive sound rewrites
//! (const-aggregate member folds, scalar const propagation) applied back onto the
//! original AST by node identity. Anything the lowering can't model ⇒ bail on
//! that function, leaving its AST untouched (partial coverage is always sound).
//!
//! The transforms here are *sound by construction*, not by hand-proof: a use is
//! rewritten only when SSA renaming proves a single concrete constant definition
//! reaches it, and escape analysis proves that definition's value cannot be
//! mutated through an alias. This is the property the structural passes could not
//! give (see `memory/soundness-audit.md`).

mod analysis;
mod cfg;
mod dom;
mod ssa;

use std::collections::HashMap;

use oxc_allocator::{Allocator, CloneIn, GetAddress};
use oxc_ast::ast::*;
use oxc_ast::AstBuilder;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_syntax::number::NumberBase;
use oxc_syntax::scope::ScopeId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;
use cfg::{DefVal, Key, Lowerer, NodeId, UseKind};
use dom::Dominators;

/// The SSA dataflow optimization pass (plan's `SsaOpt`).
#[derive(Default)]
pub struct SsaOpt;

impl Pass for SsaOpt {
    fn name(&self) -> &'static str {
        "ssa-opt"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        // Phase 1 (read-only): per function unit, lower → dominators → SSA →
        // compute the set of node-identified rewrites.
        let mut collector = UnitProcessor {
            allocator,
            scoping: &scoping,
            rewrites: HashMap::new(),
        };
        collector.visit_program(program);
        let rewrites = collector.rewrites;
        if rewrites.is_empty() {
            return false;
        }

        // Phase 2 (mutating): apply each rewrite to the exact AST node it targets,
        // matched by arena address (stable and unchanged since phase 1).
        let mut replacer = Replacer { rewrites, changed: false };
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        replacer.changed
    }
}

/// Walks the program, processing each function-level unit (the program top level,
/// every function body, every arrow body) as an independent IR problem.
struct UnitProcessor<'a, 's> {
    allocator: &'a Allocator,
    scoping: &'s Scoping,
    rewrites: HashMap<NodeId, Expression<'a>>,
}

impl<'a, 's> UnitProcessor<'a, 's> {
    fn process(&mut self, scope: ScopeId, stmts: &[Statement<'a>]) {
        let cands = analysis::collect_candidates(stmts, scope, self.scoping);
        if cands.is_empty() {
            return;
        }
        let Some(cfg) = Lowerer::new(self.allocator, self.scoping, &cands).lower(stmts) else {
            return; // bailed: unmodelled construct
        };
        let dom = Dominators::compute(&cfg.succs, cfg.entry);
        let ast = AstBuilder::new(self.allocator);
        for u in ssa::resolve(&cfg, &dom) {
            if let Some(rep) = build_replacement(u.kind, u.val, ast) {
                self.rewrites.insert(u.node, rep);
            }
        }
    }
}

impl<'a, 's> Visit<'a> for UnitProcessor<'a, 's> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.process(self.scoping.root_scope_id(), &program.body);
        walk::walk_program(self, program);
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: oxc_syntax::scope::ScopeFlags) {
        if let (Some(scope), Some(body)) = (func.scope_id.get(), &func.body) {
            self.process(scope, &body.statements);
        }
        walk::walk_function(self, func, flags);
    }

    fn visit_arrow_function_expression(&mut self, arrow: &ArrowFunctionExpression<'a>) {
        if let Some(scope) = arrow.scope_id.get() {
            self.process(scope, &arrow.body.statements);
        }
        walk::walk_arrow_function_expression(self, arrow);
    }
}

/// Build the literal that replaces a folded use, or `None` if it doesn't fold.
fn build_replacement<'a>(
    kind: &UseKind,
    val: &DefVal<'a>,
    ast: AstBuilder<'a>,
) -> Option<Expression<'a>> {
    match (kind, val) {
        // Scalar const propagation: `s` where `s = <literal>` reaches.
        (UseKind::Scalar, DefVal::Scalar(e)) => Some(e.clone_in(ast.allocator)),

        // Const-array element fold: `s[i]` where `s = [ ... ]` reaches.
        (UseKind::Member(Key::Index(i)), DefVal::Array(elems)) => {
            elems.get(*i).and_then(|slot| slot.as_ref()).map(|e| e.clone_in(ast.allocator))
        }
        // `s.length` on a known array.
        (UseKind::Member(Key::Name(n)), DefVal::Array(elems)) if n == "length" => {
            Some(num_lit(elems.len() as f64, ast))
        }

        // Const-object property fold: `s.name` / `s[i]` where `s = { ... }` reaches.
        (UseKind::Member(Key::Name(n)), DefVal::Object(map)) => {
            map.get(n).map(|e| e.clone_in(ast.allocator))
        }
        (UseKind::Member(Key::Index(i)), DefVal::Object(map)) => {
            map.get(&i.to_string()).map(|e| e.clone_in(ast.allocator))
        }

        _ => None,
    }
}

fn num_lit<'a>(n: f64, ast: AstBuilder<'a>) -> Expression<'a> {
    ast.expression_numeric_literal(oxc_span::SPAN, n, None, NumberBase::Decimal)
}

/// Applies the computed rewrites by matching each expression node's arena address.
struct Replacer<'a> {
    rewrites: HashMap<NodeId, Expression<'a>>,
    changed: bool,
}

impl<'a> Traverse<'a, ()> for Replacer<'a> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, _ctx: &mut TraverseCtx<'a, ()>) {
        if let Some(rep) = self.rewrites.remove(&expr.address()) {
            *expr = rep;
            self.changed = true;
        }
    }
}
