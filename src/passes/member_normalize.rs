//! Example pass: `obj["prop"]` → `obj.prop` when `"prop"` is a valid identifier
//! name. This is the canonical "first pass" — it demonstrates the whole
//! node-replacement dance against oxc's arena AST (read what you need into
//! `Copy` locals, take the surviving child out with `take_in`, build the new
//! node via the `AstBuilder` in `ctx.ast`, overwrite in place) without needing
//! any symbol information.

use oxc_allocator::{Allocator, TakeIn};
use oxc_ast::ast::{Expression, Program};
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

impl<'a> Traverse<'a, ()> for MemberNormalize {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Match `<obj>[<string-literal>]` where the literal is an identifier.
        let Expression::ComputedMemberExpression(computed) = expr else {
            return;
        };
        let (key_atom, key_span) = match &computed.expression {
            Expression::StringLiteral(key) if is_identifier_name(key.value.as_str()) => {
                (key.value, key.span) // Atom and Span are Copy
            }
            _ => return,
        };

        let span = computed.span;
        let optional = computed.optional;
        // `take_in` swaps in a placeholder and hands us the owned object; this is
        // the last use of `computed`, so the borrow of `*expr` ends here and we
        // may overwrite it on the next line.
        let object = computed.object.take_in(ctx.ast);

        let property = ctx.ast.identifier_name(key_span, key_atom);
        *expr = Expression::StaticMemberExpression(
            ctx.ast.alloc_static_member_expression(span, object, property, optional),
        );
        self.changed = true;
    }
}
