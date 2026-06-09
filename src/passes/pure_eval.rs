//! Whole-expression constant evaluation of self-contained pure calls.
//!
//! Generalizes decoder-lift's "run it in the sandbox" idea from decoder *calls* to
//! any closed, side-effect-free call that produces a primitive:
//!   * an immediately-invoked function expression — `var VM = (function(){ … })();`
//!     (obfuscated bytecode VMs and "compute a constant the hard way" blobs); and
//!   * a **pure method call** on a constant receiver — `['a','b'].join('')`,
//!     `"x".toUpperCase()` — the pure built-ins const-fold cannot reach. Single-use
//!     inlining feeds these literal receivers (`var s = "abc"; s.toUpperCase()` →
//!     `"abc".toUpperCase()`), which then collapse to their value here.
//!
//! Soundness — we only evaluate an IIFE whose evaluation has no observable effect
//! beyond producing its value, and whose value is a primitive we can splice as a
//! literal. Concretely the call expression must:
//!   * be an IIFE — callee is a function/arrow expression (a closed scope), so the
//!     only thing it can leak is its return value;
//!   * reference no free name other than a known side-effect-free global, use no
//!     `this`, and write to no free/outer variable (reuses decoder-lift's
//!     `FreeVars::all_allowed` — the same gate that guards decoder lifting);
//!   * evaluate to the *same* primitive twice in a fresh sandbox (the determinism
//!     guard: anything reaching `Date`/`Math.random`/host state disagrees or
//!     throws, and is left untouched).
//!
//! A non-primitive result (object/array/function/undefined) has no literal form,
//! so it is left in place. Because the IIFE is closed and effect-free, replacing
//! it with its value is semantics-preserving.

use oxc_allocator::Allocator;
use oxc_ast::ast::{Expression, Program};
use oxc_span::GetSpan;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::passes::decoder_lift::freevars_of_expr;
use crate::sandbox::{EvalValue, Sandbox};
use crate::util::build_scoping;

#[derive(Default)]
pub struct PureEval {
    changed: bool,
}

impl Pass for PureEval {
    fn name(&self) -> &'static str {
        "pure-eval"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let Some(sandbox) = Sandbox::new() else {
            return false;
        };
        let scoping = build_scoping(program);
        // Provably-immutable, closed (literal-only) aggregate bindings, as source we
        // can re-materialize in the sandbox so a `var a = […]; a.join('-')` receiver
        // resolves even though the array stays bound to a variable.
        let aggregates = immutable_aggregates(program, &scoping);
        let agg_names: std::collections::HashSet<String> = aggregates.keys().cloned().collect();
        // Pure coercion globals safe to fold as bare calls: those not bound anywhere
        // in the program (so a `Number(…)` cannot be a local shadow of the global).
        let bound: std::collections::HashSet<&str> = scoping.symbol_names().collect();
        let safe_coercion: std::collections::HashSet<&'static str> = PURE_COERCION_GLOBALS
            .iter()
            .copied()
            .filter(|n| !bound.contains(n))
            .collect();
        let mut e = Evaluator {
            sandbox,
            changed: false,
            aggregates,
            agg_names,
            safe_coercion,
            json_is_global: !bound.contains("JSON"),
        };
        traverse_mut(&mut e, allocator, program, scoping, ());
        self.changed = e.changed;
        self.changed
    }
}

struct Evaluator {
    sandbox: Sandbox,
    /// name → source of its immutable, literal-only initializer (`['a','b']`).
    aggregates: std::collections::HashMap<String, String>,
    /// Just the keys of `aggregates`, for the free-name gate.
    agg_names: std::collections::HashSet<String>,
    /// Pure coercion globals (`Boolean`/`Number`/…) provably not shadowed locally.
    safe_coercion: std::collections::HashSet<&'static str>,
    /// `true` when `JSON` is not shadowed — so `JSON.parse(<literal>)` can fold to an
    /// object/array literal (its result is by spec a plain data structure).
    json_is_global: bool,
    changed: bool,
}

/// Is `expr` a call we should try to constant-evaluate in the sandbox? Two shapes
/// qualify; the closed-and-pure gate in `enter_expression` then proves each one is
/// safe to fold (any free name outside the allowed pure globals rejects it):
///   * an **IIFE** — `(function(){…})(…)` / `(()=>…)(…)` (a closed function scope);
///   * a **method call** — `recv.m(…)` / `recv[k](…)`, which covers the pure
///     built-ins const-fold cannot reach (`['a','b'].join('')`, `"x".toUpperCase()`,
///     `[…].concat(…)`). A user method like `obj.m()` has `obj` as a free name and
///     is rejected by the gate; a literal/`Math`/`String` receiver passes.
///   * a **pure coercion global** — `Boolean(…)`, `Number(…)`, `String(…)` and
///     friends, which oxc's evaluator does not fold (unlike `parseInt`/`Math.max`).
///     Restricted to a fixed whitelist so the `freevars` walk is not run over every
///     plain `foo()` call site; the closed gate then proves the argument is pure.
fn is_foldable_call(expr: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    matches!(
        &call.callee,
        Expression::FunctionExpression(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::StaticMemberExpression(_)
            | Expression::ComputedMemberExpression(_)
    )
}

/// Is `expr` a `JSON.parse(…)` call? (Used to gate object-literal materialization
/// to the one form whose result is a guaranteed-plain data object.)
fn is_json_parse_call(expr: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    let Expression::StaticMemberExpression(m) = &call.callee else {
        return false;
    };
    m.property.name == "parse"
        && matches!(&m.object, Expression::Identifier(id) if id.name == "JSON")
}

/// Pure global functions that return a primitive and that oxc's constant evaluator
/// does not fold. Limited to value coercions whose result is fully determined by
/// (pure) arguments — the closed gate still proves the arguments are side-effect
/// free, and the double-eval guard rejects any nondeterminism.
const PURE_COERCION_GLOBALS: &[&str] = &[
    "Boolean", "Number", "String", "parseInt", "parseFloat", "isNaN", "isFinite",
];

impl<'a> Traverse<'a, ()> for Evaluator {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // A foldable call, a member access whose root is an immutable aggregate
        // (`a.join(…)`, `o.a.b`), or a bare call to a non-shadowed pure coercion
        // global (`Number('5')`).
        if !is_foldable_call(expr)
            && !self.is_aggregate_member(expr)
            && !self.is_safe_coercion_call(expr)
        {
            return;
        }
        // Only fold a call whose *result is consumed* (an initializer, argument, or
        // subexpression). A bare-statement IIFE `(function(){…})();` discards its
        // value, so collapsing it to a literal would just produce dead code for DCE
        // — but a *pure* bare IIFE is exactly the scope wrapper that
        // dead-object-prop / SSA fold from the inside, a strictly more informative
        // result. Leave those to them; our value-add is the consumed case.
        if ctx.parent().is_expression_statement() {
            return;
        }
        // Closed + effect-free gate: every free name must be a side-effect-free
        // global OR an immutable aggregate we can re-materialize (no `this`, no
        // writes to outer state).
        if !freevars_of_expr(&*expr).all_allowed(&self.agg_names, None, &Default::default()) {
            return;
        }

        let mut cg = oxc_codegen::Codegen::new();
        cg.print_expression(&*expr);
        let printed = cg.into_source_text();
        // Materialize any immutable aggregate the expression reads, so its receiver
        // resolves in the fresh sandbox. Soundness rests on the aggregate being
        // provably never mutated/aliased (see `immutable_aggregates`).
        let mut prelude = String::new();
        for name in collect_ref_names(&*expr) {
            if let Some(init) = self.aggregates.get(&name) {
                prelude.push_str(&format!("var {name} = {init};\n"));
            }
        }
        let src = format!("{prelude}({printed});");

        // Evaluate twice; require agreement (rejects non-determinism). The result
        // splices to its literal form — a primitive, or a (sandbox size-capped)
        // array literal, which materializes a pure IIFE's table so DCE can drop the
        // now-dead scaffolding once it is the only reader.
        let val = match self.sandbox.eval_to_value(&src) {
            Some(a) if self.sandbox.eval_to_value(&src).as_ref() == Some(&a) => a,
            _ => return,
        };

        // An object literal is only materialized for `JSON.parse(<literal>)`, whose
        // result is by spec a plain data object (no accessors / exotic prototype) —
        // the one case `value_to_eval`'s object read is guaranteed faithful.
        if matches!(val, EvalValue::Obj(_)) && !(self.json_is_global && is_json_parse_call(expr)) {
            return;
        }

        let span = expr.span();
        let Some(new_expr) = crate::passes::decoder_lift::eval_value_to_expr(&val, span, ctx) else {
            return; // NaN/Infinity or a non-materializable array
        };
        *expr = new_expr;
        self.changed = true;
    }
}

impl Evaluator {
    /// A bare call `f(…)` where `f` is a pure coercion global proven not shadowed.
    fn is_safe_coercion_call(&self, expr: &Expression<'_>) -> bool {
        let Expression::CallExpression(call) = expr else {
            return false;
        };
        matches!(&call.callee, Expression::Identifier(id)
            if self.safe_coercion.contains(id.name.as_str()))
    }

    /// A static/computed member access whose root receiver is an immutable
    /// aggregate (`o.a.b`, `a[0].x`) — worth resolving to its primitive value.
    fn is_aggregate_member(&self, expr: &Expression<'_>) -> bool {
        let mut cur = expr;
        loop {
            match cur {
                Expression::StaticMemberExpression(m) => cur = &m.object,
                Expression::ComputedMemberExpression(m) => cur = &m.object,
                Expression::Identifier(id) => return self.agg_names.contains(id.name.as_str()),
                _ => return false,
            }
        }
    }
}

/// Collect the names of every identifier *reference* in `expr` (free or not — the
/// caller only intersects with the immutable-aggregate set, whose names are unique
/// program-wide, so a stray match is impossible).
fn collect_ref_names(expr: &Expression<'_>) -> std::collections::HashSet<String> {
    use oxc_ast_visit::Visit;
    struct R(std::collections::HashSet<String>);
    impl<'a> Visit<'a> for R {
        fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
            self.0.insert(it.name.to_string());
        }
    }
    let mut r = R(std::collections::HashSet::new());
    r.visit_expression(expr);
    r.0
}

/// Provably-immutable, closed (literal-only) aggregate bindings: `name → init
/// source`. A binding qualifies when it is a `var name = <array|object literal>`
/// whose name is a unique symbol, is **not** in `mutable_aggregate_tables` (so it is
/// never reassigned, member-written, `.push`-ed, or read bare into a mutable alias),
/// and whose initializer is closed — its free names are all side-effect-free
/// globals — so re-materializing it standalone in the sandbox is faithful.
fn immutable_aggregates(
    program: &Program<'_>,
    scoping: &oxc_semantic::Scoping,
) -> std::collections::HashMap<String, String> {
    use oxc_ast::ast::{BindingPattern, Expression as E, Statement};
    let mut mutable = crate::ir::mutable_aggregate_tables(program, scoping);
    // `mutable_aggregate_tables` does not see a `for (var s of …)` / `for (s in …)`
    // head, which rebinds `s` every iteration — exactly the SSA `exclude_for_head`
    // hazard. Exclude any name bound or assigned by a for-in/for-of head so a
    // rebound aggregate is never baked at its stale initial value.
    mutable.extend(for_head_names(program));
    let mut out = std::collections::HashMap::new();
    // Count symbols per name to reject shadowed names (a name-keyed materialization
    // must be unambiguous).
    let mut name_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for n in scoping.symbol_names() {
        *name_counts.entry(n.to_string()).or_default() += 1;
    }
    // Only top-level declarations (the program body); a function-scoped aggregate is
    // not materialized here (its symbol scope is irrelevant to a name-based bake).
    for stmt in &program.body {
        let Statement::VariableDeclaration(vd) = stmt else {
            continue;
        };
        for d in &vd.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            let name = id.name.as_str();
            let Some(init) = &d.init else { continue };
            if !matches!(init, E::ArrayExpression(_) | E::ObjectExpression(_)) {
                continue;
            }
            if mutable.contains(name) || name_counts.get(name).copied().unwrap_or(0) != 1 {
                continue;
            }
            // Closed initializer: only literals / side-effect-free globals inside.
            if !freevars_of_expr(init).all_allowed(&Default::default(), None, &Default::default()) {
                continue;
            }
            let mut cg = oxc_codegen::Codegen::new();
            cg.print_expression(init);
            out.insert(name.to_string(), cg.into_source_text());
        }
    }
    out
}

/// Names bound or assigned by any `for-in` / `for-of` head (`for (var s of …)`,
/// `for (s in …)`) — each is rebound per iteration, so its initial value is stale.
fn for_head_names(program: &Program<'_>) -> std::collections::HashSet<String> {
    use oxc_ast::ast::{BindingPattern, ForStatementLeft};
    use oxc_ast_visit::Visit;
    struct F(std::collections::HashSet<String>);
    impl F {
        fn note_left(&mut self, left: &ForStatementLeft<'_>) {
            match left {
                ForStatementLeft::VariableDeclaration(vd) => {
                    for d in &vd.declarations {
                        if let BindingPattern::BindingIdentifier(id) = &d.id {
                            self.0.insert(id.name.to_string());
                        }
                    }
                }
                ForStatementLeft::AssignmentTargetIdentifier(id) => {
                    self.0.insert(id.name.to_string());
                }
                _ => {}
            }
        }
    }
    impl<'a> Visit<'a> for F {
        fn visit_for_of_statement(&mut self, n: &oxc_ast::ast::ForOfStatement<'a>) {
            self.note_left(&n.left);
            oxc_ast_visit::walk::walk_for_of_statement(self, n);
        }
        fn visit_for_in_statement(&mut self, n: &oxc_ast::ast::ForInStatement<'a>) {
            self.note_left(&n.left);
            oxc_ast_visit::walk::walk_for_in_statement(self, n);
        }
    }
    let mut f = F(std::collections::HashSet::new());
    f.visit_program(program);
    f.0
}
