//! Split array-literal destructuring into one binding per element.
//!
//! `var [a, b, c] = [x, y, z]` is an array-destructuring binding whose right side
//! is itself an array literal. When the pattern and the literal line up one-to-one,
//! it is exactly `var a = x, b = y, c = z` — clearer, and it exposes each element to
//! constant folding (`var [a, b] = [10, 20]` → `var a = 10, b = 20` → folds).
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
    /// Push one `name = element` declarator per pattern element.
    fn expand(
        &self,
        d: &VariableDeclarator<'a>,
        kind: oxc_ast::ast::VariableDeclarationKind,
        out: &mut ArenaVec<'a, VariableDeclarator<'a>>,
    ) {
        let BindingPattern::ArrayPattern(pat) = &d.id else {
            return;
        };
        let Some(Expression::ArrayExpression(arr)) = &d.init else {
            return;
        };
        for (el, init_el) in pat.elements.iter().zip(arr.elements.iter()) {
            // Both guaranteed by `splittable`: a plain id target, a real expression.
            let (Some(BindingPattern::BindingIdentifier(id)), Some(init)) =
                (el.as_ref(), init_el.as_expression())
            else {
                return;
            };
            let binding = self
                .ast
                .binding_pattern_binding_identifier(SPAN, id.name);
            let new = self.ast.variable_declarator(
                SPAN,
                kind,
                binding,
                oxc_ast::NONE,
                Some(init.clone_in(self.ast.allocator)),
                false,
            );
            out.push(new);
        }
    }
}

/// Does this declarator match the sound array-split shape (see the module docs)?
fn splittable(d: &VariableDeclarator<'_>) -> bool {
    let BindingPattern::ArrayPattern(pat) = &d.id else {
        return false;
    };
    let Some(Expression::ArrayExpression(arr)) = &d.init else {
        return false;
    };
    if pat.rest.is_some() || pat.elements.len() != arr.elements.len() {
        return false;
    }
    // Pattern: every element a plain binding identifier (no hole/default/nested).
    let mut names: Vec<&str> = Vec::with_capacity(pat.elements.len());
    for el in &pat.elements {
        match el {
            Some(BindingPattern::BindingIdentifier(id)) => names.push(id.name.as_str()),
            _ => return false,
        }
    }
    // Initializer: every element a real expression (`as_expression` is `None` for a
    // spread or a hole), and none may reference a name the pattern binds (the swap
    // hazard).
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
