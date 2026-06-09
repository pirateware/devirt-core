//! Constant folding via oxc's `ConstantEvaluation`.
//!
//! We provide a deliberately *conservative* evaluation context: property reads
//! and unknown globals are treated as side-effectful, so only genuinely pure
//! expressions fold. That defeats the common opaque predicates and arithmetic
//! noise (`!![]`, `!0`, `(0x1, 2)` comma chains, literal arithmetic) without
//! risking unsound rewrites. We fold to number/boolean/null only; string and
//! bigint folding are left for later (they need arena string allocation and are
//! mostly subsumed by decoder lifting).
//!
//! Also folds `if (<constant>)` to its taken branch — but preserves hoisted
//! `var`/function declarations from the dead branch is NOT handled yet, so we
//! only fold when the discarded branch is empty-ish (a single non-declaration).

use oxc_allocator::{Allocator, CloneIn, TakeIn};
use oxc_ast::ast::IdentifierReference;
use oxc_ast::ast::{Expression, Statement};
use oxc_ast::AstBuilder;
use oxc_ecmascript::constant_evaluation::{
    ConstantEvaluation, ConstantEvaluationCtx, ConstantValue,
};
use oxc_ecmascript::side_effects::{
    MayHaveSideEffects, MayHaveSideEffectsContext, PropertyReadSideEffects,
};
use oxc_ecmascript::GlobalContext;
use oxc_semantic::Scoping;
use oxc_span::GetSpan;
use oxc_syntax::number::NumberBase;
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::{build_scoping, is_pure_simple};

#[derive(Default)]
pub struct ConstantFold {
    changed: bool,
    /// Whether it is safe to emit the bare `undefined` identifier (i.e. nothing
    /// in the program binds a local named `undefined` that could shadow the
    /// read-only global). Recomputed each run.
    allow_undefined: bool,
}

impl Pass for ConstantFold {
    fn name(&self) -> &'static str {
        "const-fold"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        self.changed = false;
        let scoping = build_scoping(program);
        // `undefined` is a read-only global, but sloppy-mode code *may* bind a
        // local of that name. Only rewrite to it when no such binding exists.
        self.allow_undefined = !scoping.symbol_names().any(|n| n == "undefined");
        traverse_mut(self, allocator, program, scoping, ());
        self.changed
    }
}

use oxc_ast::ast::Program;

impl<'a> Traverse<'a, ()> for ConstantFold {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // Don't re-fold something already in canonical form. `undefined` folds
        // to itself, so without this guard the fixpoint would never converge.
        if is_literal(expr) || is_undefined_ident(expr) {
            return;
        }

        // Member access on a literal collection that is present *right here* in
        // the AST: `[a, b, c][1]` → `b`, `({x: 1}).x` → `1`, `"abc"[1]` → `"b"`,
        // `[…].length`. Unconditionally sound — there is no binding and thus no
        // ordering concern; we only fold when the dropped siblings are pure (so
        // their construction had no side effect to lose). oxc's evaluator refuses
        // these because it models every property read as side-effectful.
        if let Some(new_expr) = fold_literal_member(expr, ctx.ast) {
            *expr = new_expr;
            self.changed = true;
            return;
        }

        // An untagged template literal whose interpolations are all constant folds
        // to a single string: `` `val: ${10}` `` → `"val: 10"`. Sound — the parts
        // are pure literals, concatenated under JS string coercion.
        if let Expression::TemplateLiteral(tl) = expr {
            if let Some(s) = fold_template(tl) {
                let span = tl.span;
                let atom = ctx.ast.str(&s);
                *expr = ctx.ast.expression_string_literal(span, atom, None);
                self.changed = true;
                return;
            }
        }

        // A ternary with a constant test collapses to its taken branch. Mirrors
        // the `if (<constant>)` statement fold; the conservative eval context
        // only resolves a pure test, so dropping the untaken branch is sound.
        if let Expression::ConditionalExpression(c) = expr {
            let cond = {
                let ectx = EvalCtx {
                    ast: ctx.ast,
                    scoping: ctx.scoping(),
                };
                // Collapsing the ternary discards the test, so it must be pure
                // (a side-effecting test like `(seen = 1, 1) ? a : b` still has
                // to run). The untaken branch never executes, so dropping it is
                // always fine.
                if c.test.may_have_side_effects(&ectx) {
                    None
                } else {
                    c.test.evaluate_value_to_boolean(&ectx)
                }
            };
            if let Some(cond) = cond {
                let taken = if cond {
                    c.consequent.take_in(ctx.ast)
                } else {
                    c.alternate.take_in(ctx.ast)
                };
                *expr = taken;
                self.changed = true;
                return;
            }
        }
        let folded = {
            let ectx = EvalCtx {
                ast: ctx.ast,
                scoping: ctx.scoping(),
            };
            // Replacing the whole expression with its value drops the original,
            // so it must be side-effect free. oxc's `evaluate_value` reports the
            // *value* of e.g. `(seen = 1, 'x')` ("x") but not that discarding the
            // assignment is unsound — this guard is what makes the fold safe.
            if expr.may_have_side_effects(&ectx) {
                None
            } else {
                expr.evaluate_value(&ectx)
            }
        };
        if let Some(value) = folded {
            if let Some(new_expr) = const_to_expr(value, ctx.ast, expr, self.allow_undefined) {
                *expr = new_expr;
                self.changed = true;
            }
        }
    }

    fn enter_statement(&mut self, stmt: &mut Statement<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let Statement::IfStatement(ifs) = stmt else {
            return;
        };
        let cond = {
            let ectx = EvalCtx {
                ast: ctx.ast,
                scoping: ctx.scoping(),
            };
            // As with the ternary: the test always runs, so we may only drop it
            // when it is pure.
            if ifs.test.may_have_side_effects(&ectx) {
                None
            } else {
                ifs.test.evaluate_value_to_boolean(&ectx)
            }
        };
        let Some(cond) = cond else {
            return;
        };
        // Replace with the taken branch; the discarded branch is dropped.
        let taken: Statement<'a> = if cond {
            ifs.consequent.take_in(ctx.ast)
        } else if let Some(alt) = &mut ifs.alternate {
            alt.take_in(ctx.ast)
        } else {
            ctx.ast.statement_empty(ifs.span)
        };
        *stmt = taken;
        self.changed = true;
    }
}

/// Concatenate an untagged template literal whose every interpolation is a
/// primitive literal into the string it produces. Returns `None` (decline) on any
/// non-literal interpolation, an un-cooked quasi (an invalid escape), or a numeric
/// value outside the range where Rust and JS `ToString` provably agree.
fn fold_template(tl: &oxc_ast::ast::TemplateLiteral<'_>) -> Option<String> {
    let mut out = String::new();
    for (i, q) in tl.quasis.iter().enumerate() {
        out.push_str(q.value.cooked.as_ref()?.as_str());
        if let Some(e) = tl.expressions.get(i) {
            out.push_str(&stringify_literal(e)?);
        }
    }
    Some(out)
}

/// JS `ToString` of a primitive-literal expression, for the cases where it is
/// unambiguous. Numbers are limited to integers in the safe range so the formatted
/// digits match JS exactly (large/fractional/exponential numbers decline).
fn stringify_literal(e: &Expression<'_>) -> Option<String> {
    match e {
        Expression::StringLiteral(s) => Some(s.value.to_string()),
        Expression::BooleanLiteral(b) => Some(b.value.to_string()),
        Expression::NullLiteral(_) => Some("null".to_string()),
        Expression::NumericLiteral(n)
            if n.value.fract() == 0.0 && n.value.abs() < 9e15 =>
        {
            Some(format!("{}", n.value as i64))
        }
        _ => None,
    }
}

fn is_literal(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::NumericLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::StringLiteral(_)
    )
}

fn is_undefined_ident(expr: &Expression<'_>) -> bool {
    matches!(expr, Expression::Identifier(id) if id.name == "undefined")
}

/// Convert a folded constant back to a literal node. Returns `None` for cases
/// we deliberately skip (strings, bigint, undefined, non-finite/negative
/// numbers — the latter to avoid fold/rebuild oscillation with unary minus).
fn const_to_expr<'a>(
    value: ConstantValue<'a>,
    ast: AstBuilder<'a>,
    original: &Expression<'a>,
    allow_undefined: bool,
) -> Option<Expression<'a>> {
    let span = original.span();
    match value {
        ConstantValue::Number(n) if n.is_finite() && n >= 0.0 => {
            Some(ast.expression_numeric_literal(span, n, None, NumberBase::Decimal))
        }
        ConstantValue::Boolean(b) => Some(ast.expression_boolean_literal(span, b)),
        ConstantValue::Null => Some(ast.expression_null_literal(span)),
        // String concatenation chains (`"a" + "b"`, `"x" + 5`) collapse to one
        // literal. Common after decoder-lift splices string fragments that the
        // obfuscator had `+`-joined. evaluate_value already enforced purity.
        ConstantValue::String(s) => {
            let atom = ast.str(&s);
            Some(ast.expression_string_literal(span, atom, None))
        }
        // `void 0`, `void "x"`, etc. fold to the readable `undefined`. Only the
        // original expression's purity matters (evaluate_value already required
        // it), and we skip emission entirely if `undefined` could be shadowed.
        ConstantValue::Undefined if allow_undefined => {
            Some(ast.expression_identifier(span, "undefined"))
        }
        _ => None,
    }
}

/// A resolved member key: a non-negative integer index, or a property name.
enum Key<'k> {
    Index(usize),
    Name(&'k str),
}

/// Resolve the key of a member expression to a [`Key`], if it is static. A
/// computed numeric literal that is a non-negative integer becomes `Index`; a
/// computed string literal or a static `.name` becomes `Name`.
fn member_key<'a, 'e>(expr: &'e Expression<'a>) -> Option<(&'e Expression<'a>, Key<'e>)> {
    match expr {
        Expression::StaticMemberExpression(m) => {
            Some((&m.object, Key::Name(m.property.name.as_str())))
        }
        Expression::ComputedMemberExpression(m) => {
            let key = match &m.expression {
                Expression::NumericLiteral(n) => Key::Index(int_index(n.value)?),
                Expression::StringLiteral(s) => Key::Name(s.value.as_str()),
                _ => return None,
            };
            Some((&m.object, key))
        }
        _ => None,
    }
}

/// A finite non-negative integer as a `usize`, else `None`.
fn int_index(n: f64) -> Option<usize> {
    if n.is_finite() && n >= 0.0 && n.fract() == 0.0 && n <= usize::MAX as f64 {
        Some(n as usize)
    } else {
        None
    }
}

/// Fold a member access whose object is a literal array, object, or string.
fn fold_literal_member<'a>(expr: &Expression<'a>, ast: AstBuilder<'a>) -> Option<Expression<'a>> {
    let span = expr.span();
    let (object, key) = member_key(expr)?;
    match object {
        Expression::ArrayExpression(arr) => fold_array_member(arr, &key, span, ast),
        Expression::ObjectExpression(obj) => fold_object_member(obj, &key, ast),
        Expression::StringLiteral(s) => fold_string_member(s, &key, span, ast),
        _ => None,
    }
}

/// Whether `expr` owns or contains a scope (function/arrow/class). Cloning such a
/// node resets its scope_id cell to `None`, which the traverse walker later
/// unwraps and panics on — so a selected element containing one must not be
/// spliced. (Same hazard ProxyInline guards against.)
fn contains_scoped_node(expr: &Expression<'_>) -> bool {
    use oxc_ast_visit::Visit;
    struct Finder {
        found: bool,
    }
    impl<'a> Visit<'a> for Finder {
        fn visit_function(
            &mut self,
            _f: &oxc_ast::ast::Function<'a>,
            _flags: oxc_syntax::scope::ScopeFlags,
        ) {
            self.found = true;
        }
        fn visit_arrow_function_expression(
            &mut self,
            _f: &oxc_ast::ast::ArrowFunctionExpression<'a>,
        ) {
            self.found = true;
        }
        fn visit_class(&mut self, _c: &oxc_ast::ast::Class<'a>) {
            self.found = true;
        }
    }
    let mut f = Finder { found: false };
    f.visit_expression(expr);
    f.found
}

fn fold_array_member<'a>(
    arr: &oxc_ast::ast::ArrayExpression<'a>,
    key: &Key<'_>,
    span: oxc_span::Span,
    ast: AstBuilder<'a>,
) -> Option<Expression<'a>> {
    use oxc_ast::ast::ArrayExpressionElement as El;
    // A spread makes element positions and length unknowable: never fold.
    if arr
        .elements
        .iter()
        .any(|e| matches!(e, El::SpreadElement(_)))
    {
        return None;
    }
    // All non-selected elements must be pure (we are about to drop them).
    let all_pure = |except: Option<usize>| {
        arr.elements.iter().enumerate().all(|(i, e)| match e {
            El::Elision(_) => true,
            El::SpreadElement(_) => false,
            other => Some(i) == except || other.as_expression().is_some_and(is_pure_simple),
        })
    };
    match key {
        Key::Name("length") => {
            if !all_pure(None) {
                return None;
            }
            Some(ast.expression_numeric_literal(
                span,
                arr.elements.len() as f64,
                None,
                NumberBase::Decimal,
            ))
        }
        Key::Index(i) => {
            // A hole (`[1, , 3][1]`) or out-of-range index yields `undefined`;
            // leave those alone. Otherwise select the element, keeping it as-is,
            // and require every *other* element pure so dropping them loses no
            // side effect.
            let selected = arr.elements.get(*i)?.as_expression()?;
            if contains_scoped_node(selected) || !all_pure(Some(*i)) {
                return None;
            }
            Some(selected.clone_in(ast.allocator))
        }
        Key::Name(_) => None,
    }
}

fn fold_object_member<'a>(
    obj: &oxc_ast::ast::ObjectExpression<'a>,
    key: &Key<'_>,
    ast: AstBuilder<'a>,
) -> Option<Expression<'a>> {
    use oxc_ast::ast::{ObjectPropertyKind, PropertyKind};
    // The object must be a plain bag of non-computed data properties with static
    // names: a spread, a computed key, or a getter/setter could add unknown keys
    // or run code, so we refuse to fold any access through it.
    let mut props: Vec<(&str, &Expression<'a>)> = Vec::with_capacity(obj.properties.len());
    for prop in &obj.properties {
        let ObjectPropertyKind::ObjectProperty(p) = prop else {
            return None; // spread
        };
        if p.computed || p.kind != PropertyKind::Init {
            return None; // computed key or accessor
        }
        let name = match &p.key {
            oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            oxc_ast::ast::PropertyKey::StringLiteral(s) => s.value.as_str(),
            _ => return None,
        };
        props.push((name, &p.value));
    }
    let want = match key {
        Key::Name(n) => (*n).to_string(),
        Key::Index(i) => i.to_string(),
    };
    // Last writer wins for duplicate keys. The selected value is kept; every
    // other property's value must be pure (about to be dropped).
    let pos = props.iter().rposition(|(n, _)| *n == want)?;
    let selected = props[pos].1;
    if contains_scoped_node(selected) {
        return None;
    }
    if props
        .iter()
        .enumerate()
        .any(|(i, (_, v))| i != pos && !is_pure_simple(v))
    {
        return None;
    }
    Some(selected.clone_in(ast.allocator))
}

fn fold_string_member<'a>(
    s: &oxc_ast::ast::StringLiteral<'a>,
    key: &Key<'_>,
    span: oxc_span::Span,
    ast: AstBuilder<'a>,
) -> Option<Expression<'a>> {
    // JS indexes strings by UTF-16 code unit; our value is UTF-8. Restrict to
    // ASCII, where code-unit, char, and byte indices all coincide.
    if !s.value.is_ascii() {
        return None;
    }
    let bytes = s.value.as_bytes();
    match key {
        Key::Name("length") => Some(ast.expression_numeric_literal(
            span,
            bytes.len() as f64,
            None,
            NumberBase::Decimal,
        )),
        Key::Index(i) => {
            let ch = *bytes.get(*i)? as char;
            let atom = ast.str(ch.encode_utf8(&mut [0u8; 1]));
            Some(ast.expression_string_literal(span, atom, None))
        }
        Key::Name(_) => None,
    }
}

/// Conservative evaluation context: treat anything that could observe or mutate
/// the world as side-effectful, so only pure expressions fold.
struct EvalCtx<'a, 'b> {
    ast: AstBuilder<'a>,
    scoping: &'b Scoping,
}

impl<'a, 'b> GlobalContext<'a> for EvalCtx<'a, 'b> {
    fn is_global_reference(&self, reference: &IdentifierReference<'a>) -> bool {
        match reference.reference_id.get() {
            Some(rid) => self.scoping.get_reference(rid).symbol_id().is_none(),
            None => true,
        }
    }
}

impl<'a, 'b> MayHaveSideEffectsContext<'a> for EvalCtx<'a, 'b> {
    fn annotations(&self) -> bool {
        false
    }
    fn manual_pure_functions(&self, _callee: &Expression) -> bool {
        false
    }
    fn property_read_side_effects(&self) -> PropertyReadSideEffects {
        PropertyReadSideEffects::All
    }
    fn unknown_global_side_effects(&self) -> bool {
        true
    }
}

impl<'a, 'b> ConstantEvaluationCtx<'a> for EvalCtx<'a, 'b> {
    fn ast(&self) -> AstBuilder<'a> {
        self.ast
    }
}
