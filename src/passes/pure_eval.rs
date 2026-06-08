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
        let mut e = Evaluator {
            sandbox,
            changed: false,
        };
        traverse_mut(&mut e, allocator, program, scoping, ());
        self.changed = e.changed;
        self.changed
    }
}

struct Evaluator {
    sandbox: Sandbox,
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
///
/// Bare global calls (`parseInt(…)`, `Math.max(…)`) are intentionally excluded —
/// const-fold/oxc already evaluate those, so adding them here would only duplicate
/// work and run `freevars` over every plain `foo()` call site.
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

impl<'a> Traverse<'a, ()> for Evaluator {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        if !is_foldable_call(expr) {
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
        // Closed + effect-free gate (empty pureset: only side-effect-free globals
        // are tolerated as free names; no `this`, no writes to outer state).
        if !freevars_of_expr(&*expr).all_allowed(&Default::default(), None, &Default::default()) {
            return;
        }

        let mut cg = oxc_codegen::Codegen::new();
        cg.print_expression(&*expr);
        let src = format!("({});", cg.into_source_text());

        // Evaluate twice; require agreement (rejects non-determinism) and a
        // primitive result (the only thing with a literal form).
        let val = match self.sandbox.eval_to_value(&src) {
            Some(a) if self.sandbox.eval_to_value(&src).as_ref() == Some(&a) => a,
            _ => return,
        };

        let span = expr.span();
        *expr = match val {
            EvalValue::Str(s) => {
                let v = ctx.ast.str(&s);
                ctx.ast.expression_string_literal(span, v, None)
            }
            EvalValue::Num(n) if n.is_finite() => num_literal(n, span, ctx),
            EvalValue::Bool(b) => ctx.ast.expression_boolean_literal(span, b),
            // NaN / Infinity have no numeric literal form — leave the IIFE.
            EvalValue::Num(_) => return,
            // Array results are handled only by decoder-lift's size-capped splice;
            // pure-eval declines them (a pure IIFE returning an array is rare and
            // not worth materializing here).
            EvalValue::Arr(_) => return,
        };
        self.changed = true;
    }
}

/// Build a numeric literal for `n`, wrapping negatives as `-(lit)` (the grammar
/// has no negative numeric literal). Mirrors the helper in `decoder_lift`.
fn num_literal<'a>(n: f64, span: oxc_span::Span, ctx: &mut TraverseCtx<'a, ()>) -> Expression<'a> {
    use oxc_syntax::number::NumberBase;
    if n.is_sign_negative() {
        let lit = ctx
            .ast
            .expression_numeric_literal(span, -n, None, NumberBase::Decimal);
        ctx.ast.expression_unary(
            span,
            oxc_syntax::operator::UnaryOperator::UnaryNegation,
            lit,
        )
    } else {
        ctx.ast
            .expression_numeric_literal(span, n, None, NumberBase::Decimal)
    }
}
