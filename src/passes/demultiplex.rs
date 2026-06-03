//! Constant-selector function-array de-multiplexing.
//!
//! The reCAPTCHA/BotGuard-class obfuscation builds one immutable array of
//! *multiplexer* functions and indexes it by a constant. Each slot's first
//! parameter is a magic **selector** that picks which guarded block runs:
//!
//! ```text
//! var A = (function () { return [ function (s, a, b) { … }, … ]; })();
//! A[46](22, x, y);                  // constant index 46, constant selector 22
//! A[3].call(this, 30, x);           // selector 30, this preserved
//! ```
//!
//! When the array is immutable and the call passes a constant index `k` and a
//! constant selector `C`, slot `k`'s live branch is statically determined. We
//! *specialize* slot `k` for selector `C`: emit a copy of the slot with its
//! selector parameter bound to `C`, and route the call through it. Binding the
//! selector turns the `(s-1|15)>=s`-style guards into constants, so the existing
//! const-fold / flatten-else / DCE passes collapse the nested ternary / comma /
//! `==C && (…)` guard chains to just the code that runs.
//!
//! ## How the selector is bound (no manual substitution)
//!
//! Rather than substitute the selector identifier (a missed occurrence would
//! leave a dangling reference), we **drop parameter 0 and prepend
//! `var <selector> = C;`** to the body. This is exactly what calling the slot
//! with `C` as its first argument does — binding the selector to `C` then running
//! the body verbatim — so it is sound even when the body reassigns the selector.
//! The downstream const-prop / const-fold passes then propagate `C` and fold the
//! guards.
//!
//! ## Soundness
//!
//! * **Immutable array, name-precise.** `A` is declared exactly once (unique
//!   name), value-written exactly once (its array/IIFE initializer), and never
//!   element-written, reassigned, `delete`d, or method-mutated. A bare read does
//!   not block us — we keep the original array and only *add* specialized copies.
//! * **Constant index and selector.** `k` is a non-negative integer literal in
//!   range; `C` is a (non-negative) numeric literal — no side effect to drop.
//! * **`this` preserved.** The call is rewritten to `SPEC.call(receiver, args)` —
//!   `A` for a direct `A[k](…)` call (whose `this` is `A`), or the original
//!   receiver for `A[k].call(recv, …)`.
//! * **Effect order preserved.** Trailing argument nodes are *moved* (not cloned
//!   or reordered); the dropped selector literal has no effect; the `A`/`A[k]`
//!   reads are side-effect-free (immutable array).
//! * **`arguments`-safe.** Dropping parameter 0 shifts `arguments` indices, so a
//!   slot referencing `arguments` is left untouched.
//! * **Bounded.** Each `(A, k, C)` yields exactly one specialized function (deduped
//!   by name, skipped if it already exists), so the fixpoint terminates even for
//!   self-referential slots.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::*;
use oxc_ast::{AstBuilder, NONE};
use oxc_ast_visit::Visit;
use oxc_span::SPAN;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

/// Safety valve against pathological cascades; coverage beyond this is logged.
const MAX_SPECIALIZATIONS: usize = 4000;

pub struct Demultiplex;

impl Pass for Demultiplex {
    fn name(&self) -> &'static str {
        "demultiplex"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        let arrays = collect_immutable_fn_arrays(program, allocator);
        if arrays.is_empty() {
            return false;
        }
        let existing = collect_binding_names(program);

        let mut rw = Rewriter {
            arrays: &arrays,
            existing,
            emitted: HashMap::new(),
            new_defs: Vec::new(),
            count: 0,
            truncated: false,
            changed: false,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut rw, allocator, program, scoping, ());

        if std::env::var("DEMUX_DEBUG").is_ok() {
            // Coverage (no silent truncation): how many constant-selector sites were
            // specialized vs. capped this round, over how many immutable arrays.
            eprintln!(
                "[demux] specialized={} arrays={} truncated={}",
                rw.new_defs.len(),
                arrays.len(),
                rw.truncated
            );
        }
        if !rw.changed {
            return false;
        }

        // Append each specialized function declaration into its array's region, so
        // it is hoisted in a scope where the array is visible.
        let mut by_region: HashMap<Region, Vec<Statement<'a>>> = HashMap::new();
        for (region, stmt) in rw.new_defs {
            by_region.entry(region).or_default().push(stmt);
        }
        for (region, defs) in by_region {
            append_to_region(program, region, defs);
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Region addressing
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Region {
    Program,
    /// A top-level IIFE expression statement at this index of `program.body`.
    Iife(usize),
}

fn append_to_region<'a>(program: &mut Program<'a>, region: Region, defs: Vec<Statement<'a>>) {
    match region {
        Region::Program => program.body.extend(defs),
        Region::Iife(i) => {
            if let Some(stmt) = program.body.get_mut(i) {
                if let Some(body) = iife_body_mut(stmt) {
                    body.extend(defs);
                }
            }
        }
    }
}

fn iife_body<'b, 'a>(stmt: &'b Statement<'a>) -> Option<&'b ArenaVec<'a, Statement<'a>>> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let mut expr = &es.expression;
    while let Expression::UnaryExpression(u) = expr {
        expr = &u.argument;
    }
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    // The callee is the immediately-invoked function — directly, or via
    // `(function(){…}).call(this)` / `.apply(this)` (a `this`-setting IIFE).
    let func_callee = match &call.callee {
        Expression::StaticMemberExpression(m)
            if matches!(m.property.name.as_str(), "call" | "apply") =>
        {
            &m.object
        }
        other => other,
    };
    match func_callee {
        Expression::FunctionExpression(f) => f.body.as_ref().map(|b| &b.statements),
        Expression::ArrowFunctionExpression(f) => Some(&f.body.statements),
        _ => None,
    }
}

fn iife_body_mut<'b, 'a>(
    stmt: &'b mut Statement<'a>,
) -> Option<&'b mut ArenaVec<'a, Statement<'a>>> {
    let Statement::ExpressionStatement(es) = stmt else {
        return None;
    };
    let mut expr = &mut es.expression;
    while let Expression::UnaryExpression(u) = expr {
        expr = &mut u.argument;
    }
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    // Unwrap a `(fn).call(this)` / `.apply(this)` member to the function itself.
    let callee = &mut call.callee;
    let func = if let Expression::StaticMemberExpression(m) = callee {
        if !matches!(m.property.name.as_str(), "call" | "apply") {
            return None;
        }
        &mut m.object
    } else {
        callee
    };
    match func {
        Expression::FunctionExpression(f) => f.body.as_mut().map(|b| &mut b.statements),
        Expression::ArrowFunctionExpression(f) => Some(&mut f.body.statements),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Immutable function-array collection
// ---------------------------------------------------------------------------

struct ArrayInfo<'a> {
    region: Region,
    /// Slot clones; `None` for elements that are not plain function expressions.
    slots: Vec<Option<SlotFn<'a>>>,
}

/// A *specializable* slot: a single-`return` function expression with a
/// simple-identifier selector parameter and no `arguments` use.
struct SlotFn<'a> {
    /// A clone of the slot's `Function` node (its first param is the selector).
    func: oxc_allocator::Box<'a, Function<'a>>,
    selector: String,
}

fn collect_immutable_fn_arrays<'a>(
    program: &Program<'a>,
    allocator: &'a Allocator,
) -> HashMap<String, ArrayInfo<'a>> {
    let mut c = ArrayCollector {
        ast: AstBuilder::new(allocator),
        counts: HashMap::new(),
        value_writes: HashMap::new(),
        member_mutated: HashSet::new(),
        found: HashMap::new(),
    };
    for (i, stmt) in program.body.iter().enumerate() {
        c.scan_region_decls(stmt, Region::Program);
        if let Some(body) = iife_body(stmt) {
            for s in body {
                c.scan_region_decls(s, Region::Iife(i));
            }
        }
    }
    c.visit_program(program); // program-wide mutation / uniqueness census

    c.found
        .into_iter()
        .filter(|(name, _)| {
            c.counts.get(name).copied() == Some(1)
                && c.value_writes.get(name).copied() == Some(1)
                && !c.member_mutated.contains(name)
        })
        .collect()
}

struct ArrayCollector<'a> {
    ast: AstBuilder<'a>,
    counts: HashMap<String, usize>,
    value_writes: HashMap<String, usize>,
    member_mutated: HashSet<String>,
    found: HashMap<String, ArrayInfo<'a>>,
}

impl<'a> ArrayCollector<'a> {
    fn scan_region_decls(&mut self, stmt: &Statement<'a>, region: Region) {
        let Statement::VariableDeclaration(vd) = stmt else {
            return;
        };
        for d in &vd.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            let Some(init) = &d.init else { continue };
            if let Some(arr) = extract_fn_array(init) {
                let slots = arr.elements.iter().map(|el| self.make_slot(el)).collect();
                self.found
                    .insert(id.name.to_string(), ArrayInfo { region, slots });
            }
        }
    }

    fn make_slot(&self, el: &ArrayExpressionElement<'a>) -> Option<SlotFn<'a>> {
        let ArrayExpressionElement::FunctionExpression(f) = el else {
            return None;
        };
        let body = f.body.as_ref()?;
        if f.params.rest.is_some() {
            return None;
        }
        let selector = f.params.items.first().and_then(|p| match &p.pattern {
            BindingPattern::BindingIdentifier(id) => Some(id.name.to_string()),
            _ => None,
        });
        // Only *specializable* slots are cloned (and thus kept): a simple-identifier
        // selector, a single-`return` body (which folds cleanly once the selector is
        // bound — a large multi-statement slot would only bloat output), and no
        // `arguments` use (dropping param 0 would shift its indices). Everything else
        // is stored as `None` — never cloned, never specialized.
        //
        // NOTE (issue 02, 2026-06): widening this to multi-statement / `switch`
        // bodies was tried and *reverted* — it regressed sample_10 from 1.67 MB to
        // 3.9–10 MB. The reCAPTCHA handlers do real, non-foldable work (DOM/XHR), so
        // binding the selector only removes the dispatch guard and leaves the large
        // live body, duplicated once per distinct selector. Only the trivial
        // single-`return` slots fold to nothing; for everything else specialization
        // bloats. See issues/02-demultiplex-slot-gate.md.
        let selector = selector?;
        let single_return = body.statements.len() == 1
            && matches!(
                &body.statements[0],
                Statement::ReturnStatement(r) if r.argument.is_some()
            );
        if !single_return || references_arguments(body) {
            return None;
        }
        Some(SlotFn {
            func: f.clone_in(self.ast.allocator),
            selector,
        })
    }
}

impl<'a> Visit<'a> for ArrayCollector<'a> {
    fn visit_binding_identifier(&mut self, it: &BindingIdentifier<'a>) {
        *self.counts.entry(it.name.to_string()).or_default() += 1;
    }

    fn visit_variable_declarator(&mut self, node: &VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if node.init.is_some() {
                *self.value_writes.entry(id.name.to_string()).or_default() += 1;
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }

    fn visit_assignment_expression(&mut self, node: &AssignmentExpression<'a>) {
        match &node.left {
            AssignmentTarget::AssignmentTargetIdentifier(id) => {
                *self.value_writes.entry(id.name.to_string()).or_default() += 1;
            }
            AssignmentTarget::StaticMemberExpression(m) => {
                if let Expression::Identifier(o) = &m.object {
                    self.member_mutated.insert(o.name.to_string());
                }
            }
            AssignmentTarget::ComputedMemberExpression(m) => {
                if let Expression::Identifier(o) = &m.object {
                    self.member_mutated.insert(o.name.to_string());
                }
            }
            _ => {}
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &UpdateExpression<'a>) {
        let base = match &node.argument {
            SimpleAssignmentTarget::ComputedMemberExpression(m) => Some(&m.object),
            SimpleAssignmentTarget::StaticMemberExpression(m) => Some(&m.object),
            _ => None,
        };
        if let Some(Expression::Identifier(o)) = base {
            self.member_mutated.insert(o.name.to_string());
        }
        oxc_ast_visit::walk::walk_update_expression(self, node);
    }

    fn visit_unary_expression(&mut self, node: &UnaryExpression<'a>) {
        if matches!(node.operator, oxc_syntax::operator::UnaryOperator::Delete) {
            let base = match &node.argument {
                Expression::ComputedMemberExpression(m) => Some(&m.object),
                Expression::StaticMemberExpression(m) => Some(&m.object),
                _ => None,
            };
            if let Some(Expression::Identifier(o)) = base {
                self.member_mutated.insert(o.name.to_string());
            }
        }
        oxc_ast_visit::walk::walk_unary_expression(self, node);
    }

    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        const MUTATORS: &[&str] = &[
            "push",
            "pop",
            "shift",
            "unshift",
            "splice",
            "sort",
            "reverse",
            "fill",
            "copyWithin",
        ];
        if let Expression::StaticMemberExpression(m) = &node.callee {
            if MUTATORS.contains(&m.property.name.as_str()) {
                if let Expression::Identifier(o) = &m.object {
                    self.member_mutated.insert(o.name.to_string());
                }
            }
        }
        oxc_ast_visit::walk::walk_call_expression(self, node);
    }
}

/// The array-of-elements initializer behind `init`, either a direct array literal
/// or one returned by a single-`return` IIFE (`(function(){ … return [..]; })()`).
fn extract_fn_array<'b, 'a>(init: &'b Expression<'a>) -> Option<&'b ArrayExpression<'a>> {
    match init {
        Expression::ArrayExpression(a) => Some(a),
        Expression::CallExpression(call) if call.arguments.is_empty() => {
            let body = match &call.callee {
                Expression::FunctionExpression(f) => &f.body.as_ref()?.statements,
                Expression::ArrowFunctionExpression(f) => &f.body.statements,
                _ => return None,
            };
            for stmt in body.iter().rev() {
                if let Statement::ReturnStatement(r) = stmt {
                    if let Some(Expression::ArrayExpression(a)) = &r.argument {
                        return Some(a);
                    }
                    return None;
                }
            }
            None
        }
        _ => None,
    }
}

fn references_arguments(body: &FunctionBody<'_>) -> bool {
    struct F(bool);
    impl<'a> Visit<'a> for F {
        fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
            if id.name == "arguments" {
                self.0 = true;
            }
        }
        fn visit_function(&mut self, _f: &Function<'a>, _flags: oxc_syntax::scope::ScopeFlags) {}
        fn visit_arrow_function_expression(&mut self, _f: &ArrowFunctionExpression<'a>) {}
    }
    let mut f = F(false);
    f.visit_function_body(body);
    f.0
}

fn collect_binding_names(program: &Program<'_>) -> HashSet<String> {
    struct C(HashSet<String>);
    impl<'a> Visit<'a> for C {
        fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
            self.0.insert(id.name.to_string());
        }
    }
    let mut c = C(HashSet::new());
    c.visit_program(program);
    c.0
}

// ---------------------------------------------------------------------------
// Rewriting
// ---------------------------------------------------------------------------

struct Rewriter<'a, 'r> {
    arrays: &'r HashMap<String, ArrayInfo<'a>>,
    existing: HashSet<String>,
    emitted: HashMap<(String, usize, i64), String>,
    new_defs: Vec<(Region, Statement<'a>)>,
    count: usize,
    truncated: bool,
    changed: bool,
}

fn spec_name(array: &str, k: usize, c: i64) -> String {
    format!("__demux_{array}_{k}_{c}")
}

/// A non-negative numeric literal value as an exact integer.
fn numeric_literal_int(expr: &Expression<'_>) -> Option<i64> {
    let Expression::NumericLiteral(n) = expr else {
        return None;
    };
    let v = n.value;
    (v.fract() == 0.0 && (0.0..9e15).contains(&v)).then_some(v as i64)
}

impl<'a, 'r> Rewriter<'a, 'r> {
    /// Ensure the specialized function for `(array, k, C)` exists, returning its
    /// name; `None` if not specializable or the cap is hit.
    fn ensure_spec(
        &mut self,
        array: &str,
        k: usize,
        c: i64,
        ast: AstBuilder<'a>,
    ) -> Option<String> {
        let key = (array.to_string(), k, c);
        if let Some(n) = self.emitted.get(&key) {
            return Some(n.clone());
        }
        let name = spec_name(array, k, c);
        if self.existing.contains(&name) {
            self.emitted.insert(key, name.clone());
            return Some(name);
        }
        if self.count >= MAX_SPECIALIZATIONS {
            self.truncated = true;
            return None;
        }

        let info = self.arrays.get(array)?;
        let slot = info.slots.get(k)?.as_ref()?;
        let selector = slot.selector.clone();

        // Clone the slot function and mutate the clone into a named declaration
        // with parameter 0 dropped and `var <selector> = C;` prepended.
        let mut func = slot.func.clone_in(ast.allocator);
        func.r#type = FunctionType::FunctionDeclaration;
        func.id = Some(ast.binding_identifier(SPAN, ast.str(&name)));

        // Drop parameter 0.
        let old_items = std::mem::replace(&mut func.params.items, ast.vec());
        let mut new_items = ast.vec();
        for (i, p) in old_items.into_iter().enumerate() {
            if i != 0 {
                new_items.push(p);
            }
        }
        func.params.items = new_items;

        // Prepend `var <selector> = C;` to the body.
        if let Some(body) = &mut func.body {
            let binding = self.selector_binding(&selector, c, ast);
            let old_stmts = std::mem::replace(&mut body.statements, ast.vec());
            body.statements.push(binding);
            for s in old_stmts {
                body.statements.push(s);
            }
        } else {
            return None;
        }

        let region = info.region;
        self.new_defs
            .push((region, Statement::FunctionDeclaration(func)));
        self.existing.insert(name.clone());
        self.emitted.insert(key, name.clone());
        self.count += 1;
        Some(name)
    }

    fn selector_binding(&self, selector: &str, c: i64, ast: AstBuilder<'a>) -> Statement<'a> {
        use oxc_syntax::number::NumberBase;
        let value = ast.expression_numeric_literal(SPAN, c as f64, None, NumberBase::Decimal);
        let id = ast.binding_pattern_binding_identifier(SPAN, ast.str(selector));
        let decl = ast.variable_declarator(
            SPAN,
            VariableDeclarationKind::Var,
            id,
            NONE,
            Some(value),
            false,
        );
        let mut decls = ast.vec();
        decls.push(decl);
        Statement::VariableDeclaration(ast.alloc_variable_declaration(
            SPAN,
            VariableDeclarationKind::Var,
            decls,
            false,
        ))
    }
}

impl<'a, 'r> Traverse<'a, ()> for Rewriter<'a, 'r> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let ast = ctx.ast;

        // Inspect the call shape immutably first.
        let Expression::CallExpression(call) = &*expr else {
            return;
        };
        let Some((array, k, sel_idx, dot_call)) = classify_call(call, self.arrays) else {
            return;
        };
        let Some(c) = call
            .arguments
            .get(sel_idx)
            .and_then(|a| a.as_expression())
            .and_then(numeric_literal_int)
        else {
            return;
        };

        let Some(name) = self.ensure_spec(&array, k, c, ast) else {
            return;
        };

        // Re-borrow mutably and move out the argument vector.
        let Expression::CallExpression(call) = expr else {
            return;
        };
        let old_args = std::mem::replace(&mut call.arguments, ast.vec());

        // Receiver and trailing args.
        let mut moved: Vec<Option<Argument<'a>>> = old_args.into_iter().map(Some).collect();
        let receiver: Expression<'a> = if dot_call {
            // `.call(recv, C, …)`: the moved receiver argument. A spread/missing
            // receiver is not specializable.
            match moved.get_mut(0).and_then(Option::take) {
                Some(Argument::SpreadElement(_)) | None => return,
                Some(arg) => arg.into_expression(),
            }
        } else {
            ast.expression_identifier(SPAN, ast.str(&array))
        };

        let mut new_args = ast.vec();
        new_args.push(Argument::from(receiver));
        for slot in moved.iter_mut().skip(sel_idx + 1) {
            if let Some(arg) = slot.take() {
                new_args.push(arg);
            }
        }

        // `name.call(receiver, trailing…)`
        let callee_ident = ast.expression_identifier(SPAN, ast.str(&name));
        let member = ast.alloc_static_member_expression(
            SPAN,
            callee_ident,
            ast.identifier_name(SPAN, ast.str("call")),
            false,
        );
        let new_call = ast.expression_call(
            SPAN,
            Expression::StaticMemberExpression(member),
            NONE,
            new_args,
            false,
        );
        *expr = new_call;
        self.changed = true;
    }
}

/// Classify a call as a specializable `A[k](…)` or `A[k].call(recv, …)`. Returns
/// `(array name, k, selector arg index, is_dot_call)`.
fn classify_call(
    call: &CallExpression<'_>,
    arrays: &HashMap<String, ArrayInfo<'_>>,
) -> Option<(String, usize, usize, bool)> {
    let (member, dot_call) = match &call.callee {
        Expression::ComputedMemberExpression(m) => (&**m, false),
        Expression::StaticMemberExpression(sm) if sm.property.name == "call" => {
            let Expression::ComputedMemberExpression(m) = &sm.object else {
                return None;
            };
            (&**m, true)
        }
        _ => return None,
    };
    let Expression::Identifier(array_id) = &member.object else {
        return None;
    };
    let array = array_id.name.to_string();
    let info = arrays.get(&array)?;
    let k = numeric_literal_int(&member.expression)?;
    if k < 0 {
        return None;
    }
    let k = k as usize;
    info.slots.get(k)?.as_ref()?;
    let sel_idx = if dot_call { 1 } else { 0 };
    if call.arguments.len() <= sel_idx {
        return None;
    }
    // The selector must be a constant numeric literal.
    numeric_literal_int(call.arguments.get(sel_idx)?.as_expression()?)?;
    Some((array, k, sel_idx, dot_call))
}
