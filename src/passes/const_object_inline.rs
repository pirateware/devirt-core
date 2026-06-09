//! Interprocedural constant object-property inlining.
//!
//! `javascript-obfuscator`'s string-map (a.k.a. the "object keys" dictionary)
//! hoists every string/number constant into one literal object and replaces each
//! use with a property read:
//!
//! ```text
//! var O = { koPtQ: "Original:", jXePr: "4|0|5|1|3|2", JvYzF: "a", NqXhV: "b" };
//! …
//! console.log(O.koPtQ, …);            // a string read
//! if (O.JvYzF === O.NqXhV) { … }       // an always-false opaque predicate
//! var parts = O.jXePr.split("|");      // a CFF order array hidden behind a hop
//! ```
//!
//! Until `O.koPtQ` becomes `"Original:"`, none of the downstream machinery can
//! act: the opaque predicate never folds to `false` (so its dead arm survives
//! DCE), and the string-order unflattener never sees the literal `"…".split("|")`
//! it needs. This pass resolves those reads program-wide.
//!
//! ## Why this is not [`crate::ir::SsaOpt`]'s job
//!
//! The SSA dataflow layer already folds `O.prop` for a non-escaping object — but
//! *intraprocedurally*. The moment `O` is captured by a nested closure that is
//! not itself inlined away (every proxy function in the obfuscator's output),
//! its property reads live in a different function than the literal and SSA can
//! no longer reach them. This pass closes exactly that gap with a whole-program
//! analysis, for the one case where it is sound to do so.
//!
//! ## Soundness — default to *not* inlining
//!
//! A symbol is rewritten only when all of:
//!   * **Single object write.** Its value is established by exactly one write and
//!     that write is an object literal (the census in [`CandidateCollector`]).
//!   * **Runs before every read.** That write is the *first statement* of its
//!     enclosing function (or the program) — an unconditional declarator that
//!     executes once, before any other statement of that body. Because nothing
//!     in the body runs ahead of it, every read — including reads inside nested
//!     closures, which can only be *invoked* by a later statement — observes the
//!     fully-initialized object. (A conservative sufficient condition; a
//!     declarator that is merely unconditional but not first could still be
//!     preceded by a call that invokes a capturing closure before init.)
//!   * **Never escapes or mutates.** Every reference is the object of a *static*
//!     member *read* `O.name`. Any other shape marks it escaped ([`Classifier`]):
//!     bare `O`, computed `O[x]`, passing/returning `O`, destructuring, and —
//!     unlike `dead-object-prop`, which can treat a member write as a harmless
//!     "access" because *keeping* a property is always safe — every *write*
//!     (`O.x = v`, `O.x += v`, `O.x++`, `delete O.x`). Inlining a property with a
//!     stale initial value would be unsound if that property is ever reassigned,
//!     so writes must escape.
//!
//! Only non-computed `Init` properties whose value is an [`is_inlineable_literal`]
//! (string/number/bool/null/bigint — pure and freely duplicable) are inlined,
//! using last-write-wins for duplicate keys; an object with a spread or computed
//! key is left entirely alone (its property set is not statically known).
//! Function-valued and accessor properties are kept, so an object that also holds
//! a live method (e.g. a call proxy) simply shrinks — `dead-object-prop` prunes
//! the now-unread literal members on a later round.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, Expression, ObjectExpression, ObjectPropertyKind, Program,
    PropertyKey, PropertyKind, SimpleAssignmentTarget, Statement, UnaryExpression, UpdateExpression,
    VariableDeclarator,
};
use oxc_syntax::operator::UnaryOperator;
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::{build_scoping, is_inlineable_literal};

#[derive(Default)]
pub struct ConstObjectInline {
    changed: bool,
}

impl Pass for ConstObjectInline {
    fn name(&self) -> &'static str {
        "const-object-inline"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        // 1. Census: symbols whose value is established by exactly one write, and
        //    that write is an object literal.
        let mut cand = CandidateCollector {
            scoping: &scoping,
            total_writes: HashMap::new(),
            object_writes: HashMap::new(),
        };
        cand.visit_program(program);
        let single_obj: HashSet<SymbolId> = cand
            .object_writes
            .keys()
            .copied()
            .filter(|s| {
                cand.total_writes.get(s).copied() == Some(1)
                    && cand.object_writes.get(s).copied() == Some(1)
            })
            .collect();
        if single_obj.is_empty() {
            return false;
        }

        // 2. Ordering: keep only those whose single write is the first statement
        //    of its function/program body, and snapshot its literal properties.
        let mut snap = FirstStmtSnapshots {
            alloc: allocator,
            values: HashMap::new(),
        };
        snap.visit_program(program);
        snap.values.retain(|sym, _| single_obj.contains(sym));
        if snap.values.is_empty() {
            return false;
        }
        let candidates: HashSet<SymbolId> = snap.values.keys().copied().collect();

        // 3. Escape analysis: a candidate survives only if every reference is a
        //    clean static member read.
        let mut cls = Classifier {
            scoping: &scoping,
            candidates: &candidates,
            escaped: HashSet::new(),
        };
        cls.visit_program(program);
        for sym in &cls.escaped {
            snap.values.remove(sym);
        }
        if snap.values.is_empty() {
            return false;
        }

        // 4. Replace `O.name` reads of a surviving candidate with the literal.
        let mut replacer = Replacer {
            values: &snap.values,
            changed: false,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut replacer, allocator, program, scoping, ());
        self.changed = replacer.changed;
        self.changed
    }
}

fn binding_sym(decl: &VariableDeclarator<'_>) -> Option<SymbolId> {
    match &decl.id {
        BindingPattern::BindingIdentifier(id) => id.symbol_id.get(),
        _ => None,
    }
}

/// Static (non-computed) property name as text.
fn static_key(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// Snapshot the inlinable literal properties of an object literal, applying
/// last-write-wins for duplicate keys. Returns `None` (object left untouched) if
/// the property set is not statically known — any spread or computed key.
fn snapshot<'a>(
    obj: &ObjectExpression<'a>,
    alloc: &'a Allocator,
) -> Option<HashMap<String, Expression<'a>>> {
    // `Some(literal)` once a name's *final* property is an inlinable literal;
    // `None` once its final property is something else (function, accessor,
    // non-literal value) — a later definition can demote or promote a name, so
    // track the latest verdict and filter at the end.
    let mut final_value: HashMap<String, Option<Expression<'a>>> = HashMap::new();
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            return None; // spread: unknown property set
        };
        if p.computed {
            return None; // computed key: unknown name
        }
        let name = static_key(&p.key)?;
        let verdict = if p.kind == PropertyKind::Init && is_inlineable_literal(&p.value) {
            Some(p.value.clone_in(alloc))
        } else {
            None
        };
        final_value.insert(name, verdict);
    }
    Some(
        final_value
            .into_iter()
            .filter_map(|(k, v)| v.map(|e| (k, e)))
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// 1. Write census (identical shape to `dead-object-prop`).
// ---------------------------------------------------------------------------

struct CandidateCollector<'s> {
    scoping: &'s Scoping,
    total_writes: HashMap<SymbolId, usize>,
    object_writes: HashMap<SymbolId, usize>,
}

impl<'s> CandidateCollector<'s> {
    fn record(&mut self, sym: SymbolId, is_object: bool) {
        *self.total_writes.entry(sym).or_default() += 1;
        if is_object {
            *self.object_writes.entry(sym).or_default() += 1;
        }
    }

    fn ref_sym(&self, id: &oxc_ast::ast::IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = id.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }
}

impl<'a, 's> Visit<'a> for CandidateCollector<'s> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (Some(sym), Some(init)) = (binding_sym(decl), &decl.init) {
            self.record(sym, matches!(init, Expression::ObjectExpression(_)));
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, decl);
    }

    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &node.left {
            if let Some(sym) = self.ref_sym(id) {
                let is_object = matches!(
                    node.operator,
                    oxc_syntax::operator::AssignmentOperator::Assign
                ) && matches!(node.right, Expression::ObjectExpression(_));
                self.record(sym, is_object);
            }
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }
}

// ---------------------------------------------------------------------------
// 2. First-statement object declarators + value snapshots.
// ---------------------------------------------------------------------------

struct FirstStmtSnapshots<'a> {
    alloc: &'a Allocator,
    values: HashMap<SymbolId, HashMap<String, Expression<'a>>>,
}

impl<'a> FirstStmtSnapshots<'a> {
    /// Inspect the first statement of a function/program body for an
    /// unconditional `var X = { …literals… }` and record its snapshot.
    fn consider_body(&mut self, stmts: &[Statement<'a>]) {
        let Some(Statement::VariableDeclaration(vd)) = stmts.first() else {
            return;
        };
        // Only the leading declarator runs before anything else; a later
        // declarator in the same `var` could be preceded by a side-effecting one.
        let Some(decl) = vd.declarations.first() else {
            return;
        };
        let (Some(sym), Some(Expression::ObjectExpression(obj))) = (binding_sym(decl), &decl.init)
        else {
            return;
        };
        if let Some(map) = snapshot(obj, self.alloc) {
            if !map.is_empty() {
                self.values.insert(sym, map);
            }
        }
    }
}

impl<'a> Visit<'a> for FirstStmtSnapshots<'a> {
    fn visit_program(&mut self, program: &Program<'a>) {
        self.consider_body(&program.body);
        oxc_ast_visit::walk::walk_program(self, program);
    }

    fn visit_function_body(&mut self, body: &oxc_ast::ast::FunctionBody<'a>) {
        self.consider_body(&body.statements);
        oxc_ast_visit::walk::walk_function_body(self, body);
    }
}

// ---------------------------------------------------------------------------
// 3. Escape analysis.
// ---------------------------------------------------------------------------

struct Classifier<'s, 'c> {
    scoping: &'s Scoping,
    candidates: &'c HashSet<SymbolId>,
    escaped: HashSet<SymbolId>,
}

impl<'s, 'c> Classifier<'s, 'c> {
    fn ref_sym(&self, id: &oxc_ast::ast::IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = id.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }

    /// If `obj` is a candidate identifier, mark it escaped. Used at write sites,
    /// where the static-member-read short-circuit would otherwise hide the object
    /// identifier from `visit_identifier_reference`.
    fn escape_if_candidate_obj(&mut self, obj: &Expression<'_>) {
        if let Expression::Identifier(id) = obj {
            if let Some(sym) = self.ref_sym(id) {
                if self.candidates.contains(&sym) {
                    self.escaped.insert(sym);
                }
            }
        }
    }
}

impl<'a, 's, 'c> Visit<'a> for Classifier<'s, 'c> {
    fn visit_static_member_expression(&mut self, m: &oxc_ast::ast::StaticMemberExpression<'a>) {
        // `O.name` in *read* position: a clean static read. Do NOT descend into
        // the object identifier (that would mark it escaped). Write positions are
        // handled by the assignment-target / update / delete hooks below, which
        // run before this short-circuit can hide them.
        if let Expression::Identifier(obj) = &m.object {
            if let Some(sym) = self.ref_sym(obj) {
                if self.candidates.contains(&sym) {
                    return;
                }
            }
        }
        oxc_ast_visit::walk::walk_static_member_expression(self, m);
    }

    fn visit_assignment_target(&mut self, target: &AssignmentTarget<'a>) {
        // `O.x = v`, `O.x += v`, `[O.x] = …`, `for (O.x of …)` — a static-member
        // write. (Computed `O[x] = v` and a bare-identifier rebind escape via the
        // normal walk into the object identifier.)
        if let AssignmentTarget::StaticMemberExpression(m) = target {
            self.escape_if_candidate_obj(&m.object);
        }
        oxc_ast_visit::walk::walk_assignment_target(self, target);
    }

    fn visit_update_expression(&mut self, u: &UpdateExpression<'a>) {
        // `O.x++`, `--O.x`.
        if let SimpleAssignmentTarget::StaticMemberExpression(m) = &u.argument {
            self.escape_if_candidate_obj(&m.object);
        }
        oxc_ast_visit::walk::walk_update_expression(self, u);
    }

    fn visit_unary_expression(&mut self, u: &UnaryExpression<'a>) {
        // `delete O.x` removes a property — a mutation.
        if u.operator == UnaryOperator::Delete {
            if let Expression::StaticMemberExpression(m) = &u.argument {
                self.escape_if_candidate_obj(&m.object);
            }
        }
        oxc_ast_visit::walk::walk_unary_expression(self, u);
    }

    fn visit_identifier_reference(&mut self, id: &oxc_ast::ast::IdentifierReference<'a>) {
        // Any reference not consumed as a static read above is an escape (bare
        // use, computed access, passing/returning, an identifier rebind, …).
        if let Some(sym) = self.ref_sym(id) {
            if self.candidates.contains(&sym) {
                self.escaped.insert(sym);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Replacement.
// ---------------------------------------------------------------------------

struct Replacer<'a, 'm> {
    values: &'m HashMap<SymbolId, HashMap<String, Expression<'a>>>,
    changed: bool,
}

impl<'a, 'm> Traverse<'a, ()> for Replacer<'a, 'm> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Member expressions only appear in read/value position here; a write
        // target `O.x = v` is an AssignmentTarget, not an Expression, so it is
        // never reached (and such a candidate has already escaped).
        let Expression::StaticMemberExpression(m) = expr else {
            return;
        };
        let Expression::Identifier(obj) = &m.object else {
            return;
        };
        let Some(rid) = obj.reference_id.get() else {
            return;
        };
        let Some(sym) = ctx.scoping().get_reference(rid).symbol_id() else {
            return;
        };
        let Some(val) = self
            .values
            .get(&sym)
            .and_then(|props| props.get(m.property.name.as_str()))
        else {
            return;
        };
        *expr = val.clone_in(ctx.ast.allocator);
        self.changed = true;
    }
}
