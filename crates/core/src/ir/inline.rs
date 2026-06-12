//! Single-use value inlining.
//!
//! Obfuscators explode expressions into a chain of temporaries each used once —
//! `var t = decode(5); obj[t]` — purely to hide structure. This pass folds such a
//! temporary back into the place it is read and drops the now-spent binding,
//! turning the example into `obj[decode(5)]`. That is valuable on its own (less
//! indirection) but the real payoff is the *snowball*: putting `decode(5)` next to
//! its member access lets `decoder-lift`, `member-normalize`, and the const folds
//! finally see it, so every other pass reaches further. It is the generic
//! counterpart to `proxy-inline`, without the operator-table restriction.
//!
//! It reuses the `src/ir` flow layer: `analysis::collect_candidates` is the
//! soundness gate (a binding inlines only if it is non-escaping, non-captured, and
//! every write is a modelled simple def), and the `Lowerer` gives the ordered
//! def/use/effect event stream per straight-line block.
//!
//! ## Soundness — a single-use temp `t = EXPR` is inlined into its sole read iff:
//!
//! 1. `t` is a [`collect_candidates`] survivor (escape-safe; we see every write).
//! 2. `t` has **exactly one** value-establishing def and **exactly one** use. One
//!    use ⇒ EXPR's effects are never duplicated. The def is either a declarator
//!    `var t = EXPR` or a standalone `t = EXPR;` statement (so the removal step can
//!    drop it cleanly); a no-init `var t;` hoist is a runtime no-op and ignored.
//!    The use's *kind* does not matter — splicing EXPR for the identifier yields
//!    the same value whether the read is bare (`t`), an index (`o[t]`), or an
//!    object (`t.x` → `EXPR.x`).
//! 3. The def and use lie in the **same straight-line block**, def before use.
//!    Same block ⇒ no branch/loop boundary, so the def always reaches the use, and
//!    (because a `{ … }` block emits boundary barriers — see below) the def and use
//!    share one lexical scope, so EXPR's free names resolve identically at both.
//! 4. **Nothing but other reads sits between them**: the gap holds only `Use`
//!    events — no `Barrier` (call/new/update/delete/member-or-outer write/block
//!    boundary) and no `Def` (no candidate is rebound, including the `var t;`
//!    hoist). So no effect is reordered relative to EXPR, and every value EXPR
//!    reads is unchanged when it is moved forward to the use site.
//! 5. EXPR contains no function/class/arrow (cloning resets their scope cells,
//!    which the traverse walker would later unwrap and panic on — same guard as
//!    `proxy-inline`).
//!
//! Anything failing these is left untouched; partial coverage is always sound. The
//! `src/ir` escape analysis already excludes `eval` units, aliased or
//! member-mutated aggregates, and `for`-head rebindings, so those never reach here.
//! `arguments`-using units are *kept* — see `Collector::process`.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::*;
use oxc_ast_visit::{walk, Visit};
use oxc_semantic::Scoping;
use oxc_syntax::scope::ScopeId;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use super::analysis;
use super::cfg::{Event, Lowerer};
use crate::pass::Pass;
use crate::util::build_scoping;

/// Upper bound on the size (expression-node count) of an init we will inline.
/// Single-use inlining never duplicates, so the only cost of a large init is
/// readability — folding a 4,600-temp script into 4,600 monster expressions would
/// be *worse*. Small temps (decoder calls, member reads, arithmetic) sail under
/// this; genuinely large named subexpressions stay named.
const MAX_INLINE_WEIGHT: usize = 32;

#[derive(Default)]
pub struct InlineSingleUse;

impl Pass for InlineSingleUse {
    fn name(&self) -> &'static str {
        "inline-single-use"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        // Symbols declared in a `for (var … ; ;)` init head: their declarator is
        // not a direct member of a statement list, so the removal step (which
        // edits statement lists) cannot delete it. Excluding them keeps removal
        // and inlining in lock-step — never inline a use we cannot also unbind.
        let for_init = collect_for_init_syms(program);

        // Symbols whose value is established by a *standalone* `x = rhs;`
        // statement (`x` a plain identifier, simple `=`). These are removable the
        // same way a declarator is: drop the statement. Obfuscators emit reams of
        // these — sequence-split already breaks `a = …, b = …` chains into one
        // statement each — so they are the bulk of single-use temps.
        let standalone_assign = collect_standalone_assign_syms(program, &scoping);

        let mut collector = Collector {
            allocator,
            scoping: &scoping,
            for_init: &for_init,
            standalone_assign: &standalone_assign,
            inits: HashMap::new(),
            removable: HashSet::new(),
        };
        collector.visit_program(program);

        if collector.inits.is_empty() {
            return false;
        }
        let Collector {
            inits, removable, ..
        } = collector;

        let mut replacer = Replacer {
            inits,
            removable,
            changed: false,
        };
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        replacer.changed
    }
}

// ---------------------------------------------------------------------------
// Analysis (read-only): pick the inlinable single-use temps per function unit.
// ---------------------------------------------------------------------------

struct Collector<'a, 's> {
    allocator: &'a Allocator,
    scoping: &'s Scoping,
    for_init: &'s HashSet<SymbolId>,
    standalone_assign: &'s HashSet<SymbolId>,
    /// Inlinable symbol → the init clone to splice at its sole read. The read is
    /// matched by symbol at rewrite time (so a member/method use `t.x` rewrites
    /// its object), which is why we key by symbol, not by node address.
    inits: HashMap<SymbolId, Expression<'a>>,
    /// Same key set as `inits`; the declarators to drop. Kept as its own set so
    /// declarator removal never races the consuming `inits.remove` at the use.
    removable: HashSet<SymbolId>,
}

impl<'a, 's> Collector<'a, 's> {
    fn process(&mut self, scope: ScopeId, stmts: &[Statement<'a>]) {
        // Root-scope bindings become observable globals; never unbind them
        // (mirrors `dce::is_removable_symbol`). Real obfuscator temps live inside
        // the function/IIFE units, which are processed via the visitor below.
        if scope == self.scoping.root_scope_id() {
            return;
        }
        // `arguments`-using functions are kept (pass `false`): we never inline a
        // param, and any way `arguments` could mutate a param is a barrier in the
        // gap we require to be effect-free. See `collect_candidates`.
        let cands = analysis::collect_candidates(stmts, scope, self.scoping, false);
        if cands.is_empty() {
            return;
        }
        let Some(cfg) = Lowerer::new(self.allocator, self.scoping, &cands)
            .capture_defs()
            .lower(stmts)
        else {
            return; // bailed: an unmodelled construct
        };

        // Locate every def and use event per candidate (block index, event index).
        let mut def_pos: HashMap<SymbolId, Vec<(usize, usize)>> = HashMap::new();
        let mut use_pos: HashMap<SymbolId, Vec<(usize, usize)>> = HashMap::new();
        for (b, block) in cfg.blocks.iter().enumerate() {
            for (i, ev) in block.events.iter().enumerate() {
                match ev {
                    Event::Def { sym, .. } => def_pos.entry(*sym).or_default().push((b, i)),
                    Event::Use { sym, .. } => use_pos.entry(*sym).or_default().push((b, i)),
                    Event::Barrier => {}
                }
            }
        }

        // Pass 1: the per-symbol checks (one value def, one use, same block, def
        // before use, clean gap, size, no scoped node). The use's *kind* is
        // irrelevant — splicing the init for the identifier yields the same value
        // whether the read is bare (`t`), an index (`o[t]`), or an object
        // (`t.x` → `init.x`) — so member/method uses are inlined too.
        let mut tentative: Vec<(SymbolId, &Expression<'a>)> = Vec::new();
        for (&sym, dpos) in &def_pos {
            if self.for_init.contains(&sym) {
                continue;
            }
            let Some(upos) = use_pos.get(&sym) else {
                continue;
            };
            if upos.len() != 1 {
                continue;
            }
            // The single *value-establishing* def. A no-init `var x;` is a runtime
            // no-op (hoist only), so it does not count — but if more than one real
            // def exists (`var x = a; … x = b;`) the reaching value is ambiguous
            // here, so bail. An assignment def counts only when standalone (the
            // removal step can drop the `x = rhs;` statement).
            let mut value_def: Option<(usize, usize)> = None;
            let mut ambiguous = false;
            for &(b, i) in dpos {
                let is_value = match &cfg.blocks[b].events[i] {
                    Event::Def {
                        decl_init: Some(_), ..
                    } => true,
                    Event::Def {
                        assign: Some(_), ..
                    } => self.standalone_assign.contains(&sym),
                    _ => false,
                };
                if is_value {
                    if value_def.is_some() {
                        ambiguous = true;
                        break;
                    }
                    value_def = Some((b, i));
                }
            }
            if ambiguous {
                continue;
            }
            let Some((db, di)) = value_def else { continue };
            let (ub, ui) = upos[0];
            if db != ub || di >= ui {
                continue; // different blocks, or use does not follow the def
            }
            let events = &cfg.blocks[db].events;
            let init: &Expression<'a> = match &events[di] {
                Event::Def {
                    decl_init: Some(init),
                    ..
                } => init,
                Event::Def {
                    assign: Some(ai), ..
                } => &ai.rhs,
                _ => continue, // unreachable: value_def points at one of the above
            };
            // The gap must hold reads only — no effect, rebinding (including the
            // no-init `var x;` hoist, a Def), or scope edge.
            if !events[di + 1..ui]
                .iter()
                .all(|e| matches!(e, Event::Use { .. }))
            {
                continue;
            }
            if contains_scoped_node(init) || weight(init) > MAX_INLINE_WEIGHT {
                continue;
            }
            tentative.push((sym, init));
        }
        if tentative.is_empty() {
            return;
        }

        // Pass 2: defer any inline whose init still reads another inlinable temp.
        // Cloning it now would freeze a reference that is about to be unbound this
        // round; deferring lets the inner temp resolve first, the outer next round.
        // Name-based (clones reset reference ids) and conservative — candidate
        // names are unique within a unit, so a match is a real dependency; an
        // unrelated same-named outer symbol only costs us one extra round.
        let names: HashSet<&str> = tentative
            .iter()
            .map(|(s, _)| self.scoping.symbol_name(*s))
            .collect();
        for (sym, init) in &tentative {
            let own = self.scoping.symbol_name(*sym);
            let mut refs = HashSet::new();
            collect_ref_names(init, &mut refs);
            if refs
                .iter()
                .any(|r| r.as_str() != own && names.contains(r.as_str()))
            {
                continue;
            }
            self.inits.insert(*sym, (*init).clone_in(self.allocator));
            self.removable.insert(*sym);
        }
    }
}

impl<'a, 's> Visit<'a> for Collector<'a, 's> {
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

// ---------------------------------------------------------------------------
// Rewrite (mutating): splice each init into its use and unbind the temp.
// ---------------------------------------------------------------------------

struct Replacer<'a> {
    inits: HashMap<SymbolId, Expression<'a>>,
    removable: HashSet<SymbolId>,
    changed: bool,
}

impl<'a> Traverse<'a, ()> for Replacer<'a> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Expression::Identifier(idref) = expr else {
            return;
        };
        // A spliced init clone has its reference ids reset to `None`, so its own
        // identifiers never re-match here — the rewrite cannot cascade within one
        // pass (chains resolve a layer per fixpoint round).
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
        // `inits` holds exactly the single-use temps; this read is that one use.
        if let Some(rep) = self.inits.remove(&sym) {
            *expr = rep;
            self.changed = true;
        }
    }

    fn enter_statements(
        &mut self,
        stmts: &mut oxc_allocator::Vec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let before = stmts.len();
        // Drop the inlined temps' declarators (their init moved to the use).
        for s in stmts.iter_mut() {
            if let Statement::VariableDeclaration(vd) = s {
                vd.declarations.retain(|d| !self.is_removable_decl(d));
            }
        }
        // Drop a declaration emptied above, plus each standalone `x = rhs;`
        // statement whose `x` was inlined (its rhs likewise moved to the use).
        let scoping = ctx.scoping();
        stmts.retain(|s| match s {
            Statement::VariableDeclaration(vd) => !vd.declarations.is_empty(),
            Statement::ExpressionStatement(es) => {
                !self.is_removable_assign(&es.expression, scoping)
            }
            _ => true,
        });
        if stmts.len() != before {
            self.changed = true;
        }
    }
}

impl<'a> Replacer<'a> {
    fn is_removable_decl(&self, d: &VariableDeclarator<'a>) -> bool {
        let BindingPattern::BindingIdentifier(bid) = &d.id else {
            return false;
        };
        bid.symbol_id
            .get()
            .is_some_and(|s| self.removable.contains(&s))
    }

    /// Whether `expr` is `x = …` for an inlined symbol `x` — the def statement
    /// whose rhs has been moved to the use, now safe to drop.
    fn is_removable_assign(&self, expr: &Expression<'a>, scoping: &Scoping) -> bool {
        let Expression::AssignmentExpression(a) = expr else {
            return false;
        };
        if a.operator != oxc_syntax::operator::AssignmentOperator::Assign {
            return false;
        }
        let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
            return false;
        };
        id.reference_id
            .get()
            .and_then(|rid| scoping.get_reference(rid).symbol_id())
            .is_some_and(|s| self.removable.contains(&s))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Symbols bound by a `for (var … ; ;)` init head, program-wide. Over-collecting
/// (across units) is harmless — it only ever *excludes* an inline candidate.
fn collect_for_init_syms(program: &Program<'_>) -> HashSet<SymbolId> {
    struct V {
        out: HashSet<SymbolId>,
    }
    impl<'a> Visit<'a> for V {
        fn visit_for_statement(&mut self, f: &ForStatement<'a>) {
            if let Some(ForStatementInit::VariableDeclaration(d)) = &f.init {
                let mut c = BindingCollector { out: &mut self.out };
                c.visit_variable_declaration(d);
            }
            walk::walk_for_statement(self, f);
        }
    }
    struct BindingCollector<'x> {
        out: &'x mut HashSet<SymbolId>,
    }
    impl<'a, 'x> Visit<'a> for BindingCollector<'x> {
        fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
            if let Some(sym) = id.symbol_id.get() {
                self.out.insert(sym);
            }
        }
    }
    let mut v = V {
        out: HashSet::new(),
    };
    v.visit_program(program);
    v.out
}

/// Symbols established by a *standalone* `x = rhs;` statement (`x` a plain
/// identifier, simple `=`), program-wide. Only such defs can be removed by the
/// statement-list edit in `Replacer`; a `x = rhs` buried in a sequence or
/// condition is not collected, so it is never treated as an inlinable def.
fn collect_standalone_assign_syms(program: &Program<'_>, scoping: &Scoping) -> HashSet<SymbolId> {
    use oxc_syntax::operator::AssignmentOperator;
    struct V<'s> {
        scoping: &'s Scoping,
        out: HashSet<SymbolId>,
    }
    impl<'a, 's> Visit<'a> for V<'s> {
        fn visit_expression_statement(&mut self, es: &ExpressionStatement<'a>) {
            if let Expression::AssignmentExpression(a) = &es.expression {
                if a.operator == AssignmentOperator::Assign {
                    if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                        if let Some(sym) = id
                            .reference_id
                            .get()
                            .and_then(|rid| self.scoping.get_reference(rid).symbol_id())
                        {
                            self.out.insert(sym);
                        }
                    }
                }
            }
            walk::walk_expression_statement(self, es);
        }
    }
    let mut v = V {
        scoping,
        out: HashSet::new(),
    };
    v.visit_program(program);
    v.out
}

/// Names of every identifier *reference* in `expr` (preserved through a clone,
/// where reference ids are reset). Used for the defer dependency check.
fn collect_ref_names(expr: &Expression<'_>, out: &mut HashSet<String>) {
    struct V<'x> {
        out: &'x mut HashSet<String>,
    }
    impl<'a, 'x> Visit<'a> for V<'x> {
        fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
            self.out.insert(id.name.to_string());
        }
    }
    V { out }.visit_expression(expr);
}

/// Number of expression nodes in `expr` — a cheap readability-size proxy.
fn weight(expr: &Expression<'_>) -> usize {
    struct W {
        n: usize,
    }
    impl<'a> Visit<'a> for W {
        fn visit_expression(&mut self, e: &Expression<'a>) {
            self.n += 1;
            walk::walk_expression(self, e);
        }
    }
    let mut w = W { n: 0 };
    w.visit_expression(expr);
    w.n
}

/// Whether `expr` contains a function/arrow/class. Cloning such a node resets its
/// scope-id cell to `None`, which the traverse walker later unwraps — so we must
/// not splice one mid-traversal (same guard as `proxy-inline`).
fn contains_scoped_node(expr: &Expression<'_>) -> bool {
    struct Finder {
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(&mut self, _f: &Function<'a>, _flags: oxc_syntax::scope::ScopeFlags) {
            self.found = true;
        }
        fn visit_arrow_function_expression(&mut self, _f: &ArrowFunctionExpression<'a>) {
            self.found = true;
        }
        fn visit_class(&mut self, _c: &Class<'a>) {
            self.found = true;
        }
    }
    let mut f = Finder { found: false };
    f.visit_expression(expr);
    f.found
}
