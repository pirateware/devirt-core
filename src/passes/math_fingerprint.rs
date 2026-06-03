//! Math-proxy fingerprinting.
//!
//! Obfuscators replace plain operators with tiny pure functions whose body is an
//! algebraic identity — `function f(a, b) { return 4*(b|a) - 3*(~b&a) + 6*~(b|a)
//! - 2*~(b^a) - ~a - (b|~a) - ~b + 1; }` is just `a + b` for every 32-bit
//! integer. There are thousands of these across samples. Rather than simplify the
//! algebra symbolically, we *run* each candidate against a wide battery of integer
//! inputs in the sandbox and check whether it agrees — value-and-type exactly
//! (`Object.is`) — with exactly one simple operator (`a+b`, `b-a`, `a^b`, `a<b`,
//! `~a`, …) on every input. If a unique operator matches, twice (to rule out
//! nondeterminism), we rewrite every call `f(x, y)` to `x <op> y` and let DCE drop
//! the now-dead function. This works even when the call arguments are runtime
//! variables — the whole point — because the fingerprint is established once, on
//! the function, not per call.
//!
//! ## Why this is sound
//!
//! * **Purity is structural.** We only consider a function whose body is a single
//!   `return` of an *arithmetic-shaped* expression: identifiers, numeric/boolean/
//!   null literals, and unary/binary/logical/conditional operators — no calls, no
//!   member access, no assignments, no `this`. Such a body has no side effect and
//!   no free name beyond its parameters and the constant globals
//!   `NaN`/`Infinity`/`undefined`, so replacing a call with an operator can never
//!   drop an effect or read outer state, and the function runs standalone in the
//!   sandbox.
//! * **Unique match.** A coincidental match is made implausible by a wide,
//!   discriminating battery (zeros, ±1, mixed signs, large ints, int32 extremes)
//!   *and* by requiring that exactly one candidate operator agrees — if two agree
//!   the battery failed to discriminate, so we bail rather than guess.
//! * **Determinism guard.** Each match is confirmed by evaluating twice; a
//!   non-deterministic body never matches.
//! * **Evaluation order.** An operator evaluates its operands left-to-right, the
//!   same order a call evaluates its arguments — so the non-swapped rewrite
//!   preserves argument-effect order for free. A *swapped* operator (`b - a`)
//!   would reverse that order, so we only emit it when both arguments are
//!   side-effect-free (identifier/literal reads), where reordering is unobservable.
//!
//! ## Soundness boundary
//!
//! Agreement is established by sampling over the integer domain these proxies are
//! built for — a bitwise `+`-proxy equals `+` only on int32 inputs, exactly the
//! domain the obfuscator ever feeds it — not proven over all doubles. A genuine
//! `return a + b` matches `+` everywhere and is rewritten with full soundness; a
//! bitwise polynomial is rewritten under the same int-domain assumption the
//! obfuscator already relies on. This mirrors how every practical deobfuscator
//! handles operator virtualisation.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, Box as ArenaBox, CloneIn};
use oxc_ast::ast::{
    BindingPattern, Expression, FormalParameters, FunctionBody, Program, Statement,
};
use oxc_ast::AstBuilder;
use oxc_ast_visit::Visit;
use oxc_codegen::Codegen;
use oxc_span::{GetSpan, SourceType, SPAN};
use oxc_syntax::operator::{BinaryOperator, LogicalOperator, UnaryOperator};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use crate::pass::Pass;
use crate::sandbox::{EvalValue, Sandbox};
use crate::util::build_scoping;

/// Bare global identifiers a pure arithmetic body may reference (constants with a
/// fixed value in the sandbox). Anything else free is an outer binding we cannot
/// reproduce standalone, so the candidate is rejected.
const CONST_GLOBALS: &[&str] = &["NaN", "Infinity", "undefined"];

/// Battery of `(a, b)` integer pairs. Spans zeros, ±1, small primes in both
/// orders, mixed signs, powers of two, byte/word boundaries, large magnitudes,
/// and the int32 extremes — enough to separate every candidate operator (so the
/// "exactly one match" requirement is meaningful) and make a coincidence absurd.
const BIN_INPUTS: &[(i64, i64)] = &[
    (0, 0),
    (0, 1),
    (1, 0),
    (1, 1),
    (2, 1),
    (1, 2),
    (2, 3),
    (3, 2),
    (5, 7),
    (7, 5),
    (-1, 1),
    (1, -1),
    (-1, -1),
    (-2, 3),
    (3, -2),
    (255, 255),
    (256, 1),
    (1, 256),
    (-256, 7),
    (7, -256),
    (1023, 12),
    (12, 1023),
    (65535, 3),
    (3, 65535),
    (123456, 789),
    (789, 123456),
    (-123456, 789),
    (2147483647, 1),
    (-2147483648, -1),
    (100, 3),
    (3, 100),
    (8, 2),
    (2, 8),
    (-7, -9),
    (11, 13),
    (13, 11),
    (0, 5),
    (5, 0),
    (0, -5),
    (-5, 0),
];

/// Battery of single integer inputs for unary candidates.
const UN_INPUTS: &[i64] = &[
    0,
    1,
    -1,
    2,
    -2,
    255,
    256,
    -256,
    1023,
    65535,
    123456,
    -123456,
    2147483647,
    -2147483648,
    7,
    -7,
    42,
    -42,
    3,
    5,
];

#[derive(Default)]
pub struct MathFingerprint {
    changed: bool,
}

impl Pass for MathFingerprint {
    fn name(&self) -> &'static str {
        "math-fingerprint"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // Name stability: a candidate must be declared exactly once and never
        // reassigned, so every `f(...)` call binds to the body we fingerprinted.
        let names = NameScan::scan(program);

        let mut coll = CandidateCollector {
            allocator,
            out: Vec::new(),
        };
        coll.visit_program(program);
        if coll.out.is_empty() {
            return false;
        }

        // Dedupe by name; a name claimed by two different candidates is ambiguous.
        let mut by_name: HashMap<String, Candidate<'a>> = HashMap::new();
        let mut dup: HashSet<String> = HashSet::new();
        for c in coll.out {
            if by_name.contains_key(&c.name) {
                dup.insert(c.name.clone());
            }
            by_name.insert(c.name.clone(), c);
        }
        by_name.retain(|n, _| {
            !dup.contains(n)
                && names.decl_counts.get(n).copied() == Some(1)
                && !names.reassigned.contains(n)
        });
        if by_name.is_empty() {
            return false;
        }

        // Define every candidate (name-unique) in one throwaway preamble and reuse
        // a single sandbox for all the per-function batteries.
        let ast = AstBuilder::new(allocator);
        let mut body = ast.vec();
        for c in by_name.values() {
            body.push(c.decl.clone_in(allocator));
        }
        let prog = ast.program(
            SPAN,
            SourceType::default(),
            "",
            ast.vec(),
            None,
            ast.vec(),
            body,
        );
        let code = Codegen::new().build(&prog).code;

        let Some(sandbox) = Sandbox::new() else {
            return false;
        };
        if !sandbox.setup(&code) {
            return false;
        }

        let mut matched: HashMap<String, Matched> = HashMap::new();
        for c in by_name.values() {
            if let Some(m) = fingerprint(&sandbox, &c.name, c.arity) {
                matched.insert(c.name.clone(), m);
            }
        }
        if matched.is_empty() {
            return false;
        }

        if std::env::var("MF_DEBUG").is_ok() {
            let mut sample: Vec<String> = matched
                .iter()
                .map(|(n, m)| format!("{n}={}", m.describe()))
                .collect();
            sample.sort();
            eprintln!(
                "[mf] candidates={} matched={} {:?}",
                by_name.len(),
                matched.len(),
                &sample[..sample.len().min(8)]
            );
        }

        let mut rw = Rewriter {
            matched: &matched,
            changed: false,
            _marker: std::marker::PhantomData,
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut rw, allocator, program, scoping, ());
        self.changed = rw.changed;
        self.changed
    }
}

// ---------------------------------------------------------------------------
// What a candidate collapsed to
// ---------------------------------------------------------------------------

/// The operator a function fingerprinted to, plus operand order for binary forms
/// (`swap` = the right argument is the left operand, e.g. `b - a`).
#[derive(Clone, Copy)]
enum Matched {
    /// `coerce` = the proxy equals the operator only on ToInt32-coerced operands
    /// (an MBA reconstruction with a bitwise body), so the rewrite must wrap each
    /// operand in `| 0` to stay faithful for non-integer arguments.
    Bin { op: BinaryOperator, swap: bool, coerce: bool },
    Log { op: LogicalOperator, swap: bool },
    Un { op: UnaryOperator, coerce: bool },
}

impl Matched {
    fn arity(&self) -> usize {
        match self {
            Matched::Un { .. } => 1,
            _ => 2,
        }
    }

    fn describe(&self) -> String {
        let c = |coerce: bool| if coerce { "|0" } else { "" };
        match self {
            Matched::Bin { op, swap, coerce } => {
                format!("{}{}{}", op.as_str(), if *swap { "~" } else { "" }, c(*coerce))
            }
            Matched::Log { op, swap } => format!("{}{}", op.as_str(), if *swap { "~" } else { "" }),
            Matched::Un { op, coerce } => format!("{}{}", op.as_str(), c(*coerce)),
        }
    }
}

// ---------------------------------------------------------------------------
// Candidate collection
// ---------------------------------------------------------------------------

struct Candidate<'a> {
    name: String,
    arity: usize,
    /// A standalone declaration of the function, cloned for the preamble.
    decl: Statement<'a>,
}

struct CandidateCollector<'a> {
    allocator: &'a Allocator,
    out: Vec<Candidate<'a>>,
}

impl<'a> CandidateCollector<'a> {
    /// Record a candidate from a `function name(params){ return <arith> }`
    /// declaration, if it fits the arithmetic-proxy grammar.
    fn try_func_decl(&mut self, func: &oxc_ast::ast::Function<'a>) {
        let Some(id) = &func.id else { return };
        let Some(b) = &func.body else { return };
        let Some(arity) = self.eligible(&func.params, b, false) else {
            return;
        };
        let cloned = func.clone_in(self.allocator);
        let decl = Statement::FunctionDeclaration(ArenaBox::new_in(cloned, self.allocator));
        self.out.push(Candidate {
            name: id.name.to_string(),
            arity,
            decl,
        });
    }

    /// Record a candidate from `var name = function/arrow`, rebuilt as a fresh
    /// single-declarator `var name = <init>;` for the preamble.
    fn try_var(&mut self, id: &oxc_ast::ast::BindingIdentifier<'a>, init: &Expression<'a>) {
        let arity = match init {
            Expression::FunctionExpression(f) => {
                let Some(b) = &f.body else { return };
                self.eligible(&f.params, b, false)
            }
            Expression::ArrowFunctionExpression(f) => {
                self.eligible(&f.params, &f.body, f.expression)
            }
            _ => return,
        };
        let Some(arity) = arity else { return };

        use oxc_ast::ast::VariableDeclarationKind::Var;
        let ast = AstBuilder::new(self.allocator);
        let id_pat = ast.binding_pattern_binding_identifier(SPAN, id.name);
        let declr = ast.variable_declarator(
            SPAN,
            Var,
            id_pat,
            oxc_ast::NONE,
            Some(init.clone_in(self.allocator)),
            false,
        );
        let mut decls = ast.vec();
        decls.push(declr);
        let decl = Statement::VariableDeclaration(
            ast.alloc(ast.variable_declaration(SPAN, Var, decls, false)),
        );
        self.out.push(Candidate {
            name: id.name.to_string(),
            arity,
            decl,
        });
    }

    /// Return the arity (1 or 2) if `params`/`body` is an eligible arithmetic
    /// proxy: 1–2 simple params, a single `return <expr>`, the expression
    /// arithmetic-shaped with at least one operator, and every free name a param
    /// or a constant global. `None` otherwise.
    fn eligible(
        &self,
        params: &FormalParameters<'a>,
        body: &FunctionBody<'a>,
        arrow_expr: bool,
    ) -> Option<usize> {
        let param_names = simple_param_names(params)?;
        if param_names.is_empty() || param_names.len() > 2 {
            return None;
        }
        let ret = single_return_expr(body, arrow_expr)?;
        if !is_arith_shaped(ret) || !has_operator(ret) {
            return None;
        }
        let allowed: HashSet<&str> = param_names.iter().map(String::as_str).collect();
        if !idents_within(ret, &allowed) {
            return None;
        }
        Some(param_names.len())
    }
}

impl<'a> Visit<'a> for CandidateCollector<'a> {
    fn visit_statement(&mut self, stmt: &Statement<'a>) {
        if let Statement::FunctionDeclaration(f) = stmt {
            self.try_func_decl(f);
        }
        oxc_ast_visit::walk::walk_statement(self, stmt);
    }

    fn visit_variable_declarator(&mut self, node: &oxc_ast::ast::VariableDeclarator<'a>) {
        if let BindingPattern::BindingIdentifier(id) = &node.id {
            if let Some(init) = &node.init {
                self.try_var(id, init);
            }
        }
        oxc_ast_visit::walk::walk_variable_declarator(self, node);
    }
}

fn simple_param_names(params: &FormalParameters<'_>) -> Option<Vec<String>> {
    if params.rest.is_some() {
        return None;
    }
    let mut out = Vec::new();
    for item in &params.items {
        let BindingPattern::BindingIdentifier(id) = &item.pattern else {
            return None;
        };
        out.push(id.name.to_string());
    }
    Some(out)
}

fn single_return_expr<'b, 'a>(
    body: &'b FunctionBody<'a>,
    arrow_expr: bool,
) -> Option<&'b Expression<'a>> {
    if body.statements.len() != 1 {
        return None;
    }
    match &body.statements[0] {
        Statement::ReturnStatement(r) => r.argument.as_ref(),
        Statement::ExpressionStatement(e) if arrow_expr => Some(&e.expression),
        _ => None,
    }
}

/// An arithmetic-shaped expression: identifiers, numeric/boolean/null literals,
/// and unary/binary/logical/conditional operators over the same. No member
/// access, call, string/bigint literal, `this`, or `delete`/`in`/`instanceof`.
/// This grammar is *pure by construction*: it has no side effect and no free name
/// beyond its leaf identifiers.
fn is_arith_shaped(e: &Expression<'_>) -> bool {
    match e {
        Expression::Identifier(_)
        | Expression::NumericLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_) => true,
        Expression::UnaryExpression(u) => {
            !matches!(u.operator, UnaryOperator::Delete) && is_arith_shaped(&u.argument)
        }
        Expression::BinaryExpression(b) => {
            !matches!(b.operator, BinaryOperator::In | BinaryOperator::Instanceof)
                && is_arith_shaped(&b.left)
                && is_arith_shaped(&b.right)
        }
        Expression::LogicalExpression(l) => is_arith_shaped(&l.left) && is_arith_shaped(&l.right),
        Expression::ConditionalExpression(c) => {
            is_arith_shaped(&c.test)
                && is_arith_shaped(&c.consequent)
                && is_arith_shaped(&c.alternate)
        }
        _ => false,
    }
}

/// The expression contains an operator at the top — a bare identifier or literal
/// can never fingerprint to an operator, so skip it (and the wasted sandbox runs).
fn has_operator(e: &Expression<'_>) -> bool {
    matches!(
        e,
        Expression::UnaryExpression(_)
            | Expression::BinaryExpression(_)
            | Expression::LogicalExpression(_)
            | Expression::ConditionalExpression(_)
    )
}

/// Every identifier leaf is an allowed param or a constant global.
fn idents_within(e: &Expression<'_>, allowed: &HashSet<&str>) -> bool {
    match e {
        Expression::Identifier(id) => {
            allowed.contains(id.name.as_str()) || CONST_GLOBALS.contains(&id.name.as_str())
        }
        Expression::NumericLiteral(_)
        | Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_) => true,
        Expression::UnaryExpression(u) => idents_within(&u.argument, allowed),
        Expression::BinaryExpression(b) => {
            idents_within(&b.left, allowed) && idents_within(&b.right, allowed)
        }
        Expression::LogicalExpression(l) => {
            idents_within(&l.left, allowed) && idents_within(&l.right, allowed)
        }
        Expression::ConditionalExpression(c) => {
            idents_within(&c.test, allowed)
                && idents_within(&c.consequent, allowed)
                && idents_within(&c.alternate, allowed)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Name stability scan
// ---------------------------------------------------------------------------

#[derive(Default)]
struct NameScan {
    decl_counts: HashMap<String, usize>,
    reassigned: HashSet<String>,
}

impl NameScan {
    fn scan(program: &Program<'_>) -> NameScan {
        let mut s = NameScan::default();
        s.visit_program(program);
        s
    }
}

impl<'a> Visit<'a> for NameScan {
    fn visit_binding_identifier(&mut self, it: &oxc_ast::ast::BindingIdentifier<'a>) {
        *self.decl_counts.entry(it.name.to_string()).or_default() += 1;
    }

    fn visit_assignment_expression(&mut self, node: &oxc_ast::ast::AssignmentExpression<'a>) {
        if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &node.left {
            self.reassigned.insert(id.name.to_string());
        }
        oxc_ast_visit::walk::walk_assignment_expression(self, node);
    }

    fn visit_update_expression(&mut self, node: &oxc_ast::ast::UpdateExpression<'a>) {
        if let oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) = &node.argument
        {
            self.reassigned.insert(id.name.to_string());
        }
        oxc_ast_visit::walk::walk_update_expression(self, node);
    }
}

// ---------------------------------------------------------------------------
// Fingerprinting
// ---------------------------------------------------------------------------

/// Candidate operators of the given arity, paired with the JS source token used
/// to compute the expected value in the battery.
fn candidates(arity: usize) -> Vec<(String, Matched)> {
    let mut v: Vec<(String, Matched)> = Vec::new();
    if arity == 2 {
        // Self-coercing bitwise ops: they already operate on ToInt32 of both
        // operands, so `a OP b` is faithful for every argument type — no coerced
        // variant (adding one would match identically and look ambiguous).
        for (tok, op) in [
            ("&", BinaryOperator::BitwiseAnd),
            ("|", BinaryOperator::BitwiseOR),
            ("^", BinaryOperator::BitwiseXOR),
        ] {
            v.push((tok.to_string(), Matched::Bin { op, swap: false, coerce: false }));
        }
        for (tok, op) in [
            ("<<", BinaryOperator::ShiftLeft),
            (">>", BinaryOperator::ShiftRight),
            (">>>", BinaryOperator::ShiftRightZeroFill),
        ] {
            v.push((tok.to_string(), Matched::Bin { op, swap: false, coerce: false }));
            v.push((tok.to_string(), Matched::Bin { op, swap: true, coerce: false }));
        }
        // Arithmetic/relational ops do NOT coerce to int32, but an MBA
        // reconstruction (a bitwise body) only equals the operator on coerced
        // operands. So each gets a bare and a coerced candidate; the non-integer
        // probes in the battery make the two disagree, so a proxy matches exactly
        // one (a true `+` matches bare; an MBA `+` matches `(a|0)+(b|0)`).
        for (tok, op) in [
            ("+", BinaryOperator::Addition),
            ("*", BinaryOperator::Multiplication),
        ] {
            for coerce in [false, true] {
                v.push((tok.to_string(), Matched::Bin { op, swap: false, coerce }));
            }
        }
        for (tok, op) in [
            ("-", BinaryOperator::Subtraction),
            ("/", BinaryOperator::Division),
            ("%", BinaryOperator::Remainder),
            ("**", BinaryOperator::Exponential),
        ] {
            for swap in [false, true] {
                for coerce in [false, true] {
                    v.push((tok.to_string(), Matched::Bin { op, swap, coerce }));
                }
            }
        }
        for (tok, op) in [
            ("<", BinaryOperator::LessThan),
            (">", BinaryOperator::GreaterThan),
            ("<=", BinaryOperator::LessEqualThan),
            (">=", BinaryOperator::GreaterEqualThan),
        ] {
            for coerce in [false, true] {
                v.push((tok.to_string(), Matched::Bin { op, swap: false, coerce }));
            }
        }
        // Logical short-circuit returns an actual operand, so a coerced form is
        // never faithful; only a bare match (across the full battery) is accepted.
        for (tok, op) in [("&&", LogicalOperator::And), ("||", LogicalOperator::Or)] {
            v.push((tok.to_string(), Matched::Log { op, swap: false }));
            v.push((tok.to_string(), Matched::Log { op, swap: true }));
        }
    } else {
        // `~` self-coerces; negation has a coerced (`-(a|0)`, the `~a+1` MBA) form;
        // the rest are bare-only.
        v.push(("~".to_string(), Matched::Un { op: UnaryOperator::BitwiseNot, coerce: false }));
        for coerce in [false, true] {
            v.push(("-".to_string(), Matched::Un { op: UnaryOperator::UnaryNegation, coerce }));
        }
        for (tok, op) in [
            ("!", UnaryOperator::LogicalNot),
            ("void ", UnaryOperator::Void),
            ("typeof ", UnaryOperator::Typeof),
        ] {
            v.push((tok.to_string(), Matched::Un { op, coerce: false }));
        }
    }
    v
}

/// Fingerprint `name` (defined in the sandbox) at the given arity. Returns the
/// unique matching operator, or `None` if zero or more than one match.
fn fingerprint(sandbox: &Sandbox, name: &str, arity: usize) -> Option<Matched> {
    let mut found: Option<Matched> = None;
    for (tok, matched) in candidates(arity) {
        let snippet = if arity == 2 {
            bin_snippet(name, &tok, &matched)
        } else {
            un_snippet(name, &tok, &matched)
        };
        if matches_twice(sandbox, &snippet) {
            if found.is_some() {
                return None; // battery did not discriminate — refuse to guess
            }
            found = Some(matched);
        }
    }
    found
}

/// Evaluate `snippet` (a battery that returns a boolean) twice; a match requires
/// `true` both times, so a non-deterministic body never matches.
fn matches_twice(sandbox: &Sandbox, snippet: &str) -> bool {
    matches!(sandbox.eval_to_value(snippet), Some(EvalValue::Bool(true)))
        && matches!(sandbox.eval_to_value(snippet), Some(EvalValue::Bool(true)))
}

fn bin_snippet(name: &str, tok: &str, m: &Matched) -> String {
    let (swap, coerce) = match m {
        Matched::Bin { swap, coerce, .. } => (*swap, *coerce),
        Matched::Log { swap, .. } => (*swap, false),
        Matched::Un { .. } => (false, false),
    };
    let (l, r) = if swap { ("b", "a") } else { ("a", "b") };
    // `(a|0)` forces ToInt32 on the operand, matching an MBA reconstruction.
    let wrap = |x: &str| if coerce { format!("({x}|0)") } else { x.to_string() };
    let (l, r) = (wrap(l), wrap(r));
    let pairs = bin_inputs_js();
    format!(
        "(function(){{var I={pairs};for(var i=0;i<I.length;i++){{var a=I[i][0],b=I[i][1];\
         if(!Object.is({name}(a,b),(({l}){tok}({r}))))return false;}}return true;}})()"
    )
}

fn un_snippet(name: &str, tok: &str, m: &Matched) -> String {
    let coerce = matches!(m, Matched::Un { coerce: true, .. });
    let a = if coerce { "(a|0)" } else { "a" };
    let vals = un_inputs_js();
    format!(
        "(function(){{var I={vals};for(var i=0;i<I.length;i++){{var a=I[i];\
         if(!Object.is({name}(a),({tok}({a}))))return false;}}return true;}})()"
    )
}

/// Non-integer probes appended to the binary battery. These make a bare operator
/// (`a OP b`) and its ToInt32-coerced form (`(a|0) OP (b|0)`) disagree — via
/// string concatenation/comparison, fractional values, `-0`, and magnitudes past
/// int32 — so a proxy matches exactly one of the two and a coercion-divergent
/// proxy is never rewritten to the wrong form.
const NONINT_BIN: &[&str] = &[
    "[0.5,0.5]", "[1.5,2.5]", "[-1.5,2.5]", "[2.5,-0]",
    "[\"3\",\"4\"]", "[\"a\",\"b\"]", "[\"5\",2]", "[6,\"2\"]",
    "[4294967296,1]", "[1,4294967296]", "[3000000000,3000000000]",
];

/// Non-integer probes for the unary battery (same purpose).
const NONINT_UN: &[&str] = &["0.5", "-0.5", "1.5", "\"5\"", "\"a\"", "-0", "4294967296"];

fn bin_inputs_js() -> String {
    let mut s = String::from("[");
    for (i, (a, b)) in BIN_INPUTS.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("[{a},{b}]"));
    }
    for extra in NONINT_BIN {
        s.push(',');
        s.push_str(extra);
    }
    s.push(']');
    s
}

fn un_inputs_js() -> String {
    let mut s = String::from("[");
    for (i, a) in UN_INPUTS.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&a.to_string());
    }
    for extra in NONINT_UN {
        s.push(',');
        s.push_str(extra);
    }
    s.push(']');
    s
}

// ---------------------------------------------------------------------------
// Rewriting
// ---------------------------------------------------------------------------

struct Rewriter<'a, 'r> {
    matched: &'r HashMap<String, Matched>,
    changed: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'r> Traverse<'a, ()> for Rewriter<'a, 'r> {
    fn enter_expression(&mut self, expr: &mut Expression<'a>, ctx: &mut TraverseCtx<'a, ()>) {
        let replacement: Option<Expression<'a>> = {
            let Expression::CallExpression(call) = &*expr else {
                return;
            };
            let Expression::Identifier(callee) = &call.callee else {
                return;
            };
            let Some(m) = self.matched.get(callee.name.as_str()).copied() else {
                return;
            };
            if call.arguments.len() != m.arity() {
                return;
            }
            // Collect plain-expression arguments (no spread).
            let mut args: Vec<&Expression<'a>> = Vec::with_capacity(call.arguments.len());
            for a in &call.arguments {
                match a.as_expression() {
                    Some(e) => args.push(e),
                    None => return,
                }
            }
            // A spliced clone of a function/class/arrow resets its scope cell and
            // panics the traversal; arithmetic args almost never contain one, so
            // just decline (semantics unaffected).
            if args.iter().any(|e| contains_scoped_node(e)) {
                return;
            }

            let span = call.span();
            match m {
                Matched::Un { op, coerce } => {
                    let mut arg = args[0].clone_in(ctx.ast.allocator);
                    if coerce {
                        arg = coerce_i32(ctx, arg);
                    }
                    Some(ctx.ast.expression_unary(span, op, arg))
                }
                Matched::Bin { op, swap, coerce } => {
                    if swap && !args.iter().all(|e| is_side_effect_free(e)) {
                        return; // swapping operands would reorder argument effects
                    }
                    let (li, ri) = if swap { (1, 0) } else { (0, 1) };
                    let mut left = args[li].clone_in(ctx.ast.allocator);
                    let mut right = args[ri].clone_in(ctx.ast.allocator);
                    if coerce {
                        left = coerce_i32(ctx, left);
                        right = coerce_i32(ctx, right);
                    }
                    Some(ctx.ast.expression_binary(span, left, op, right))
                }
                Matched::Log { op, swap } => {
                    if swap && !args.iter().all(|e| is_side_effect_free(e)) {
                        return;
                    }
                    let (li, ri) = if swap { (1, 0) } else { (0, 1) };
                    let left = args[li].clone_in(ctx.ast.allocator);
                    let right = args[ri].clone_in(ctx.ast.allocator);
                    Some(ctx.ast.expression_logical(span, left, op, right))
                }
            }
        };

        if let Some(r) = replacement {
            *expr = r;
            self.changed = true;
        }
    }
}

/// Wrap `e` as `(e | 0)` — forces ToInt32, making a rewrite faithful to an MBA
/// proxy whose bitwise body coerces its operands.
fn coerce_i32<'a>(ctx: &TraverseCtx<'a, ()>, e: Expression<'a>) -> Expression<'a> {
    let zero = ctx.ast.expression_numeric_literal(
        SPAN,
        0.0,
        None,
        oxc_syntax::number::NumberBase::Decimal,
    );
    ctx.ast.expression_binary(SPAN, e, BinaryOperator::BitwiseOR, zero)
}

fn is_side_effect_free(expr: &Expression<'_>) -> bool {
    matches!(
        expr,
        Expression::Identifier(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_)
            | Expression::BigIntLiteral(_)
    )
}

/// Whether `expr` contains a function/arrow/class node, whose clone would carry a
/// reset scope cell the traverse walker later unwraps.
fn contains_scoped_node(expr: &Expression<'_>) -> bool {
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
