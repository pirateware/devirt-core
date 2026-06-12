//! Split array-literal destructuring into one binding per element.
//!
//! `var [a, b, c] = [x, y, z]` is an array-destructuring binding whose right side
//! is itself an array literal. When the pattern and the literal line up one-to-one,
//! it is exactly `var a = x, b = y, c = z` — clearer, and it exposes each element to
//! constant folding (`var [a, b] = [10, 20]` → `var a = 10, b = 20` → folds). The
//! object form `var {a, b} = {a: 1, b: 2}` is handled likewise (see
//! `splittable_object` for its extra purity gate).
//!
//! Soundness — the rewrite must reproduce array-destructuring semantics exactly, so
//! it fires only on the unambiguous shape:
//!   * the pattern is all plain `BindingIdentifier` elements — no hole/elision, no
//!     default, no nested pattern, no `...rest`;
//!   * the initializer is an array literal with no spread and no hole, of the **same
//!     length** as the pattern (so no element's side effect is dropped or added);
//!   * no initializer element references a name the pattern binds (a write like
//!     `var [a, b] = [b, a]` would otherwise change meaning once the binding is
//!     per-element and sequential).
//!
//! Anything else is left untouched.

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{BindingPattern, Expression, Program, VariableDeclarator};
use oxc_ast::AstBuilder;
use oxc_ast_visit::{walk_mut, VisitMut};
use oxc_span::SPAN;

use crate::pass::Pass;

#[derive(Default)]
pub struct DestructureSplit {
    changed: bool,
}

impl Pass for DestructureSplit {
    fn name(&self) -> &'static str {
        "destructure-split"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let mut v = Splitter {
            ast: AstBuilder::new(allocator),
            changed: false,
        };
        walk_mut::walk_program(&mut v, program);
        self.changed = v.changed;
        self.changed
    }
}

struct Splitter<'a> {
    ast: AstBuilder<'a>,
    changed: bool,
}

impl<'a> VisitMut<'a> for Splitter<'a> {
    fn visit_variable_declaration(&mut self, decl: &mut oxc_ast::ast::VariableDeclaration<'a>) {
        if !decl.declarations.iter().any(splittable) {
            walk_mut::walk_variable_declaration(self, decl);
            return;
        }
        let kind = decl.kind;
        let old = std::mem::replace(&mut decl.declarations, self.ast.vec());
        for d in old {
            if splittable(&d) {
                self.expand(&d, kind, &mut decl.declarations);
                self.changed = true;
            } else {
                decl.declarations.push(d);
            }
        }
        walk_mut::walk_variable_declaration(self, decl);
    }
}

impl<'a> Splitter<'a> {
    /// Push one declarator per pattern element/property.
    fn expand(
        &self,
        d: &VariableDeclarator<'a>,
        kind: oxc_ast::ast::VariableDeclarationKind,
        out: &mut ArenaVec<'a, VariableDeclarator<'a>>,
    ) {
        match &d.id {
            BindingPattern::ArrayPattern(pat) => {
                let Some(Expression::ArrayExpression(arr)) = &d.init else {
                    return;
                };
                for (el, init_el) in pat.elements.iter().zip(arr.elements.iter()) {
                    // Both guaranteed by `splittable`: an id target, a real expression.
                    let (Some(BindingPattern::BindingIdentifier(id)), Some(init)) =
                        (el.as_ref(), init_el.as_expression())
                    else {
                        return;
                    };
                    self.push_decl(kind, id.name.as_str(), init, out);
                }
            }
            BindingPattern::ObjectPattern(pat) => {
                let Some(Expression::ObjectExpression(obj)) = &d.init else {
                    return;
                };
                for prop in &pat.properties {
                    let BindingPattern::BindingIdentifier(target) = &prop.value else {
                        return;
                    };
                    let Some(key) = static_key(&prop.key) else {
                        return;
                    };
                    let Some(init) = object_value(obj, &key) else {
                        return;
                    };
                    self.push_decl(kind, target.name.as_str(), init, out);
                }
            }
            _ => {}
        }
    }

    fn push_decl(
        &self,
        kind: oxc_ast::ast::VariableDeclarationKind,
        name: &str,
        init: &Expression<'a>,
        out: &mut ArenaVec<'a, VariableDeclarator<'a>>,
    ) {
        let binding = self
            .ast
            .binding_pattern_binding_identifier(SPAN, self.ast.str(name));
        out.push(self.ast.variable_declarator(
            SPAN,
            kind,
            binding,
            oxc_ast::NONE,
            Some(init.clone_in(self.ast.allocator)),
            false,
        ));
    }
}

/// The static key name of a binding/property key (`Identifier` or string literal).
fn static_key(key: &oxc_ast::ast::PropertyKey<'_>) -> Option<String> {
    use oxc_ast::ast::PropertyKey;
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// The value expression of object literal `obj`'s data property named `key`.
fn object_value<'b, 'a>(
    obj: &'b oxc_ast::ast::ObjectExpression<'a>,
    key: &str,
) -> Option<&'b Expression<'a>> {
    use oxc_ast::ast::ObjectPropertyKind;
    obj.properties.iter().find_map(|p| {
        let ObjectPropertyKind::ObjectProperty(op) = p else {
            return None;
        };
        (static_key(&op.key).as_deref() == Some(key)).then_some(&op.value)
    })
}

/// Does this declarator match a sound array- or object-split shape?
fn splittable(d: &VariableDeclarator<'_>) -> bool {
    match (&d.id, &d.init) {
        (BindingPattern::ArrayPattern(_), Some(Expression::ArrayExpression(_))) => {
            splittable_array(d)
        }
        (BindingPattern::ObjectPattern(_), Some(Expression::ObjectExpression(_))) => {
            splittable_object(d)
        }
        _ => false,
    }
}

/// `var [a, b] = [x, y]` — array literal, 1:1 with a plain-identifier pattern, no
/// spread/hole/rest, no initializer element referencing a bound name (swap hazard).
fn splittable_array(d: &VariableDeclarator<'_>) -> bool {
    let (BindingPattern::ArrayPattern(pat), Some(Expression::ArrayExpression(arr))) =
        (&d.id, &d.init)
    else {
        return false;
    };
    if pat.rest.is_some() || pat.elements.len() != arr.elements.len() {
        return false;
    }
    let mut names: Vec<&str> = Vec::with_capacity(pat.elements.len());
    for el in &pat.elements {
        match el {
            Some(BindingPattern::BindingIdentifier(id)) => names.push(id.name.as_str()),
            _ => return false,
        }
    }
    for el in &arr.elements {
        let Some(expr) = el.as_expression() else {
            return false;
        };
        if references_any(expr, &names) {
            return false;
        }
    }
    true
}

/// `var {a, b} = {a: 1, b: 2}` — object literal whose every value is **pure** (so
/// reordering pattern vs. literal order, and dropping an unmatched value, lose no
/// side effect). The pattern must be plain (id values, static non-computed keys, no
/// default/nested/rest), the literal all-static-key data properties (no spread /
/// computed / accessor / method / duplicate key), every pattern key present, and no
/// value referencing a bound name.
fn splittable_object(d: &VariableDeclarator<'_>) -> bool {
    use oxc_ast::ast::ObjectPropertyKind;
    let (BindingPattern::ObjectPattern(pat), Some(Expression::ObjectExpression(obj))) =
        (&d.id, &d.init)
    else {
        return false;
    };
    if pat.rest.is_some() {
        return false;
    }
    // The literal: all static-key data properties, no duplicates; values pure.
    let mut keys_seen: Vec<String> = Vec::with_capacity(obj.properties.len());
    for p in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(op) = p else {
            return false; // spread
        };
        if op.computed || op.method || op.kind != oxc_ast::ast::PropertyKind::Init {
            return false; // computed / method / accessor
        }
        let Some(k) = static_key(&op.key) else {
            return false;
        };
        if keys_seen.contains(&k) {
            return false; // duplicate key — which one wins is order-sensitive
        }
        keys_seen.push(k);
        if !crate::util::is_pure_simple(&op.value) {
            return false;
        }
    }
    // The pattern: plain identifier values, static keys, all present in the literal.
    let mut names: Vec<&str> = Vec::with_capacity(pat.properties.len());
    for prop in &pat.properties {
        if prop.computed {
            return false;
        }
        let BindingPattern::BindingIdentifier(id) = &prop.value else {
            return false; // default / nested
        };
        names.push(id.name.as_str());
        let Some(k) = static_key(&prop.key) else {
            return false;
        };
        if !keys_seen.contains(&k) {
            return false; // key absent → would bind `undefined`
        }
    }
    // No value may reference a name the pattern binds.
    !obj.properties.iter().any(|p| {
        matches!(p, ObjectPropertyKind::ObjectProperty(op) if references_any(&op.value, &names))
    })
}

/// Does `expr` reference any identifier in `names`?
fn references_any(expr: &Expression<'_>, names: &[&str]) -> bool {
    use oxc_ast_visit::Visit;
    struct R<'r> {
        names: &'r [&'r str],
        found: bool,
    }
    impl<'a, 'r> Visit<'a> for R<'r> {
        fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
            if self.names.contains(&it.name.as_str()) {
                self.found = true;
            }
        }
    }
    let mut r = R { names, found: false };
    r.visit_expression(expr);
    r.found
}
