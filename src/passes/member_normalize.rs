//! Example pass: `obj["prop"]` → `obj.prop` when `"prop"` is a valid identifier
//! name. This is the canonical "first pass" — it demonstrates the whole
//! node-replacement dance against oxc's arena AST (read what you need into
//! `Copy` locals, take the surviving child out with `take_in`, build the new
//! node via the `AstBuilder` in `ctx.ast`, overwrite in place) without needing
//! any symbol information.

use oxc_allocator::{Allocator, TakeIn};
use oxc_ast::ast::{
    ComputedMemberExpression, Expression, Program, SimpleAssignmentTarget, StaticMemberExpression,
};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::ident::is_identifier_name;
use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct MemberNormalize {
    changed: bool,
}

impl Pass for MemberNormalize {
    fn name(&self) -> &'static str {
        "member-normalize"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        // `()` is the per-traversal State; we don't need any. `traverse_mut`
        // needs a Scoping even though this pass ignores symbols.
        let scoping = build_scoping(program);
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

/// Rewrite `<obj>["ident"]` → `<obj>.ident`, returning the new node, or `None`
/// if `computed` is not a string-literal key that is a valid identifier name.
/// Shared by the read-position (`Expression`) and write-position
/// (`SimpleAssignmentTarget`) hooks so `a["x"] = a["x"]` normalizes on both
/// sides identically.
fn try_static<'a>(
    computed: &mut ComputedMemberExpression<'a>,
    ctx: &mut TraverseCtx<'a, ()>,
) -> Option<oxc_allocator::Box<'a, StaticMemberExpression<'a>>> {
    let (key_atom, key_span) = match &computed.expression {
        Expression::StringLiteral(key) if is_identifier_name(key.value.as_str()) => {
            (key.value, key.span) // Atom and Span are Copy
        }
        _ => return None,
    };
    let span = computed.span;
    let optional = computed.optional;
    let object = computed.object.take_in(ctx.ast);
    let property = ctx.ast.identifier_name(key_span, key_atom);
    Some(
        ctx.ast
            .alloc_static_member_expression(span, object, property, optional),
    )
}

impl<'a> Traverse<'a, ()> for MemberNormalize {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Match `<obj>[<string-literal>]` where the literal is an identifier.
        let Expression::ComputedMemberExpression(computed) = expr else {
            return;
        };
        if let Some(member) = try_static(computed, ctx) {
            *expr = Expression::StaticMemberExpression(member);
            self.changed = true;
        }
    }

    /// The LHS of an assignment (`a["x"] = ...`) is a `SimpleAssignmentTarget`,
    /// not an `Expression`, so `enter_expression` never sees it. Normalize it
    /// here too — otherwise `a["x"] = a.x` prints with mismatched access forms.
    fn enter_simple_assignment_target(
        &mut self,
        target: &mut SimpleAssignmentTarget<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let SimpleAssignmentTarget::ComputedMemberExpression(computed) = target else {
            return;
        };
        if let Some(member) = try_static(computed, ctx) {
            *target = SimpleAssignmentTarget::StaticMemberExpression(member);
            self.changed = true;
        }
    }
}
