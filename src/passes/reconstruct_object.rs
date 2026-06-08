//! Object-literal reconstruction.
//!
//! `transformObjectKeys` (and hand-written packers) lower an object literal into
//! an empty object plus a run of property writes:
//!
//! ```js
//! const O = {};
//! O.name = "P1";
//! O.price = 600;
//! ```
//!
//! This pass folds that contiguous run back into the literal it came from —
//! `const O = { name: "P1", price: 600 }`. That alone improves readability, but
//! the bigger payoff is downstream: obfuscators build their *operator-proxy
//! tables* the same way (`const t = {}; t.m = function(a,b){…}; …`), so
//! reconstruction is what lets `proxy_inline` recognize the table at all, which
//! in turn folds the opaque predicates guarding dead branches so DCE can remove
//! them.
//!
//! Soundness — we fold only when the rewrite cannot change observable behavior:
//!   * The seed is a single declarator `X = {}` with an *empty* object literal.
//!   * We consume only the *immediately following, contiguous* statements of the
//!     form `X.<staticName> = <expr>` (plain `=`, non-computed key). Contiguity
//!     guarantees nothing reads the half-built object between the writes.
//!   * The value expression must not reference `X` at all (even inside a nested
//!     function): in the literal, `X` is not yet bound, so a value that read `X`
//!     would change meaning. This is conservative (a closure-only reference would
//!     be safe) but always sound.
//!   * `__proto__` is excluded (its literal form has special prototype-setting
//!     semantics) and a repeated key stops the run (last-write-wins is preserved
//!     by not merging across a duplicate).
//!
//! Property values keep their original order, so any side effects in them run in
//! the same sequence as the original writes.

use std::collections::HashSet;

use oxc_allocator::{Allocator, Vec as ArenaVec};
use oxc_ast::ast::{
    AssignmentOperator, AssignmentTarget, Expression, ObjectPropertyKind, Program, PropertyKind,
    Statement,
};
use oxc_ast::AstBuilder;
use oxc_ast_visit::Visit;
use oxc_semantic::Scoping;
use oxc_syntax::symbol::SymbolId;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct ReconstructObject {
    changed: bool,
}

impl Pass for ReconstructObject {
    fn name(&self) -> &'static str {
        "reconstruct-object"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let scoping = build_scoping(program);
        self.changed = false;
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

/// Symbol bound by a `X = {}` seed statement, plus a mutable handle to its
/// (currently empty) object literal is obtained later; here we just classify.
fn seed_symbol(stmt: &Statement<'_>) -> Option<SymbolId> {
    let Statement::VariableDeclaration(vd) = stmt else {
        return None;
    };
    if vd.declarations.len() != 1 {
        return None;
    }
    let decl = &vd.declarations[0];
    let id = decl.id.get_binding_identifier()?;
    match &decl.init {
        Some(Expression::ObjectExpression(obj)) if obj.properties.is_empty() => id.symbol_id.get(),
        _ => None,
    }
}

/// If `stmt` is `seed.<name> = <expr>` (plain assign, static key) for `seed`,
/// return the property name. Read-only — used to validate a run before folding.
fn member_assign_key<'a>(
    stmt: &Statement<'a>,
    seed: SymbolId,
    scoping: &Scoping,
) -> Option<String> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let Expression::AssignmentExpression(a) = &es.expression else {
        return None;
    };
    if a.operator != AssignmentOperator::Assign {
        return None;
    }
    let AssignmentTarget::StaticMemberExpression(m) = &a.left else {
        return None;
    };
    let Expression::Identifier(obj) = &m.object else {
        return None;
    };
    let rid = obj.reference_id.get()?;
    if scoping.get_reference(rid).symbol_id() != Some(seed) {
        return None;
    }
    Some(m.property.name.to_string())
}

/// Does `expr` contain any reference resolving to `seed`?
fn references_symbol(expr: &Expression<'_>, seed: SymbolId, scoping: &Scoping) -> bool {
    struct Finder<'s> {
        seed: SymbolId,
        scoping: &'s Scoping,
        found: bool,
    }
    impl<'a, 's> Visit<'a> for Finder<'s> {
        fn visit_identifier_reference(&mut self, id: &oxc_ast::ast::IdentifierReference<'a>) {
            if let Some(rid) = id.reference_id.get() {
                if self.scoping.get_reference(rid).symbol_id() == Some(self.seed) {
                    self.found = true;
                }
            }
        }
    }
    let mut f = Finder {
        seed,
        scoping,
        found: false,
    };
    f.visit_expression(expr);
    f.found
}

impl<'a> Traverse<'a, ()> for ReconstructObject {
    fn enter_statements(
        &mut self,
        stmts: &mut ArenaVec<'a, Statement<'a>>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        // Cheap precheck: any empty-object seed at all?
        if !stmts.iter().any(|s| seed_symbol(s).is_some()) {
            return;
        }

        let n = stmts.len();
        let mut old: Vec<Option<Statement<'a>>> =
            std::mem::replace(stmts, ctx.ast.vec()).into_iter().map(Some).collect();

        let mut i = 0;
        while i < n {
            // Plan a fold using read-only borrows of `old` + scoping.
            let plan: Option<Vec<usize>> = seed_symbol(old[i].as_ref().unwrap()).and_then(|seed| {
                let scoping = ctx.scoping();
                let mut consumed = Vec::new();
                let mut keys = HashSet::new();
                let mut j = i + 1;
                while j < n {
                    let Some(key) = member_assign_key(old[j].as_ref().unwrap(), seed, scoping)
                    else {
                        break;
                    };
                    if key == "__proto__" || !keys.insert(key) {
                        break;
                    }
                    // value must not observe the not-yet-bound seed object.
                    let rhs = assign_rhs(old[j].as_ref().unwrap());
                    if references_symbol(rhs, seed, scoping) {
                        break;
                    }
                    consumed.push(j);
                    j += 1;
                }
                (!consumed.is_empty()).then_some(consumed)
            });

            match plan {
                Some(consumed) => {
                    let next = consumed.last().unwrap() + 1;
                    // Build the property list, then attach it to the seed's literal.
                    let mut props = ctx.ast.vec_with_capacity(consumed.len());
                    for cj in &consumed {
                        let stmt = old[*cj].take().unwrap();
                        props.push(take_member_property(stmt, ctx.ast));
                    }
                    let mut seed_stmt = old[i].take().unwrap();
                    if let Statement::VariableDeclaration(vd) = &mut seed_stmt {
                        if let Some(Expression::ObjectExpression(obj)) = &mut vd.declarations[0].init
                        {
                            obj.properties = props;
                        }
                    }
                    stmts.push(seed_stmt);
                    self.changed = true;
                    i = next;
                }
                None => {
                    stmts.push(old[i].take().unwrap());
                    i += 1;
                }
            }
        }
    }
}

fn assign_rhs<'b, 'a>(stmt: &'b Statement<'a>) -> &'b Expression<'a> {
    let Statement::ExpressionStatement(es) = stmt else {
        unreachable!()
    };
    let Expression::AssignmentExpression(a) = &es.expression else {
        unreachable!()
    };
    &a.right
}

/// Consume a validated `seed.name = value` statement into the object property
/// `name: value`. Reuses the member key's arena-allocated identifier directly.
fn take_member_property<'a>(stmt: Statement<'a>, ast: AstBuilder<'a>) -> ObjectPropertyKind<'a> {
    let Statement::ExpressionStatement(es) = stmt else {
        unreachable!()
    };
    let Expression::AssignmentExpression(a) = es.unbox().expression else {
        unreachable!()
    };
    let a = a.unbox();
    let AssignmentTarget::StaticMemberExpression(m) = a.left else {
        unreachable!()
    };
    let m = m.unbox();
    let span = m.span;
    let key = ast.property_key_static_identifier(span, m.property.name);
    ast.object_property_kind_object_property(span, PropertyKind::Init, key, a.right, false, false, false)
}
