//! Remove never-accessed properties from a non-escaping object literal.
//!
//! After ProxyInline rewrites every `tbl.op(a, b)` call, an operator/decoder
//! table that *also* holds a live data field keeps its now-uncalled method
//! definitions: DCE can drop a fully-dead object but not individual dead
//! members. This pass closes that gap — for an object literal bound to a local
//! that is only ever read as `O.staticName`, any literal property whose name is
//! never read (and whose value is pure to construct) is dropped.
//!
//! Soundness is by construction — we default to *keeping*:
//!   * A candidate is a non-root, never-reassigned binding initialized with an
//!     object literal.
//!   * Escape analysis treats every reference to the binding as disqualifying
//!     UNLESS it is the object of a *static* member read (`O.name`). Any other
//!     shape — `O[x]`, bare `O`, `O.x = v` (the object read in target
//!     position), destructuring, passing `O` — leaves the identifier to reach
//!     `visit_identifier_reference`, which marks the object escaped. An escaped
//!     object is never touched.
//!   * Only `Init` properties with non-computed keys and pure values are
//!     removable; their construction has no side effect, so dropping an
//!     unaccessed one changes nothing.

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPattern, Expression, ObjectPropertyKind, Program, PropertyKey, VariableDeclarator,
};
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::{build_scoping, is_pure_simple};

#[derive(Default)]
pub struct DeadObjectProp {
    changed: bool,
}

impl Pass for DeadObjectProp {
    fn name(&self) -> &'static str {
        "dead-object-prop"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        // 1. Candidate object-literal bindings: non-root, never reassigned.
        let mut cand = CandidateCollector { scoping: &scoping, candidates: HashSet::new() };
        cand.visit_program(program);
        if cand.candidates.is_empty() {
            return false;
        }

        // 2. Classify every reference: record static-read accesses, escape all else.
        let mut cls = Classifier {
            scoping: &scoping,
            candidates: &cand.candidates,
            accessed: HashMap::new(),
            escaped: HashSet::new(),
        };
        cls.visit_program(program);

        // 3. A binding is rewritable if it did not escape.
        let live: HashSet<SymbolId> =
            cand.candidates.iter().copied().filter(|s| !cls.escaped.contains(s)).collect();
        if live.is_empty() {
            return false;
        }

        // 4. Drop unaccessed pure properties from each live object literal.
        let mut rm = Remover {
            live: &live,
            accessed: &cls.accessed,
            changed: false,
            _marker: std::marker::PhantomData,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut rm, allocator, program, scoping, ());
        self.changed = rm.changed;
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

struct CandidateCollector<'s> {
    scoping: &'s Scoping,
    candidates: HashSet<SymbolId>,
}

impl<'a, 's> Visit<'a> for CandidateCollector<'s> {
    fn visit_variable_declarator(&mut self, decl: &VariableDeclarator<'a>) {
        if let (Some(sym), Some(Expression::ObjectExpression(_))) = (binding_sym(decl), &decl.init) {
            // Non-root (a global may be read externally) and never reassigned.
            if self.scoping.symbol_scope_id(sym) != self.scoping.root_scope_id()
                && !self.scoping.symbol_is_mutated(sym)
            {
                self.candidates.insert(sym);
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, decl);
    }
}

struct Classifier<'s, 'c> {
    scoping: &'s Scoping,
    candidates: &'c HashSet<SymbolId>,
    accessed: HashMap<SymbolId, HashSet<String>>,
    escaped: HashSet<SymbolId>,
}

impl<'s, 'c> Classifier<'s, 'c> {
    fn ref_sym(&self, id: &oxc_ast::ast::IdentifierReference<'_>) -> Option<SymbolId> {
        let rid = id.reference_id.get()?;
        self.scoping.get_reference(rid).symbol_id()
    }
}

impl<'a, 's, 'c> Visit<'a> for Classifier<'s, 'c> {
    fn visit_static_member_expression(&mut self, m: &oxc_ast::ast::StaticMemberExpression<'a>) {
        // `O.name` where O is a candidate: a clean static read. Record the name
        // as accessed and do NOT descend into the object identifier (descending
        // would route it through visit_identifier_reference and mark it escaped).
        if let Expression::Identifier(obj) = &m.object {
            if let Some(sym) = self.ref_sym(obj) {
                if self.candidates.contains(&sym) {
                    self.accessed.entry(sym).or_default().insert(m.property.name.to_string());
                    return;
                }
            }
        }
        oxc_ast_visit::walk::walk_static_member_expression(self, m);
    }

    fn visit_identifier_reference(&mut self, id: &oxc_ast::ast::IdentifierReference<'a>) {
        // Any reference to a candidate that was not consumed as a static read
        // above is an escape (bare use, computed access, assignment target, …).
        if let Some(sym) = self.ref_sym(id) {
            if self.candidates.contains(&sym) {
                self.escaped.insert(sym);
            }
        }
    }
}

struct Remover<'a, 'r> {
    live: &'r HashSet<SymbolId>,
    accessed: &'r HashMap<SymbolId, HashSet<String>>,
    changed: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'r> Traverse<'a, ()> for Remover<'a, 'r> {
    fn enter_variable_declarator(
        &mut self,
        decl: &mut VariableDeclarator<'a>,
        _ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let Some(sym) = binding_sym(decl) else { return };
        if !self.live.contains(&sym) {
            return;
        }
        let Some(Expression::ObjectExpression(obj)) = &mut decl.init else {
            return;
        };
        let empty = HashSet::new();
        let accessed = self.accessed.get(&sym).unwrap_or(&empty);
        let before = obj.properties.len();
        obj.properties.retain(|prop| keep_property(prop, accessed));
        if obj.properties.len() != before {
            self.changed = true;
        }
    }
}

/// Keep a property unless it is a removable dead member: a non-computed `Init`
/// property with a pure value whose name is never accessed.
fn keep_property(prop: &ObjectPropertyKind<'_>, accessed: &HashSet<String>) -> bool {
    let ObjectPropertyKind::ObjectProperty(p) = prop else {
        return true; // spread: keep
    };
    if p.computed || p.kind != oxc_ast::ast::PropertyKind::Init {
        return true; // computed key or getter/setter: keep
    }
    let Some(name) = static_key(&p.key) else {
        return true;
    };
    if accessed.contains(&name) {
        return true;
    }
    // Dead name; only drop if constructing the value has no side effect.
    !is_pure_simple(&p.value)
}
