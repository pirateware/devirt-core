//! Lower a private object's integer *state* property to a local variable.
//!
//! Some dispatchers keep their control-flow state on an object property —
//! `var ctx = { s: 1, … }; while (ctx.s) switch (ctx.s) { … ctx.s = K … }` — which
//! the identifier-based unflatteners cannot recognize. When the object is provably
//! private we rewrite `ctx.s` to a fresh local `_state`, turning the machine into
//! the bare-identifier form the unflatteners already handle.
//!
//! Soundness — replacing every `ctx.s` with a local slot is equivalent only when no
//! one else can observe that property. We require, program-wide:
//!   * `ctx` is a single-declarator `var ctx = { … }` object literal, never
//!     reassigned, and its name denotes exactly one symbol (no shadowing), so a
//!     name-based rewrite cannot hit the wrong binding;
//!   * `s` is a plain data property with an integer-literal initial value (no
//!     getter/setter/computed key/spread);
//!   * **every** use of `ctx` is a static member access `ctx.<x>` — never bare
//!     (escape), never computed (`ctx[k]`), never the operand of `delete`. So the
//!     object cannot leak and the `s` slot cannot be aliased or removed.
//!
//! Under those gates `ctx.s` is a private integer cell; a fresh `var _state = init`
//! initialized to the literal, with every `ctx.s` read/write retargeted to it,
//! preserves behavior exactly (and the now-`s`-less object is cleaned by DCE if it
//! falls unused). Declines on any doubt.

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast::AstBuilder;
use oxc_ast_visit::{walk_mut, Visit, VisitMut};
use oxc_span::SPAN;

use crate::pass::Pass;
use crate::util::build_scoping;

pub struct ObjStateLower;

impl Pass for ObjStateLower {
    fn name(&self) -> &'static str {
        "objstate-lower"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // One candidate per run; the fixpoint repeats for further objects.
        let Some(plan) = self.find_candidate(program) else {
            return false;
        };
        let fresh = fresh_name(program, &plan.obj);
        let mut rw = Rewriter {
            ast: AstBuilder::new(allocator),
            obj: plan.obj,
            prop: plan.prop,
            init: plan.init,
            fresh,
        };
        walk_mut::walk_program(&mut rw, program);
        true
    }
}

/// A validated lowering target: the object name, its state property, and the
/// property's integer initial value.
struct Plan {
    obj: String,
    prop: String,
    init: f64,
}

impl ObjStateLower {
    /// Find the first `var obj = { prop: <int>, … }` whose object passes the
    /// privacy gate, choosing `prop` as a state-property candidate.
    fn find_candidate(&self, program: &Program<'_>) -> Option<Plan> {
        let scoping = build_scoping(program);
        for cand in collect_object_state_props(program) {
            // Only lower a property actually used as a dispatcher state: it must be
            // the discriminant of some `switch (obj.prop)`. This keeps the pass to
            // its purpose and leaves ordinary private object data untouched.
            if !is_switch_state(program, &cand.obj, &cand.prop) {
                continue;
            }
            // The name must denote exactly one symbol (no shadowing) for a
            // name-based rewrite to be sound.
            if scoping.symbol_names().filter(|n| *n == cand.obj).count() != 1 {
                continue;
            }
            let mut g = Gate {
                obj: &cand.obj,
                decl_start: cand.decl_start,
                base_depth: cand.base_depth,
                depth: 0,
                refs: 0,
                member_objs: 0,
                bad: false,
            };
            g.visit_program(program);
            // Sound when every `obj` use is a static-member access (no escape),
            // none precede the declaration (no read-before-init), and none sit in a
            // nested function (no closure that could run before init).
            if !g.bad && g.refs > 0 && g.refs == g.member_objs {
                return Some(Plan {
                    obj: cand.obj,
                    prop: cand.prop,
                    init: cand.init,
                });
            }
        }
        None
    }
}

/// A raw candidate from the declaration scan: the object/property/init plus the
/// declaration's source position and function-nesting depth, used by the gate to
/// reject reads-before-init and closure captures.
struct Cand {
    obj: String,
    prop: String,
    init: f64,
    decl_start: u32,
    base_depth: u32,
}

/// Every `var obj = { prop: <int literal>, … }` (single declarator, plain data
/// property), with the declaration's position and function depth. Multiple int
/// props yield multiple candidates; the gate picks the private, in-scope one.
fn collect_object_state_props(program: &Program<'_>) -> Vec<Cand> {
    struct C {
        out: Vec<Cand>,
        depth: u32,
    }
    impl<'a> Visit<'a> for C {
        fn visit_function(&mut self, f: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
            self.depth += 1;
            oxc_ast_visit::walk::walk_function(self, f, flags);
            self.depth -= 1;
        }
        fn visit_arrow_function_expression(&mut self, f: &ArrowFunctionExpression<'a>) {
            self.depth += 1;
            oxc_ast_visit::walk::walk_arrow_function_expression(self, f);
            self.depth -= 1;
        }
        fn visit_variable_declarator(&mut self, d: &VariableDeclarator<'a>) {
            if let (BindingPattern::BindingIdentifier(id), Some(Expression::ObjectExpression(obj))) =
                (&d.id, d.init.as_ref())
            {
                for p in &obj.properties {
                    if let ObjectPropertyKind::ObjectProperty(op) = p {
                        if op.kind == PropertyKind::Init && !op.computed && !op.method {
                            if let (Some(key), Some(val)) =
                                (static_key(&op.key), int_of(&op.value))
                            {
                                self.out.push(Cand {
                                    obj: id.name.to_string(),
                                    prop: key,
                                    init: val,
                                    decl_start: d.span.start,
                                    base_depth: self.depth,
                                });
                            }
                        }
                    }
                }
            }
            oxc_ast_visit::walk::walk_variable_declarator(self, d);
        }
    }
    let mut c = C {
        out: Vec::new(),
        depth: 0,
    };
    c.visit_program(program);
    c.out
}

/// Is `obj.prop` the discriminant of some `switch` statement? (The dispatcher
/// shape that makes this property a control-flow state worth lowering.)
fn is_switch_state(program: &Program<'_>, obj: &str, prop: &str) -> bool {
    struct S<'r> {
        obj: &'r str,
        prop: &'r str,
        found: bool,
    }
    impl<'a, 'r> Visit<'a> for S<'r> {
        fn visit_switch_statement(&mut self, sw: &SwitchStatement<'a>) {
            if let Expression::StaticMemberExpression(m) = &sw.discriminant {
                if m.property.name == self.prop
                    && matches!(&m.object, Expression::Identifier(id) if id.name == self.obj)
                {
                    self.found = true;
                }
            }
            oxc_ast_visit::walk::walk_switch_statement(self, sw);
        }
    }
    let mut s = S {
        obj,
        prop,
        found: false,
    };
    s.visit_program(program);
    s.found
}

/// The static key name of a property key (`Identifier` or string literal).
fn static_key(key: &PropertyKey<'_>) -> Option<String> {
    match key {
        PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
        PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
        _ => None,
    }
}

/// An integer-valued numeric literal expression.
fn int_of(expr: &Expression<'_>) -> Option<f64> {
    match expr {
        Expression::NumericLiteral(n) if n.value.fract() == 0.0 && n.value.is_finite() => {
            Some(n.value)
        }
        _ => None,
    }
}

/// Privacy gate: scans every use of `obj`. `member_objs` counts uses that are the
/// object of a *static* member access; `refs` counts all identifier references.
/// `bad` is set by a computed access, a `delete` on a member, or an assignment to
/// `obj` itself — anything that lets the object or its slot escape/alias.
struct Gate<'r> {
    obj: &'r str,
    decl_start: u32,
    base_depth: u32,
    depth: u32,
    refs: usize,
    member_objs: usize,
    bad: bool,
}

impl<'a, 'r> Visit<'a> for Gate<'r> {
    fn visit_function(&mut self, f: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        self.depth += 1;
        oxc_ast_visit::walk::walk_function(self, f, flags);
        self.depth -= 1;
    }
    fn visit_arrow_function_expression(&mut self, f: &ArrowFunctionExpression<'a>) {
        self.depth += 1;
        oxc_ast_visit::walk::walk_arrow_function_expression(self, f);
        self.depth -= 1;
    }

    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        if id.name == self.obj {
            self.refs += 1;
            // Read-before-init: a use textually before the declaration would observe
            // `undefined` on a local but throw on the (still-undefined) object.
            if id.span.start < self.decl_start {
                self.bad = true;
            }
            // Closure capture: a use in a nested function could run before the
            // declaration executes; only same-level uses keep textual order = exec.
            if self.depth != self.base_depth {
                self.bad = true;
            }
        }
    }

    fn visit_static_member_expression(&mut self, m: &StaticMemberExpression<'a>) {
        if matches!(&m.object, Expression::Identifier(id) if id.name == self.obj) {
            self.member_objs += 1;
        }
        oxc_ast_visit::walk::walk_static_member_expression(self, m);
    }

    fn visit_computed_member_expression(&mut self, m: &ComputedMemberExpression<'a>) {
        if matches!(&m.object, Expression::Identifier(id) if id.name == self.obj) {
            self.bad = true; // obj[k] — could alias the state slot
        }
        oxc_ast_visit::walk::walk_computed_member_expression(self, m);
    }

    fn visit_unary_expression(&mut self, u: &UnaryExpression<'a>) {
        if u.operator == oxc_syntax::operator::UnaryOperator::Delete {
            if let Expression::StaticMemberExpression(m) = &u.argument {
                if matches!(&m.object, Expression::Identifier(id) if id.name == self.obj) {
                    self.bad = true; // delete obj.s — removes the slot
                }
            }
        }
        oxc_ast_visit::walk::walk_unary_expression(self, u);
    }

    fn visit_assignment_target(&mut self, t: &AssignmentTarget<'a>) {
        // A write to `obj` itself (reassignment) breaks the single-init assumption.
        if let AssignmentTarget::AssignmentTargetIdentifier(id) = t {
            if id.name == self.obj {
                self.bad = true;
            }
        }
        oxc_ast_visit::walk::walk_assignment_target(self, t);
    }
}

/// A program-wide unique `_state`-style name.
fn fresh_name(program: &Program<'_>, _obj: &str) -> String {
    let scoping = build_scoping(program);
    let taken: std::collections::HashSet<String> =
        scoping.symbol_names().map(|s| s.to_string()).collect();
    let mut i = 0usize;
    loop {
        let cand = if i == 0 {
            "_state".to_string()
        } else {
            format!("_state{i}")
        };
        if !taken.contains(&cand) {
            return cand;
        }
        i += 1;
    }
}

/// Rewrites every `obj.prop` to the fresh local, removes `prop` from the object
/// literal, and adds a `fresh = init` declarator beside the object declaration.
struct Rewriter<'a> {
    ast: AstBuilder<'a>,
    obj: String,
    prop: String,
    init: f64,
    fresh: String,
}

impl<'a> Rewriter<'a> {
    fn is_obj_prop(&self, m: &StaticMemberExpression<'a>) -> bool {
        m.property.name == self.prop.as_str()
            && matches!(&m.object, Expression::Identifier(id) if id.name == self.obj.as_str())
    }
}

impl<'a> VisitMut<'a> for Rewriter<'a> {
    fn visit_expression(&mut self, expr: &mut Expression<'a>) {
        if let Expression::StaticMemberExpression(m) = expr {
            if self.is_obj_prop(m) {
                *expr = self
                    .ast
                    .expression_identifier(SPAN, self.ast.str(&self.fresh));
                return;
            }
        }
        walk_mut::walk_expression(self, expr);
    }

    fn visit_assignment_target(&mut self, target: &mut AssignmentTarget<'a>) {
        if let AssignmentTarget::StaticMemberExpression(m) = target {
            if self.is_obj_prop(m) {
                *target = AssignmentTarget::AssignmentTargetIdentifier(
                    self.ast
                        .alloc(self.ast.identifier_reference(SPAN, self.ast.str(&self.fresh))),
                );
                return;
            }
        }
        walk_mut::walk_assignment_target(self, target);
    }

    fn visit_variable_declaration(&mut self, decl: &mut VariableDeclaration<'a>) {
        let mut add: Option<VariableDeclarationKind> = None;
        for d in decl.declarations.iter_mut() {
            let is_target = matches!(&d.id,
                BindingPattern::BindingIdentifier(id) if id.name == self.obj.as_str());
            if !is_target {
                continue;
            }
            if let Some(Expression::ObjectExpression(obj)) = d.init.as_mut() {
                obj.properties.retain(|p| {
                    !matches!(p, ObjectPropertyKind::ObjectProperty(op)
                        if static_key(&op.key).as_deref() == Some(self.prop.as_str()))
                });
                add = Some(decl.kind);
            }
        }
        if let Some(kind) = add {
            // `var obj = {…}` → `var obj = {…}, _state = <init>`.
            let pat = self
                .ast
                .binding_pattern_binding_identifier(SPAN, self.ast.str(&self.fresh));
            let init = self.ast.expression_numeric_literal(
                SPAN,
                self.init,
                None,
                oxc_syntax::number::NumberBase::Decimal,
            );
            let new_decl =
                self.ast
                    .variable_declarator(SPAN, kind, pat, oxc_ast::NONE, Some(init), false);
            decl.declarations.push(new_decl);
        }
        walk_mut::walk_variable_declaration(self, decl);
    }
}
