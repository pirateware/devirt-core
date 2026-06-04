//! String-array-order control-flow unflattening (the Cloudflare CFF shape).
//!
//! A second flattening family encodes execution order not in an integer state
//! variable but in a *static array of keys* walked by a monotonically advancing
//! cursor:
//! ```text
//! for (a = "0|8|9|10|4|3|6|7|1|2|5".split("|"), i = 0; true;)
//!     switch (a[i++]) { case "0": …; continue; case "1": …; continue; … }
//! ```
//! The execution order is *exactly the array order*: the cursor reads
//! `a[0], a[1], …` in turn and dispatches each. Since the array is a compile-time
//! constant and the cursor only ever advances by one (`i++`), this is a fixed
//! linear chain we can execute at compile time — emit the case bodies in array
//! order and the loop disappears.
//!
//! This is the cursor-sequence generalization of the integer-state unflattener in
//! `unflatten.rs`: there the "next state" is computed from a transition assignment;
//! here it is simply "the next array element". The case keys may be strings (the
//! `.split("|")` form) or — after const folding produces a pre-split literal array
//! — anything an array element resolves to; we support string and integer keys.
//!
//! Soundness is by construction + heavy gating. We transform a `for` loop only when
//! *all* of the following hold, and otherwise leave it untouched (a sound no-op):
//!   * the loop is `for (<init>; true; )` / `for (<init>; ; )` — i.e. it never
//!     exits via the test, only via a terminal case (or running off the chain);
//!   * `<init>` establishes, in order, the order array `a` (= a provably constant
//!     array: a `"…".split("…")` on string literals, or a literal array of
//!     string/number elements) and the cursor `i = 0`;
//!   * the loop body is `switch (a[i++]) { … }` (optionally the switch is the sole
//!     statement of a block, optionally followed by a `break`);
//!   * the switch has no `default` clause;
//!   * `a` is never mutated anywhere in its scope (checked via
//!     `mutable_aggregate_tables`) and, together with `i`, is referenced *only* in
//!     the init and as the `a[i++]` discriminant — so no case body can observe or
//!     alter the dispatch cursor;
//!   * every case body is dispatch-clean (does not read `a` or `i`), ends in a
//!     `continue` (advance to the next key) or a `return`/`throw` terminator, and
//!     contains no other top-level control flow;
//!   * the simulated walk visits each array slot at most once (the cursor is
//!     strictly monotonic, so this is automatic) and terminates by running off the
//!     end of the array or hitting a terminal case.
//!
//! Because `a`/`i` are dispatch-only and the walk is deterministic and acyclic, the
//! emitted sequence runs the same statements in the same order as the dispatcher.

use std::collections::{HashMap, HashSet};

use oxc_allocator::{Allocator, CloneIn, Vec as ArenaVec};
use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, Expression, ForStatementInit, Program, Statement,
    SwitchStatement, UpdateOperator,
};
use oxc_ast::AstBuilder;
use oxc_syntax::operator::AssignmentOperator;

use super::unflatten::{flatten_case, is_top_level_control, mentions};
use crate::ir::mutable_aggregate_tables;
use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct UnflattenStringOrder;

impl Pass for UnflattenStringOrder {
    fn name(&self) -> &'static str {
        "unflatten-string-order"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // Program-wide immutability gate: the set of aggregate (array/object) local
        // bindings that are NOT provably write-once-and-never-mutated. An order
        // array whose name is in this set could be mutated (`a.push(…)`, `a[k] = …`)
        // and is rejected. Name-keyed to match the structural analysis below.
        let scoping = build_scoping(program);
        let mutable = mutable_aggregate_tables(program, &scoping);
        // Program-wide census of every identifier *reference* (a read or write) by
        // name. The escape gate compares a candidate name's total reference count
        // against its count inside the for-statement: if they match, every read/write
        // of that name lives *inside* the loop (the for-init write + the `a[i++]`
        // read), so the cursor is dispatch-only. Binding-id declarations are *not*
        // counted — a binding without a write cannot change the value, and the sole
        // write (the for-init) is inside the loop. Counting by name (not symbol) is
        // conservative: a shadowed unrelated binding of the same name inflates the
        // global count and makes the gate decline, which is sound.
        let mut census = NameCensus {
            counts: HashMap::new(),
        };
        oxc_ast_visit::Visit::visit_program(&mut census, program);
        let mut w = Walker {
            ast: AstBuilder::new(allocator),
            changed: false,
            mutable,
            global_counts: census.counts,
        };
        w.lists(&mut program.body);
        w.changed
    }
}

/// Counts every identifier *reference* (read/write) occurrence by name. Binding
/// declarations are deliberately not counted (see the escape gate rationale).
struct NameCensus {
    counts: HashMap<String, usize>,
}

impl<'a> oxc_ast_visit::Visit<'a> for NameCensus {
    fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
        *self.counts.entry(it.name.to_string()).or_default() += 1;
    }
}

/// Count identifier-reference occurrences within a single statement (escape gate).
fn count_name_in_stmt(stmt: &Statement<'_>, name: &str) -> usize {
    let mut c = NameCensus {
        counts: HashMap::new(),
    };
    oxc_ast_visit::Visit::visit_statement(&mut c, stmt);
    c.counts.get(name).copied().unwrap_or(0)
}

/// The rewrite is a **hand-written recursion**, not `traverse_mut`: the linearized
/// sequence is built by `clone_in`, and oxc's walker panics if it descends into a
/// cloned `Function`/arrow whose `scope_id` is stale (see `unflatten_cfg`). The
/// recursion descends into each list's *existing* children first, then performs the
/// replacement, which it never re-visits.
struct Walker<'a> {
    ast: AstBuilder<'a>,
    changed: bool,
    mutable: HashSet<String>,
    /// Program-wide occurrence count per identifier name (the escape census).
    global_counts: HashMap<String, usize>,
}

impl<'a> Walker<'a> {
    fn lists(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>) {
        for stmt in stmts.iter_mut() {
            self.recurse_stmt(stmt);
        }
        self.try_recover(stmts);
    }

    fn try_recover(&mut self, stmts: &mut ArenaVec<'a, Statement<'a>>) {
        let mut hit: Option<(usize, Vec<Statement<'a>>)> = None;
        for i in 0..stmts.len() {
            let Statement::ForStatement(f) = &stmts[i] else {
                continue;
            };
            // The loop must never exit via its own test: `for (…; true; )` or
            // `for (…; ; )`. A loop with a real test could fall through before the
            // chain ends — not a static linear walk.
            let test_is_always = match &f.test {
                None => true,
                Some(Expression::BooleanLiteral(b)) => b.value,
                _ => false,
            };
            if !test_is_always {
                continue;
            }
            // No `for`-update clause: the cursor is advanced solely by `i++` inside
            // the discriminant. An update would be a second, unmodeled mutation.
            if f.update.is_some() {
                continue;
            }
            let Some(init) = &f.init else { continue };
            let Some((arr_name, idx_name, keys)) = parse_init(init) else {
                continue;
            };
            // The order array must be provably immutable (never `push`/`[k]=`/etc.).
            if self.mutable.contains(&arr_name) {
                continue;
            }
            // Body: `switch (a[i++]) { … }`, optionally wrapped in a block and/or
            // trailed by a `break`. `has_break_exit` is true when the loop body has a
            // trailing `break` reachable when the switch matches nothing — i.e. the
            // loop cleanly exits once the cursor runs off the end of the array.
            let Some((switch, has_break_exit)) = body_switch(&f.body, &arr_name, &idx_name) else {
                continue;
            };
            if switch.cases.iter().any(|c| c.test.is_none()) {
                continue; // no `default` clause
            }
            let Some(cases) = parse_cases(switch, &arr_name, &idx_name) else {
                continue;
            };
            // `a` and `i` must be dispatch-only: referenced *nowhere* outside this
            // `for` statement, program-wide. Compare each name's total occurrence
            // count to its count inside the for-statement; equal ⇒ no escape. (The
            // census counts binding ids too, so the param/var declaration of `a`/`i`
            // outside the loop also counts as an escape and is correctly rejected —
            // unless, as in the CFF shape, the name's only binding is the for-init
            // assignment target, which lives inside the for-statement.)
            let for_stmt = &stmts[i];
            let arr_local = count_name_in_stmt(for_stmt, &arr_name);
            let idx_local = count_name_in_stmt(for_stmt, &idx_name);
            if self.global_counts.get(&arr_name).copied().unwrap_or(0) != arr_local
                || self.global_counts.get(&idx_name).copied().unwrap_or(0) != idx_local
            {
                continue;
            }
            let Some(seq) = simulate_sequence(&keys, &cases, has_break_exit, self.ast.allocator)
            else {
                continue;
            };
            hit = Some((i, seq));
            break;
        }

        let Some((i, seq)) = hit else {
            return;
        };
        // Splice: replace the `for` dispatcher (statement i) with the linear
        // sequence. Everything else stays in order.
        let old = std::mem::replace(stmts, self.ast.vec());
        let mut seq = Some(seq);
        for (k, s) in old.into_iter().enumerate() {
            if k == i {
                for st in seq.take().unwrap() {
                    stmts.push(st);
                }
                continue;
            }
            stmts.push(s);
        }
        self.changed = true;
    }

    /// Descend into every statement list reachable from `stmt`, so nested
    /// dispatchers (inside functions, `try` blocks, branches, …) are recovered.
    /// Mirrors `unflatten_cfg`'s recursion.
    fn recurse_stmt(&mut self, stmt: &mut Statement<'a>) {
        match stmt {
            Statement::BlockStatement(b) => self.lists(&mut b.body),
            Statement::FunctionDeclaration(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements);
                }
            }
            Statement::VariableDeclaration(vd) => {
                for d in &mut vd.declarations {
                    if let Some(init) = &mut d.init {
                        self.recurse_expr_fn(init);
                    }
                }
            }
            Statement::ExpressionStatement(es) => self.recurse_expr_fn(&mut es.expression),
            Statement::ReturnStatement(r) => {
                if let Some(arg) = &mut r.argument {
                    self.recurse_expr_fn(arg);
                }
            }
            Statement::IfStatement(i) => {
                self.recurse_stmt(&mut i.consequent);
                if let Some(alt) = &mut i.alternate {
                    self.recurse_stmt(alt);
                }
            }
            Statement::ForStatement(f) => self.recurse_stmt(&mut f.body),
            Statement::ForInStatement(f) => self.recurse_stmt(&mut f.body),
            Statement::ForOfStatement(f) => self.recurse_stmt(&mut f.body),
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
            _ => {}
        }
    }

    fn recurse_expr_fn(&mut self, expr: &mut Expression<'a>) {
        match expr {
            Expression::FunctionExpression(f) => {
                if let Some(body) = &mut f.body {
                    self.lists(&mut body.statements);
                }
            }
            Expression::ArrowFunctionExpression(f) => {
                self.lists(&mut f.body.statements);
            }
            Expression::CallExpression(c) => {
                self.recurse_expr_fn(&mut c.callee);
                for arg in &mut c.arguments {
                    if let Some(e) = arg.as_expression_mut() {
                        self.recurse_expr_fn(e);
                    }
                }
            }
            Expression::ParenthesizedExpression(p) => self.recurse_expr_fn(&mut p.expression),
            Expression::UnaryExpression(u) => self.recurse_expr_fn(&mut u.argument),
            Expression::SequenceExpression(s) => {
                for e in &mut s.expressions {
                    self.recurse_expr_fn(e);
                }
            }
            Expression::AssignmentExpression(a) => self.recurse_expr_fn(&mut a.right),
            _ => {}
        }
    }
}

/// A dispatch key drawn from the order array: a string or an integer.
#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    Str(String),
    Int(i64),
}

/// Parse the `for`-init into `(array-name, cursor-name, the constant key sequence)`.
///
/// Accepts both the param/assignment form
/// `a = "…".split("|"), i = 0`
/// and the declaration form
/// `var a = "…".split("|"), i = 0`.
/// Requires exactly two initializers in this order: the array (a provably constant
/// array expression) then the cursor (initialized to integer `0`).
fn parse_init(init: &ForStatementInit<'_>) -> Option<(String, String, Vec<Key>)> {
    // Collect the (target-name, init-expr) pairs in source order.
    let pairs: Vec<(String, &Expression<'_>)> = match init {
        ForStatementInit::VariableDeclaration(vd) => {
            let mut out = Vec::with_capacity(vd.declarations.len());
            for d in &vd.declarations {
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    return None;
                };
                out.push((id.name.to_string(), d.init.as_ref()?));
            }
            out
        }
        // An expression init: a single assignment or a comma sequence of them.
        _ => {
            let expr = init.as_expression()?;
            let mut exprs: Vec<&Expression<'_>> = Vec::new();
            match expr {
                Expression::SequenceExpression(seq) => {
                    for e in &seq.expressions {
                        exprs.push(e);
                    }
                }
                other => exprs.push(other),
            }
            let mut out = Vec::with_capacity(exprs.len());
            for e in exprs {
                let Expression::AssignmentExpression(a) = e else {
                    return None;
                };
                if a.operator != AssignmentOperator::Assign {
                    return None;
                }
                let AssignmentTarget::AssignmentTargetIdentifier(id) = &a.left else {
                    return None;
                };
                out.push((id.name.to_string(), &a.right));
            }
            out
        }
    };

    if pairs.len() != 2 {
        return None;
    }
    let (arr_name, arr_init) = &pairs[0];
    let (idx_name, idx_init) = &pairs[1];
    if arr_name == idx_name {
        return None;
    }
    // The cursor must start at 0.
    if int_literal(idx_init)? != 0 {
        return None;
    }
    let keys = const_key_array(arr_init)?;
    Some((arr_name.clone(), idx_name.clone(), keys))
}

/// Resolve a provably constant array of dispatch keys from the array initializer:
///   * `"a|b|c".split("|")` (any single-char/string separator) → split the literal;
///   * `["a", "b", 3]` → a literal array of string/number elements.
///
/// Anything else (a member read, a call we can't evaluate, a non-constant element)
/// → `None`, and the loop is declined.
fn const_key_array(expr: &Expression<'_>) -> Option<Vec<Key>> {
    match expr {
        Expression::CallExpression(call) => {
            // `<string literal>.split(<string literal>)`.
            let member = match &call.callee {
                Expression::StaticMemberExpression(m) if m.property.name == "split" => &m.object,
                _ => return None,
            };
            let Expression::StringLiteral(s) = member else {
                return None;
            };
            if call.arguments.len() != 1 {
                return None;
            }
            let arg = call.arguments[0].as_expression()?;
            let Expression::StringLiteral(sep) = arg else {
                return None;
            };
            // A genuine split. An empty separator splits per UTF-16 code unit; we
            // only model a non-empty separator (the `|`/`;` CFF form), declining the
            // empty case rather than risk a code-unit mismatch.
            if sep.value.is_empty() {
                return None;
            }
            Some(
                s.value
                    .split(sep.value.as_str())
                    .map(|p| Key::Str(p.to_string()))
                    .collect(),
            )
        }
        Expression::ArrayExpression(arr) => {
            let mut out = Vec::with_capacity(arr.elements.len());
            for el in &arr.elements {
                let e = el.as_expression()?; // no holes / spreads
                out.push(match e {
                    Expression::StringLiteral(s) => Key::Str(s.value.to_string()),
                    _ => Key::Int(int_literal(e)?),
                });
            }
            Some(out)
        }
        _ => None,
    }
}

/// `<int>` / `-<int>` numeric literal → its value.
fn int_literal(expr: &Expression<'_>) -> Option<i64> {
    match expr {
        Expression::NumericLiteral(n) if n.value.fract() == 0.0 && n.value.is_finite() => {
            Some(n.value as i64)
        }
        Expression::UnaryExpression(u)
            if matches!(
                u.operator,
                oxc_syntax::operator::UnaryOperator::UnaryNegation
            ) =>
        {
            int_literal(&u.argument).map(|v| -v)
        }
        _ => None,
    }
}

/// If the loop body is `switch (a[i++]) { … }` (optionally wrapped in a block
/// containing only the switch and an optional trailing `break`), return the switch.
fn body_switch<'b, 'a>(
    body: &'b Statement<'a>,
    arr: &str,
    idx: &str,
) -> Option<(&'b SwitchStatement<'a>, bool)> {
    let (switch, has_break_exit) = match body {
        Statement::SwitchStatement(sw) => (sw, false),
        Statement::BlockStatement(b) => {
            // The block may hold the switch plus an optional trailing `break`. That
            // `break` is the loop's exit once the switch matches nothing (the cursor
            // ran off the end of the array): control falls out of the switch to the
            // `break` and leaves the loop. We only accept the switch first, then at
            // most a single bare `break`; anything else is unmodeled.
            let mut sw_ref = None;
            let mut brk = false;
            for st in &b.body {
                match st {
                    Statement::SwitchStatement(s) if sw_ref.is_none() && !brk => sw_ref = Some(s),
                    Statement::BreakStatement(br)
                        if br.label.is_none() && sw_ref.is_some() && !brk =>
                    {
                        brk = true;
                    }
                    _ => return None,
                }
            }
            (sw_ref?, brk)
        }
        _ => return None,
    };
    if !is_cursor_read(&switch.discriminant, arr, idx) {
        return None;
    }
    Some((switch, has_break_exit))
}

/// Is `expr` exactly `a[i++]` (post-increment cursor read of the order array)?
fn is_cursor_read(expr: &Expression<'_>, arr: &str, idx: &str) -> bool {
    let Expression::ComputedMemberExpression(m) = expr else {
        return false;
    };
    let Expression::Identifier(obj) = &m.object else {
        return false;
    };
    if obj.name != arr {
        return false;
    }
    let Expression::UpdateExpression(u) = &m.expression else {
        return false;
    };
    if u.operator != UpdateOperator::Increment || u.prefix {
        return false;
    }
    matches!(&u.argument,
        oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(id) if id.name == idx)
}

/// A parsed case: the statements to emit and whether it advances or terminates.
struct ParsedCase<'b, 'a> {
    emit: Vec<&'b Statement<'a>>,
    terminal: bool,
}

/// Parse every case keyed by its dispatch key. Each case body must be dispatch-clean
/// (never reads `a`/`i`) and end in either `continue` (advance the cursor) or a
/// `return`/`throw` terminator. Returns `None` (decline) on any unmodeled shape.
fn parse_cases<'b, 'a>(
    switch: &'b SwitchStatement<'a>,
    arr: &str,
    idx: &str,
) -> Option<HashMap<Key, ParsedCase<'b, 'a>>> {
    let mut out = HashMap::new();
    for case in &switch.cases {
        let key = case_key(case.test.as_ref()?)?;

        let mut flat: Vec<&Statement> = Vec::new();
        flatten_case(&case.consequent, &mut flat);

        // The first top-level terminator decides the case end; anything after it is
        // dead.
        let term_idx = flat.iter().position(|st| is_top_level_control(st))?;
        let (terminal, upto) = match flat[term_idx] {
            Statement::ContinueStatement(c) if c.label.is_none() => (false, term_idx),
            Statement::BreakStatement(b) if b.label.is_none() => {
                // A bare `break` exits the loop entirely. Treat it as terminal: the
                // walk stops, control falls past the loop. Equivalent to running off
                // the chain, so it is sound to model as a hard stop.
                (true, term_idx)
            }
            Statement::ReturnStatement(_) | Statement::ThrowStatement(_) => (true, term_idx + 1),
            _ => return None, // labeled break/continue → unmodeled
        };

        let mut emit = Vec::new();
        for (k, &st) in flat[..upto].iter().enumerate() {
            let is_terminal_stmt = terminal && upto == term_idx + 1 && k == upto - 1;
            // The body must not touch the dispatch cursor.
            if mentions(st, arr) || mentions(st, idx) {
                return None;
            }
            if !is_terminal_stmt && is_top_level_control(st) {
                return None; // a second dispatch exit we don't model
            }
            emit.push(st);
        }
        if out.insert(key, ParsedCase { emit, terminal }).is_some() {
            return None; // duplicate case label
        }
    }
    Some(out)
}

/// A case label expression → its dispatch key (string or integer literal).
fn case_key(expr: &Expression<'_>) -> Option<Key> {
    match expr {
        Expression::StringLiteral(s) => Some(Key::Str(s.value.to_string())),
        _ => int_literal(expr).map(Key::Int),
    }
}

/// Walk the key sequence, emitting case bodies in array order. Stop when a case is
/// terminal or the cursor runs off the end of the array. A missing case for a key
/// means a real `default`-less fallthrough we don't model → decline.
fn simulate_sequence<'a>(
    keys: &[Key],
    cases: &HashMap<Key, ParsedCase<'_, 'a>>,
    has_break_exit: bool,
    alloc: &'a Allocator,
) -> Option<Vec<Statement<'a>>> {
    let mut seq: Vec<Statement<'a>> = Vec::new();
    for key in keys {
        let Some(case) = cases.get(key) else {
            // No matching case for this key. The switch matches nothing and control
            // falls through: to the trailing `break` (clean loop exit — stop here) if
            // present, otherwise the loop spins forever on `for(;;)`. Without a break
            // we can't model the (non-terminating) original → decline.
            if has_break_exit {
                return Some(seq);
            }
            return None;
        };
        for st in &case.emit {
            seq.push((*st).clone_in(alloc));
        }
        if case.terminal {
            return Some(seq);
        }
    }
    // Ran off the end of the array: the next `a[i++]` is `undefined`, the switch
    // matches nothing. With a trailing `break` the loop exits cleanly (stop). Without
    // one the loop spins forever — we cannot model that, so decline.
    if has_break_exit {
        Some(seq)
    } else {
        None
    }
}
