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
mod inline;
mod ssa;

pub use inline::InlineSingleUse;

/// Symbol-precise "which local aggregate tables are NOT provably immutable" query,
/// reused by `decoder-lift` as the soundness gate for baking a table into the
/// decoder preamble (plan §5.3). One source of truth for "provably stable local".
pub use analysis::mutable_aggregate_tables;

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

        // Names bound by exactly one symbol. Copy propagation emits an identifier
        // by name, so it must only do so for a globally-unique name — otherwise the
        // emitted name could resolve to a shadowing binding at the use site.
        let mut name_counts: HashMap<&str, u32> = HashMap::new();
        for name in scoping.symbol_names() {
            *name_counts.entry(name).or_default() += 1;
        }
        let unique_names: std::collections::HashSet<String> = name_counts
            .into_iter()
            .filter(|(_, c)| *c == 1)
            .map(|(n, _)| n.to_string())
            .collect();

        // Phase 1 (read-only): per function unit, lower → dominators → SSA →
        // compute the set of node-identified rewrites.
        let mut collector = UnitProcessor {
            allocator,
            scoping: &scoping,
            unique_names: &unique_names,
            rewrites: HashMap::new(),
        };
        collector.visit_program(program);
        let rewrites = collector.rewrites;
        if rewrites.is_empty() {
            return false;
        }

        // Phase 2 (mutating): apply each rewrite to the exact AST node it targets,
        // matched by arena address (stable and unchanged since phase 1).
        let mut replacer = Replacer {
            rewrites,
            changed: false,
        };
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        replacer.changed
    }
}

/// Walks the program, processing each function-level unit (the program top level,
/// every function body, every arrow body) as an independent IR problem.
struct UnitProcessor<'a, 's> {
    allocator: &'a Allocator,
    scoping: &'s Scoping,
    unique_names: &'s std::collections::HashSet<String>,
    rewrites: HashMap<NodeId, Expression<'a>>,
}

impl<'a, 's> UnitProcessor<'a, 's> {
    fn process(&mut self, scope: ScopeId, stmts: &[Statement<'a>]) {
        let cands = analysis::collect_candidates(stmts, scope, self.scoping, true);
        if cands.is_empty() {
            return;
        }
        let Some(cfg) = Lowerer::new(self.allocator, self.scoping, &cands).lower(stmts) else {
            return; // bailed: unmodelled construct
        };
        let dom = Dominators::compute(&cfg.succs, cfg.entry);
        let ast = AstBuilder::new(self.allocator);
        for r in ssa::resolve(&cfg, &dom) {
            match r {
                ssa::Resolved::Const { node, kind, val } => {
                    if let Some(rep) = build_replacement(kind, val, ast) {
                        self.rewrites.insert(node, rep);
                    }
                }
                ssa::Resolved::Copy { node, src } => {
                    // Replace the use with the source identifier — only when its
                    // name is globally unique (no shadowing hazard).
                    let name = self.scoping.symbol_name(src);
                    if self.unique_names.contains(name) {
                        let id = ast.expression_identifier(oxc_span::SPAN, ast.str(name));
                        self.rewrites.insert(node, id);
                    }
                }
            }
        }
        collect_dead_stores(&cfg, ast, &mut self.rewrites);
    }
}

/// Flow-sensitive dead-store removal: within a block, a `s = rhs` whose stored
/// version is overwritten by a later `s = …` before any read of `s` is dead. Its
/// node is rewritten to a clone of `rhs` — preserving the rhs value and effects,
/// dropping only the now-useless store. Intra-block only, so no phi/liveness
/// reasoning is needed: the later def provably overwrites with no read between.
fn collect_dead_stores<'a>(
    cfg: &cfg::Cfg<'a>,
    ast: AstBuilder<'a>,
    rewrites: &mut HashMap<NodeId, Expression<'a>>,
) {
    use cfg::Event;
    for block in &cfg.blocks {
        // Per symbol: the last `s = rhs` store not yet read or overwritten.
        let mut pending: HashMap<oxc_syntax::symbol::SymbolId, &cfg::AssignInfo<'a>> =
            HashMap::new();
        for ev in &block.events {
            match ev {
                Event::Def { sym, assign, .. } => {
                    // The previous pending store of `sym` is overwritten unread.
                    if let Some(prev) = pending.remove(sym) {
                        rewrites.insert(prev.node, prev.rhs.clone_in(ast.allocator));
                    }
                    if let Some(info) = assign {
                        pending.insert(*sym, info);
                    }
                }
                Event::Use { sym, .. } => {
                    // A read makes the pending store live.
                    pending.remove(sym);
                }
                // A barrier (a call etc.) cannot read a candidate (they don't
                // escape), so it leaves the pending store dead.
                Event::Barrier => {}
            }
        }
        // Stores still pending at block end may be read on a later block → keep.
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
        (UseKind::Member(Key::Index(i)), DefVal::Array(elems)) => elems
            .get(*i)
            .and_then(|slot| slot.as_ref())
            .map(|e| e.clone_in(ast.allocator)),
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
