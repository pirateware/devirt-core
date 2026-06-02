//! Rename-by-role.
//!
//! Obfuscators use deliberately illegible identifiers (`WL`, `jK`, `_0x3a`).
//! We rename every *opaque* binding to a stable, unique, kind-tagged name
//! (`fn1`, `v1`, …) so scopes become navigable. This is done through the symbol
//! table: we map `SymbolId → new name` once, then rewrite binding identifiers
//! and identifier *references* (never property keys, which aren't references),
//! so all uses stay consistent and no distinct symbols are ever merged.
//!
//! Safety: globally-unique target names mean we can never introduce a collision
//! or accidental capture. Property accesses like `obj.WL` are untouched because
//! they are `IdentifierName`s, not references to the renamed symbol.

use std::collections::{HashMap, HashSet};

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingIdentifier, IdentifierReference, Program};
use oxc_syntax::symbol::{SymbolFlags, SymbolId};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct RenameByRole {
    changed: bool,
}

impl Pass for RenameByRole {
    fn name(&self) -> &'static str {
        "rename-by-role"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);

        // Existing names we must not collide with.
        let existing: HashSet<&str> = scoping.symbol_names().collect();
        let existing: HashSet<String> = existing.iter().map(|s| s.to_string()).collect();

        let mut rename: HashMap<SymbolId, String> = HashMap::new();
        let mut fn_n = 0usize;
        let mut var_n = 0usize;

        let root = scoping.root_scope_id();
        for sym in scoping.symbol_ids() {
            // Never rename root-scope (global / observable) bindings — they may
            // be the program's public entry point, like DCE's removal guard.
            if scoping.symbol_scope_id(sym) == root {
                continue;
            }
            let name = scoping.symbol_name(sym);
            if !is_opaque(name) {
                continue;
            }
            let is_fn = scoping.symbol_flags(sym).intersects(
                SymbolFlags::Function | SymbolFlags::Class,
            );
            let new = loop {
                // Targets must themselves be non-opaque (≥3 non-symbol chars)
                // or the pass would re-rename its own output forever.
                let candidate = if is_fn {
                    fn_n += 1;
                    format!("fn{fn_n}")
                } else {
                    var_n += 1;
                    format!("var{var_n}")
                };
                if !existing.contains(&candidate) && !rename.values().any(|v| v == &candidate) {
                    break candidate;
                }
            };
            rename.insert(sym, new);
        }

        if rename.is_empty() {
            return false;
        }

        let mut renamer = Renamer { rename: &rename, changed: false, _marker: Default::default() };
        let scoping = build_scoping(program);
        traverse_mut(&mut renamer, allocator, program, scoping, ());
        self.changed = renamer.changed;
        self.changed
    }
}

/// An identifier that reads as machine-generated: ≤ 2 chars, or a `_0x…` hex
/// name. Conservative on purpose — never touches plausibly-meaningful names.
fn is_opaque(name: &str) -> bool {
    name.starts_with("_0x") || name.chars().filter(|c| *c != '_' && *c != '$').count() <= 2
}

struct Renamer<'a, 'r> {
    rename: &'r HashMap<SymbolId, String>,
    changed: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'r> Traverse<'a, ()> for Renamer<'a, 'r> {
    fn enter_binding_identifier(
        &mut self,
        node: &mut BindingIdentifier<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        if let Some(sym) = node.symbol_id.get() {
            if let Some(new) = self.rename.get(&sym) {
                node.name = ctx.ast.ident(new);
                self.changed = true;
            }
        }
    }

    fn enter_identifier_reference(
        &mut self,
        node: &mut IdentifierReference<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let Some(rid) = node.reference_id.get() else {
            return;
        };
        if let Some(sym) = ctx.scoping().get_reference(rid).symbol_id() {
            if let Some(new) = self.rename.get(&sym) {
                node.name = ctx.ast.ident(new);
                self.changed = true;
            }
        }
    }
}
