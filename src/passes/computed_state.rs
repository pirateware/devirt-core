//! Computed / object-state discriminant recognition (Plan A3, sample_4 shape).
//!
//! Some flatteners do not switch on a bare integer state variable. Instead the
//! discriminant is an **expression over a dispatch object and a counter**:
//!
//! ```text
//! var t = <state object>;          // prefix construction (may be several stmts)
//! do {
//!   switch (t + b) {               // discriminant = object `valueOf()` + counter
//!     case 100: …; b += 1; break;
//!     case 101: …; b += 1; break;
//!     …
//!   }
//! } while (<test>);
//! ```
//!
//! This pass **normalizes the deterministic subcase** so the downstream
//! unflattener / relooper can act on it: when the object provably coerces to a
//! *constant* numeric value `C` (a `{ valueOf: () => LIT }`, a `new Number(LIT)`,
//! or a plain numeric binding), then `t OP b === C OP b`, so the discriminant is a
//! pure function of the counter `b`. We rewrite:
//!
//!   * the discriminant `t + b`  →  `b`, and
//!   * every `case K:`           →  `case (K - C):`   (for `+`; mirrored for `-`)
//!
//! yielding a plain integer-state dispatcher over `b` that the existing passes
//! recover. The object binding `t` becomes dead and is dropped by DCE.
//!
//! **Soundness: it is a no-op unless the coercion is a provable constant.** A
//! per-read *mutating* `valueOf` (the real sample_4 object re-applies an opaque
//! transform on every coercion, so successive reads of `t` differ) is NOT a
//! constant; we DECLINE there — the discriminant's value depends on evaluation
//! history we do not model, and folding it would be unsound. We also decline on
//! any operator other than `+`/`-`, any non-literal case label, a `default`
//! clause, or a counter that is also read inside a case body in a way that would
//! change which constant we substitute. When in doubt, decline (leave the switch).
//!
//! ## The sample_4 wall (documented)
//!
//! sample_4's object is built by `QT(new Number(HH), cT)`: its `valueOf` does
//! `this[f] = cT(this[f])` — it **mutates and re-applies** the obfuscated transform
//! `cT` on every coercion. The discriminant is therefore a stateful stream, not a
//! function of the counter, so this recognizer declines on it (correctly). Folding
//! s4 needs abstract interpretation of `cT` (itself a proxy-laden loop) plus a
//! model of the object's coercion stream — out of scope for a sound, bounded pass.

use oxc_allocator::{Allocator, Vec as ArenaVec};
use oxc_ast::ast::*;
use oxc_ast::AstBuilder;
use oxc_ast_visit::Visit;
use oxc_span::SPAN;
use oxc_syntax::number::NumberBase;
use oxc_syntax::operator::BinaryOperator;

use crate::pass::Pass;

#[derive(Default)]
pub struct ComputedState {
    changed: bool,
}

impl Pass for ComputedState {
    fn name(&self) -> &'static str {
        "computed-state"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let ast = AstBuilder::new(allocator);
        let mut w = Walker {
            ast,
            changed: false,
        };
        w.lists(&mut program.body);
        self.changed = w.changed;
        self.changed
    }
}

struct Walker<'a> {
    ast: AstBuilder<'a>,
    changed: bool,
}

impl<'a> Walker<'a> {
    /// Recurse into every statement list, then try to normalize a computed-state
    /// dispatcher in this list. Mirrors `unflatten_cfg`'s hand-written recursion
    /// (a freshly-built switch has no `scope_id`; the oxc walker would panic).
    fn lists(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>) {
        for stmt in stmts.iter_mut() {
            self.recurse_stmt(stmt);
        }
        self.try_normalize(stmts);
    }

    fn try_normalize(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>) {
        // Look for: `var t = <obj>` (or any binding) followed later by a
        // do/while/for loop whose body switches on `t OP b`. The object binding `t`
        // must be referenced only by the discriminant (a private dispatch object).
        for i in 0..stmts.len() {
            let Some(obj_name) = single_binding_name(&stmts[i]) else {
                continue;
            };
            // The object's constant coercion value, or decline.
            let Some(const_val) = const_coercion(&stmts[i]) else {
                continue;
            };
            // Find the dispatcher loop that switches on `obj_name OP counter`.
            for j in (i + 1)..stmts.len() {
                let Some((op, counter)) = loop_switches_on(&stmts[j], &obj_name) else {
                    continue;
                };
                // `obj_name` must appear ONLY in the discriminant (so substituting
                // its constant value can't change any case body's behavior).
                let obj_uses: usize = (0..stmts.len())
                    .filter(|&k| k != i && mentions(&stmts[k], &obj_name))
                    .count();
                // Exactly one other statement mentions it — the dispatcher — and
                // within it only the discriminant (checked by `loop_switches_on`).
                if obj_uses != 1 {
                    break;
                }
                // The counter must not be the object itself.
                if counter == obj_name {
                    break;
                }
                // SOUNDNESS: `switch(C + b)` ≡ `switch(b)` (with offset labels) only
                // when `b` is a *number* (so `+` is numeric add, matching the `===`
                // switch on numeric labels). A string `b` would concat, not add.
                // Require the counter to be initialized to an integer literal before
                // the loop and only ever assigned integer-literal expressions — the
                // same "stays integer" invariant the integer-state unflattener uses.
                if !counter_is_integer(stmts, j, &counter) {
                    break;
                }
                // Rewrite the dispatcher in place.
                if self.rewrite_dispatcher(&mut stmts[j], op, const_val, &counter) {
                    // Drop the now-dead object binding `i`.
                    stmts.remove(i);
                    self.changed = true;
                    return;
                }
                break;
            }
        }
    }

    /// Rewrite `switch(obj OP counter) { case K: … }` to `switch(counter) { case
    /// (K-C or C-K): … }`. Returns false (no change) if any case label is not an
    /// integer literal or a `default` clause exists.
    fn rewrite_dispatcher(
        &self,
        stmt: &mut Statement<'a>,
        op: BinaryOperator,
        c: f64,
        counter: &str,
    ) -> bool {
        let switch = match switch_of_loop_mut(stmt) {
            Some(sw) => sw,
            None => return false,
        };
        // Validate every case label is an integer literal first (all-or-nothing).
        for case in &switch.cases {
            let Some(test) = &case.test else {
                return false; // default clause
            };
            if int_lit(test).is_none() {
                return false;
            }
        }
        // Substitute discriminant → counter; case K → case (new label).
        switch.discriminant = self.ast.expression_identifier(SPAN, self.ast.str(counter));
        for case in &mut switch.cases {
            let Some(test) = &case.test else { continue };
            let k = int_lit(test).expect("validated above");
            // discriminant value was `C OP counter == K` ⇒ counter == K - C (for +)
            // / C - K (for -). So the new label is that counter value.
            let new_label = match op {
                BinaryOperator::Addition => k - c,
                BinaryOperator::Subtraction => c - k,
                _ => return false,
            };
            if new_label.fract() != 0.0 || !new_label.is_finite() {
                return false;
            }
            case.test = Some(num_lit(new_label, self.ast));
        }
        true
    }

    fn recurse_stmt(&mut self, stmt: &mut Statement<'a>) {
        match stmt {
            Statement::BlockStatement(b) => self.lists(&mut b.body),
            Statement::FunctionDeclaration(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements);
                }
            }
            Statement::IfStatement(i) => {
                self.recurse_stmt(&mut i.consequent);
                if let Some(alt) = &mut i.alternate {
                    self.recurse_stmt(alt);
                }
            }
            Statement::ForStatement(f) => self.recurse_stmt(&mut f.body),
            Statement::WhileStatement(w) => self.recurse_stmt(&mut w.body),
            Statement::DoWhileStatement(d) => self.recurse_stmt(&mut d.body),
            Statement::LabeledStatement(l) => self.recurse_stmt(&mut l.body),
            Statement::TryStatement(t) => {
                self.lists(&mut t.block.body);
                if let Some(h) = &mut t.handler {
                    self.lists(&mut h.body.body);
                }
                if let Some(f) = &mut t.finalizer {
                    self.lists(&mut f.body);
                }
            }
            Statement::SwitchStatement(sw) => {
                for case in &mut sw.cases {
                    self.lists(&mut case.consequent);
                }
            }
            Statement::ExpressionStatement(es) => self.recurse_expr(&mut es.expression),
            Statement::VariableDeclaration(vd) => {
                for d in &mut vd.declarations {
                    if let Some(init) = &mut d.init {
                        self.recurse_expr(init);
                    }
                }
            }
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &mut r.argument {
                    self.recurse_expr(arg);
                }
            }
            _ => {}
        }
    }

    fn recurse_expr(&mut self, expr: &mut Expression<'a>) {
        match expr {
            Expression::FunctionExpression(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements);
                }
            }
            Expression::ArrowFunctionExpression(f) => self.lists(&mut f.body.statements),
            Expression::CallExpression(c) => {
                self.recurse_expr(&mut c.callee);
                for arg in &mut c.arguments {
                    if let Some(e) = arg.as_expression_mut() {
                        self.recurse_expr(e);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => self.recurse_expr(&mut p.expression),
            Expression::AssignmentExpression(a) => self.recurse_expr(&mut a.right),
            _ => {}
        }
    }
}

/// The single-declarator binding name of `var x = …;` (the dispatch object).
fn single_binding_name(stmt: &Statement<'_>) -> Option<String> {
    let Statement::VariableDeclaration(vd) = stmt else {
        return None;
    };
    if vd.declarations.len() != 1 {
        return None;
    }
    let d = &vd.declarations[0];
    let BindingPattern::BindingIdentifier(id) = &d.id else {
        return None;
    };
    d.init.as_ref()?; // must be initialized
    Some(id.name.to_string())
}

/// If the binding `var x = <init>` provably coerces to a *constant* numeric value,
/// return it. Recognized constant-coercing inits:
///   * `{ valueOf: function () { return <int literal>; } }` / arrow form
///   * `new Number(<int literal>)`
///   * a bare `<int literal>` (a plain numeric var)
///
/// A `valueOf` whose body does anything other than `return <int literal>` (e.g.
/// reads/writes `this`, calls a function) is NOT a constant → `None` (decline).
fn const_coercion(stmt: &Statement<'_>) -> Option<f64> {
    let Statement::VariableDeclaration(vd) = stmt else {
        return None;
    };
    let init = vd.declarations.first()?.init.as_ref()?;
    match init {
        Expression::NumericLiteral(n) => Some(n.value),
        Expression::NewExpression(ne) => {
            // new Number(LIT)
            if !is_global_ident(&ne.callee, "Number") {
                return None;
            }
            if ne.arguments.len() != 1 {
                return None;
            }
            let arg = ne.arguments[0].as_expression()?;
            num_value(arg)
        }
        Expression::ObjectExpression(obj) => const_valueof_object(obj),
        _ => None,
    }
}

/// `{ valueOf: () => LIT }` (or `function(){ return LIT; }`) with no other
/// enumerable behavior that affects coercion → the constant `LIT`. Any extra
/// property whose presence could change `+` coercion (e.g. a `Symbol.toPrimitive`,
/// or a `valueOf` that isn't a constant return) makes us decline.
fn const_valueof_object(obj: &ObjectExpression<'_>) -> Option<f64> {
    let mut value_of: Option<f64> = None;
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            return None; // spread → decline
        };
        let key = match &p.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(s) => s.value.as_str(),
            _ => return None, // computed key → decline
        };
        if key == "valueOf" || key == "toString" {
            let lit = const_return_fn(&p.value)?;
            if key == "valueOf" {
                value_of = Some(lit);
            }
        }
        // Other data properties are fine: they don't affect `+` coercion as long as
        // a constant `valueOf` is present. (If valueOf is absent we return None.)
    }
    value_of
}

/// A function/arrow whose entire body is `return <int literal>;` → the literal.
/// Anything else (multiple statements, `this`, calls, non-literal) → `None`.
fn const_return_fn(expr: &Expression<'_>) -> Option<f64> {
    let body_stmts: &[Statement] = match expr {
        Expression::FunctionExpression(f) => &f.body.as_ref()?.statements,
        Expression::ArrowFunctionExpression(a) => {
            // Expression-bodied arrow: `() => LIT`.
            if a.expression {
                let Statement::ExpressionStatement(es) = a.body.statements.first()? else {
                    return None;
                };
                return num_value(&es.expression);
            }
            &a.body.statements
        }
        _ => return None,
    };
    if body_stmts.len() != 1 {
        return None;
    }
    let Statement::ReturnStatement(r) = &body_stmts[0] else {
        return None;
    };
    num_value(r.argument.as_ref()?)
}

/// If `stmt` is a loop `do/while`/`while`/`for` whose body is `switch(obj OP
/// counter) {…}`, return the operator and the counter identifier name. The
/// discriminant operands must be exactly `obj` and a single counter identifier.
fn loop_switches_on(stmt: &Statement<'_>, obj: &str) -> Option<(BinaryOperator, String)> {
    let body = match stmt {
        Statement::DoWhileStatement(d) => &d.body,
        Statement::WhileStatement(w) => &w.body,
        Statement::ForStatement(f) => &f.body,
        _ => return None,
    };
    let switch = switch_of_body(body)?;
    let Expression::BinaryExpression(b) = &switch.discriminant else {
        return None;
    };
    if !matches!(
        b.operator,
        BinaryOperator::Addition | BinaryOperator::Subtraction
    ) {
        return None;
    }
    // One operand must be `obj`, the other a counter identifier (order: obj first
    // for the offset math; `K - C` vs `C - K` is operator-specific and we only
    // support `obj <op> counter`).
    let (Expression::Identifier(l), Expression::Identifier(r)) = (&b.left, &b.right) else {
        return None;
    };
    if l.name == obj && r.name != obj {
        Some((b.operator, r.name.to_string()))
    } else {
        None
    }
}

/// The switch inside a loop body (`switch{…}` or `{ switch{…} }`).
fn switch_of_body<'b, 'a>(body: &'b Statement<'a>) -> Option<&'b SwitchStatement<'a>> {
    match body {
        Statement::SwitchStatement(sw) => Some(sw),
        Statement::BlockStatement(b) if b.body.len() == 1 => {
            if let Statement::SwitchStatement(sw) = &b.body[0] {
                Some(sw)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Mutable access to the loop-body switch, for the rewrite.
fn switch_of_loop_mut<'b, 'a>(stmt: &'b mut Statement<'a>) -> Option<&'b mut SwitchStatement<'a>> {
    let body = match stmt {
        Statement::DoWhileStatement(d) => &mut d.body,
        Statement::WhileStatement(w) => &mut w.body,
        Statement::ForStatement(f) => &mut f.body,
        _ => return None,
    };
    match body {
        Statement::SwitchStatement(sw) => Some(sw),
        Statement::BlockStatement(b) if b.body.len() == 1 => {
            if let Statement::SwitchStatement(sw) = &mut b.body[0] {
                Some(sw)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_global_ident(expr: &Expression<'_>, name: &str) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name == name)
}

/// A numeric literal value (`5`, `-1`, `0x10`).
fn num_value(expr: &Expression<'_>) -> Option<f64> {
    match expr {
        Expression::NumericLiteral(n) => Some(n.value),
        Expression::UnaryExpression(u)
            if matches!(
                u.operator,
                oxc_syntax::operator::UnaryOperator::UnaryNegation
            ) =>
        {
            num_value(&u.argument).map(|v| -v)
        }
        _ => None,
    }
}

/// An *integer*-valued numeric literal (case labels must be integers).
fn int_lit(expr: &Expression<'_>) -> Option<f64> {
    let v = num_value(expr)?;
    (v.fract() == 0.0 && v.is_finite()).then_some(v)
}

fn num_lit<'a>(v: f64, ast: AstBuilder<'a>) -> Expression<'a> {
    if v < 0.0 {
        ast.expression_unary(
            SPAN,
            oxc_syntax::operator::UnaryOperator::UnaryNegation,
            ast.expression_numeric_literal(SPAN, -v, None, NumberBase::Decimal),
        )
    } else {
        ast.expression_numeric_literal(SPAN, v, None, NumberBase::Decimal)
    }
}

/// Prove the counter `name` stays an *integer*: it is initialized to an integer
/// literal in a statement before the loop (index `< loop_idx`), and *every* write
/// to it (anywhere in the list, including inside the cases) is an integer-literal
/// assignment (`= K`, `+= K`, `-= K`, `*= K`). Any other write (a string, a call,
/// a compound that could go non-integer) → not provably integer → decline.
fn counter_is_integer(stmts: &[Statement<'_>], loop_idx: usize, name: &str) -> bool {
    // 1. An integer-literal initializer before the loop.
    let mut inited = false;
    for stmt in stmts.iter().take(loop_idx) {
        if let Some(v) = init_value(stmt, name) {
            if v.fract() != 0.0 || !v.is_finite() {
                return false;
            }
            inited = true;
        }
    }
    if !inited {
        return false;
    }
    // 2. Every write to `name` (across the whole list) is integer-literal-valued.
    struct W<'r> {
        name: &'r str,
        ok: bool,
    }
    impl<'a> Visit<'a> for W<'_> {
        fn visit_assignment_expression(&mut self, a: &AssignmentExpression<'a>) {
            if let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left {
                if id.name == self.name {
                    use oxc_syntax::operator::AssignmentOperator as Op;
                    let numeric_op = matches!(
                        a.operator,
                        Op::Assign | Op::Addition | Op::Subtraction | Op::Multiplication
                    );
                    if !numeric_op || int_lit(&a.right).is_none() {
                        self.ok = false;
                    }
                }
            }
            oxc_ast_visit::walk::walk_assignment_expression(self, a);
        }
        fn visit_update_expression(&mut self, u: &UpdateExpression<'a>) {
            // `b++` / `b--` keep it integer — fine.
            oxc_ast_visit::walk::walk_update_expression(self, u);
        }
    }
    let mut w = W { name, ok: true };
    for stmt in stmts {
        w.visit_statement(stmt);
    }
    w.ok
}

/// If `stmt` initializes `name` (`var name = <num>` or `name = <num>`), its value.
fn init_value(stmt: &Statement<'_>, name: &str) -> Option<f64> {
    match stmt {
        Statement::VariableDeclaration(vd) => {
            for d in &vd.declarations {
                if let BindingPattern::BindingIdentifier(id) = &d.id {
                    if id.name == name {
                        return d.init.as_ref().and_then(num_value);
                    }
                }
            }
            None
        }
        Statement::ExpressionStatement(es) => {
            let Expression::AssignmentExpression(a) = &es.expression else {
                return None;
            };
            if !matches!(a.operator, oxc_syntax::operator::AssignmentOperator::Assign) {
                return None;
            }
            let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
                return None;
            };
            if id.name != name {
                return None;
            }
            num_value(&a.right)
        }
        _ => None,
    }
}

/// Does `stmt` reference `name` anywhere?
fn mentions(stmt: &Statement<'_>, name: &str) -> bool {
    struct M<'r> {
        name: &'r str,
        found: bool,
    }
    impl<'a> Visit<'a> for M<'_> {
        fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
            if it.name == self.name {
                self.found = true;
            }
        }
    }
    let mut m = M { name, found: false };
    m.visit_statement(stmt);
    m.found
}
