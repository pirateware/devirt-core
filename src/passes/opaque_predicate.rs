//! Opaque-predicate elimination.
//!
//! Obfuscators guard large decoy payloads behind a predicate that is *always*
//! false but not syntactically constant, so naive const-folding leaves it — and
//! the dead code it gates — in place. The dominant shape (PerimeterX/HUMAN) is a
//! property-existence probe against an empty "trap" function:
//!
//! ```js
//! function t() {}                       // never given a property
//! if ("N8dRlF2" in t && (function(){ … decoy … })()) …
//! ```
//!
//! `"N8dRlF2" in t` is statically `false` (a function object has no such
//! property), so the whole `&&` short-circuits and the decoy never runs. We fold
//! the `in` to `false`, then short-circuit the enclosing logical op so the decoy
//! expression is dropped — leaving `if (false) …` / a bare `false`, which the
//! const-fold and DCE passes downstream remove.
//!
//! Soundness — we only fold `"<key>" in <fn>` to `false` when:
//!  * `<fn>` is a *function declaration with an empty body*, declared exactly
//!    once in the whole program (no shadowing, so the name resolves unambiguously
//!    without symbol tracking), and
//!  * that name never escapes — every reference is either a direct call
//!    `fn(…)` (a no-op that cannot add a property) or the right operand of an
//!    `in` test. Any other use (an argument, an alias, a member write, a
//!    reassignment) disqualifies it, since a callee could add the very property
//!    we are about to prove absent.
//!  * `<key>` is not a built-in function/object property (`length`, `call`,
//!    `constructor`, …) — those genuinely *are* `in` a function, so they decline.
//!
//! The logical short-circuit fold (`false && x → false`, `true || x → true`) is
//! the standard one and applies whenever the left operand is a constant literal,
//! which is exactly what the `in` fold produces.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, TakeIn};
use oxc_ast::ast::{
    BinaryExpression, BindingIdentifier, CallExpression, Expression, Function, FunctionType,
    IdentifierReference, Program,
};
use oxc_ast_visit::Visit;
use oxc_syntax::operator::{BinaryOperator, LogicalOperator};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct OpaquePredicate {
    changed: bool,
}

impl Pass for OpaquePredicate {
    fn name(&self) -> &'static str {
        "opaque-predicate"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // Phase 1 — candidate empty function declarations, declared exactly once.
        let mut decl = DeclScan::default();
        decl.visit_program(program);
        let mut candidates: HashSet<String> = decl
            .empty_fn_decls
            .into_iter()
            .filter(|n| decl.decl_count.get(n).copied() == Some(1))
            .collect();
        if candidates.is_empty() {
            return false;
        }

        // Phase 2 — drop any candidate that escapes (referenced anywhere but as a
        // direct callee or an `in` right operand).
        let mut dq = Disqualify {
            candidates: &candidates,
            unsafe_refs: HashSet::new(),
        };
        dq.visit_program(program);
        let unsafe_refs = dq.unsafe_refs;
        candidates.retain(|n| !unsafe_refs.contains(n));
        if candidates.is_empty() {
            return false;
        }

        // Phase 3 — rewrite.
        let mut rw = Rewriter {
            empty_fns: candidates,
            changed: false,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut rw, allocator, program, scoping, ());
        self.changed = rw.changed;
        self.changed
    }
}

// ---------------------------------------------------------------------------
// Phase 1: declarations
// ---------------------------------------------------------------------------

#[derive(Default)]
struct DeclScan {
    /// Number of binding sites per name, at any scope (function/var/param/class).
    /// A candidate must have exactly one, so the name resolves unambiguously.
    decl_count: HashMap<String, usize>,
    /// Names of empty-bodied function *declarations*.
    empty_fn_decls: HashSet<String>,
}

impl<'a> Visit<'a> for DeclScan {
    fn visit_binding_identifier(&mut self, id: &BindingIdentifier<'a>) {
        *self.decl_count.entry(id.name.to_string()).or_insert(0) += 1;
    }

    fn visit_function(&mut self, func: &Function<'a>, flags: oxc_semantic::ScopeFlags) {
        if func.r#type == FunctionType::FunctionDeclaration {
            if let Some(id) = &func.id {
                let empty = func
                    .body
                    .as_ref()
                    .is_some_and(|b| b.statements.is_empty());
                if empty {
                    self.empty_fn_decls.insert(id.name.to_string());
                }
            }
        }
        oxc_ast_visit::walk::walk_function(self, func, flags);
    }
}

// ---------------------------------------------------------------------------
// Phase 2: escape analysis
// ---------------------------------------------------------------------------

struct Disqualify<'c> {
    candidates: &'c HashSet<String>,
    unsafe_refs: HashSet<String>,
}

impl<'a> Visit<'a> for Disqualify<'_> {
    fn visit_identifier_reference(&mut self, id: &IdentifierReference<'a>) {
        // Reached only for references the safe-context overrides below did *not*
        // skip — i.e. an escape. Mark the candidate as disqualified.
        if self.candidates.contains(id.name.as_str()) {
            self.unsafe_refs.insert(id.name.to_string());
        }
    }

    fn visit_call_expression(&mut self, node: &CallExpression<'a>) {
        // `NAME(…)` — calling an empty function is a no-op that cannot add a
        // property, so the callee identifier is not an escape. Visit only the
        // arguments. `NAME.m(…)` / `f(NAME)` fall through and disqualify.
        if matches!(&node.callee, Expression::Identifier(_)) {
            for arg in &node.arguments {
                self.visit_argument(arg);
            }
            return;
        }
        oxc_ast_visit::walk::walk_call_expression(self, node);
    }

    fn visit_binary_expression(&mut self, node: &BinaryExpression<'a>) {
        // `"<key>" in NAME` — the right operand is the safe use we intend to fold;
        // visit only the left so it does not count as an escape.
        if node.operator == BinaryOperator::In && matches!(&node.right, Expression::Identifier(_)) {
            self.visit_expression(&node.left);
            return;
        }
        oxc_ast_visit::walk::walk_binary_expression(self, node);
    }
}

// ---------------------------------------------------------------------------
// Phase 3: rewrite
// ---------------------------------------------------------------------------

struct Rewriter {
    empty_fns: HashSet<String>,
    changed: bool,
}

impl<'a> Traverse<'a, ()> for Rewriter {
    fn exit_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        // `"<key>" in <emptyFn>` → false (when `<key>` is not a real fn/object prop).
        if let Expression::BinaryExpression(b) = &*expr {
            if b.operator == BinaryOperator::In {
                if let (Expression::StringLiteral(key), Expression::Identifier(id)) =
                    (&b.left, &b.right)
                {
                    if self.empty_fns.contains(id.name.as_str())
                        && !is_builtin_prop(key.value.as_str())
                    {
                        let span = b.span;
                        *expr = ctx.ast.expression_boolean_literal(span, false);
                        self.changed = true;
                        return;
                    }
                }
            }
        }

        // Short-circuit a logical op whose left operand is now a constant literal.
        // `false && x → false`, `true && x → x`, `true || x → true`, `false || x → x`.
        // We visit bottom-up (exit), so the `in` fold above has already run on the
        // left operand when we reach the enclosing `&&`.
        if let Expression::LogicalExpression(l) = &mut *expr {
            let Some(truthy) = literal_truthy(&l.left) else {
                return;
            };
            let take_left = match l.operator {
                LogicalOperator::And => !truthy, // false&&x → left(false); true&&x → right
                LogicalOperator::Or => truthy,   // true||x → left;        false||x → right
                LogicalOperator::Coalesce => return,
            };
            let new = if take_left {
                l.left.take_in(ctx.ast)
            } else {
                l.right.take_in(ctx.ast)
            };
            *expr = new;
            self.changed = true;
        }
    }
}

/// Truthiness of a primitive *literal*, or `None` for anything else (an
/// identifier, a call, …). `undefined` is an identifier, not a literal, so it is
/// conservatively excluded.
fn literal_truthy(e: &Expression<'_>) -> Option<bool> {
    match e {
        Expression::BooleanLiteral(b) => Some(b.value),
        Expression::NullLiteral(_) => Some(false),
        Expression::NumericLiteral(n) => Some(n.value != 0.0 && !n.value.is_nan()),
        Expression::StringLiteral(s) => Some(!s.value.is_empty()),
        _ => None,
    }
}

/// Property names that genuinely exist `in` a function object (its own
/// properties, plus `Function.prototype` and `Object.prototype`). For these,
/// `"<key>" in fn` is `true`, so we must decline to fold.
fn is_builtin_prop(k: &str) -> bool {
    matches!(
        k,
        "length"
            | "name"
            | "prototype"
            | "arguments"
            | "caller"
            | "call"
            | "apply"
            | "bind"
            | "constructor"
            | "toString"
            | "toLocaleString"
            | "valueOf"
            | "hasOwnProperty"
            | "isPrototypeOf"
            | "propertyIsEnumerable"
            | "__proto__"
            | "__defineGetter__"
            | "__defineSetter__"
            | "__lookupGetter__"
            | "__lookupSetter__"
    )
}
