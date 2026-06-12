//! Reconstruct a `Function` constructor with constant string arguments into a
//! function expression.
//!
//! Obfuscators hide code from static reading behind the `Function` constructor:
//! `new Function('a', 'b', 'return a + b')`. When every argument is a string
//! literal the function is fully determined, so we parse it and splice back the
//! equivalent `function (a, b) { return a + b; }`, which the dataflow passes
//! (inline, pure-eval, const-fold) can then see through.
//!
//! Soundness — a `Function`-constructed function closes over the **global** scope
//! only, whereas a function *literal* at the splice site closes over the local
//! scope. They are equivalent only if the body cannot capture a local binding, so
//! we splice only when the parsed body's free names are all either parameters
//! (bound by the function itself) or side-effect-free globals (the same gate
//! decoder-lift / pure-eval trust). Any other free name — which could resolve to a
//! local at the splice site — declines. Two further guards keep it safe:
//!   * `Function` must be the global: if any binding named `Function` exists in the
//!     program, a use might be that rebinding, so the whole pass declines;
//!   * the constructed source must parse to exactly one statement (one function
//!     expression), so a body crafted to break out of the wrapper
//!     (`"}); evil(); ({"`) is rejected.
//!
//! Uses `VisitMut` rather than `oxc_traverse`: it splices a freshly-parsed function
//! node (no `scope_id`), which the semantic-aware traverser would panic descending
//! into. The plain AST visitor has no such bookkeeping.

use oxc_allocator::{Allocator, CloneIn};
use oxc_ast::ast::{Argument, Expression, Program, Statement};
use oxc_ast_visit::{walk_mut, VisitMut};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;

use crate::pass::Pass;
use crate::passes::decoder_lift::freevars_of_expr;
use crate::util::build_scoping;

pub struct FunctionCtor;

impl Pass for FunctionCtor {
    fn name(&self) -> &'static str {
        "function-ctor"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // If `Function` is bound anywhere in the program, a use might be that local
        // rebinding rather than the global constructor — decline the whole pass.
        let scoping = build_scoping(program);
        if scoping.symbol_names().any(|n| n == "Function") {
            return false;
        }
        let mut v = Reconstruct {
            alloc: allocator,
            changed: false,
        };
        walk_mut::walk_program(&mut v, program);
        v.changed
    }
}

struct Reconstruct<'a> {
    alloc: &'a Allocator,
    changed: bool,
}

impl<'a> VisitMut<'a> for Reconstruct<'a> {
    fn visit_expression(&mut self, expr: &mut Expression<'a>) {
        if let Some(func) = self.try_reconstruct(expr) {
            *expr = func;
            self.changed = true;
            return; // do not descend into the reconstructed node
        }
        walk_mut::walk_expression(self, expr);
    }
}

impl<'a> Reconstruct<'a> {
    /// If `expr` is `new Function(…)` / `Function(…)` with all-string-literal
    /// arguments, reconstruct the equivalent function expression. `None` otherwise.
    fn try_reconstruct(&self, expr: &Expression<'a>) -> Option<Expression<'a>> {
        let (callee, args) = match expr {
            Expression::NewExpression(n) => (&n.callee, &n.arguments),
            Expression::CallExpression(c) => (&c.callee, &c.arguments),
            _ => return None,
        };
        let Expression::Identifier(id) = callee else {
            return None;
        };
        if id.name != "Function" {
            return None;
        }
        // Every argument must be a plain string literal; the last is the body, the
        // rest are parameter lists. Zero arguments → no body to reconstruct.
        if args.is_empty() {
            return None;
        }
        let mut strs: Vec<&str> = Vec::with_capacity(args.len());
        for a in args {
            let Argument::StringLiteral(s) = a else {
                return None;
            };
            strs.push(s.value.as_str());
        }
        let body = strs.pop().unwrap(); // args non-empty
        let params = strs.join(",");
        build_function_expr(&params, body, self.alloc)
    }
}

/// Parse `(function (<params>) { <body> })` in a throwaway arena and, if it is a
/// single, capture-free function expression, clone it into `main`. Returns `None`
/// on a parse error, a multi-statement breakout, or a body that could capture a
/// local (a free name outside the parameters and the allowed globals).
fn build_function_expr<'a>(
    params: &str,
    body: &str,
    main: &'a Allocator,
) -> Option<Expression<'a>> {
    let src = format!("(function ({params}) {{\n{body}\n}})");
    let tmp = Allocator::default();
    let parsed = Parser::new(&tmp, &src, SourceType::default())
        .with_options(ParseOptions {
            preserve_parens: false,
            ..ParseOptions::default()
        })
        .parse();
    // A parse error, or anything other than exactly one expression statement, means
    // the arguments were not a clean function (or tried to break out of the wrapper).
    if !parsed.errors.is_empty() || parsed.program.body.len() != 1 {
        return None;
    }
    let Some(Statement::ExpressionStatement(es)) = parsed.program.body.first() else {
        return None;
    };
    let Expression::FunctionExpression(_) = &es.expression else {
        return None;
    };
    // Capture gate: the body may reference only its own parameters (bound inside the
    // function) and side-effect-free globals — never a name that could bind to a
    // local at the splice site.
    if !freevars_of_expr(&es.expression).all_allowed(&Default::default(), None, &Default::default())
    {
        return None;
    }
    Some(es.expression.clone_in(main))
}
