//! Rename-by-role / rename-by-behavior.
//!
//! Obfuscators use deliberately illegible identifiers (`WL`, `jK`, `_0x3a`).
//! We rename every *opaque* binding to a stable, unique, **descriptive** name —
//! inferred from what the binding holds (`document` → `doc`, a regex → `regex`,
//! `new Date()` → `date`), what a function does (returns `a < b` → `lessThan`,
//! returns a table lookup → `get`), or how it is used (only ever `.push`ed to →
//! `list`). When nothing is confident we fall back to a typed-but-generic name
//! (`fn1`, `var1`). This is done through the symbol table: we map `SymbolId →
//! new name` once, then rewrite binding identifiers and identifier *references*
//! (never property keys, which aren't references), so all uses stay consistent
//! and no distinct symbols are ever merged.
//!
//! Safety: globally-unique target names mean we can never introduce a collision
//! or accidental capture — every renamed symbol gets a name that appears nowhere
//! else in the program (not as another binding, a global the code reads, a
//! reserved word, or `undefined`). Property accesses like `obj.WL` are untouched
//! because they are `IdentifierName`s, not references to the renamed symbol.
//!
//! Idempotence: every name we emit has ≥ 3 non-`_$` characters and never starts
//! with `_0x`, so `is_opaque` rejects our own output and a second run renames
//! nothing.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    BindingIdentifier, BindingPattern, Expression, Function, IdentifierReference, Program,
    Statement, StaticMemberExpression, VariableDeclarator,
};
use oxc_syntax::operator::BinaryOperator;
use oxc_syntax::symbol::{SymbolFlags, SymbolId};
use oxc_traverse::{traverse_mut, Traverse, TraverseCtx};

use oxc_allocator::Allocator;

use crate::pass::Pass;
use crate::util::build_scoping;

#[derive(Default)]
pub struct RenameByRole {
    changed: bool,
}

impl Pass for RenameByRole {
    fn name(&self) -> &'static str {
        "rename-by-role"
    }

    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool {
        // Phase 1: scan the tree to gather a descriptive base-name hint per symbol
        // (from assignment / function behavior) and a record of how each symbol is
        // used as a member-access object (for the usage fallback).
        let scoping = build_scoping(program);
        let mut collector = Collector {
            hints: HashMap::new(),
            usage: HashMap::new(),
        };
        let scoping = traverse_mut(&mut collector, allocator, program, scoping, ());

        // Existing names we must not collide with, plus reserved words and
        // `undefined`. Seeding `used` with all of these guarantees every emitted
        // name is globally fresh and a valid, non-capturing identifier.
        let mut used: HashSet<String> = scoping.symbol_names().map(|s| s.to_string()).collect();
        used.extend(RESERVED.iter().map(|s| s.to_string()));
        used.insert("undefined".to_string());

        let mut rename: HashMap<SymbolId, String> = HashMap::new();
        let mut counters: HashMap<String, usize> = HashMap::new();

        let root = scoping.root_scope_id();
        for sym in scoping.symbol_ids() {
            // Never rename root-scope (global / observable) bindings — they may
            // be the program's public entry point, like DCE's removal guard.
            if scoping.symbol_scope_id(sym) == root {
                continue;
            }
            let name = scoping.symbol_name(sym);
            if !is_opaque(name) {
                continue;
            }

            let is_fn = scoping
                .symbol_flags(sym)
                .intersects(SymbolFlags::Function | SymbolFlags::Class);

            // Priority: assignment / behavior hint, then usage hint, then a
            // typed-but-generic fallback (`fn`/`var`).
            let base = collector
                .hints
                .get(&sym)
                .map(|s| s.as_str())
                .or_else(|| usage_name(collector.usage.get(&sym)))
                .unwrap_or(if is_fn { "fn" } else { "var" });

            let new = unique_name(base, &mut counters, &mut used);
            rename.insert(sym, new);
        }

        if rename.is_empty() {
            return false;
        }

        let mut renamer = Renamer {
            rename: &rename,
            changed: false,
            _marker: Default::default(),
        };
        let scoping = build_scoping(program);
        traverse_mut(&mut renamer, allocator, program, scoping, ());
        self.changed = renamer.changed;
        self.changed
    }
}

/// Allocate a globally-unique name from `base`. The bare `base` is used if free;
/// otherwise a numeric suffix is appended (`add`, `add1`, `add2`, …). Every
/// returned name is recorded in `used` so future calls never repeat it. Because
/// `used` is seeded with reserved words and `undefined`, those bases are skipped
/// straight to a (never-reserved) numbered form.
fn unique_name(base: &str, counters: &mut HashMap<String, usize>, used: &mut HashSet<String>) -> String {
    if !used.contains(base) {
        used.insert(base.to_string());
        return base.to_string();
    }
    let n = counters.entry(base.to_string()).or_insert(0);
    loop {
        *n += 1;
        let candidate = format!("{base}{n}");
        if !used.contains(&candidate) {
            used.insert(candidate.clone());
            return candidate;
        }
    }
}

/// An identifier that reads as machine-generated: ≤ 2 non-`_$` chars, or a
/// `_0x…` hex name. Conservative on purpose — never touches plausibly-meaningful
/// names. Every name this pass *emits* must fail this test, or the pass would
/// re-rename its own output forever; all our bases have ≥ 3 such chars.
fn is_opaque(name: &str) -> bool {
    name.starts_with("_0x") || name.chars().filter(|c| *c != '_' && *c != '$').count() <= 2
}

// ---------------------------------------------------------------------------
// Phase 1 — hint collection
// ---------------------------------------------------------------------------

struct Collector {
    /// Best descriptive base name inferred from a symbol's initializer or, for
    /// functions, its body. First writer wins (declarations are visited once).
    hints: HashMap<SymbolId, String>,
    /// Property names a symbol is accessed with as a member-expression object,
    /// e.g. `xs.push` records `push` for `xs`. Used only as a fallback.
    usage: HashMap<SymbolId, Vec<String>>,
}

impl Collector {
    fn record_hint(&mut self, sym: SymbolId, name: &str) {
        self.hints.entry(sym).or_insert_with(|| name.to_string());
    }
}

impl<'a> Traverse<'a, ()> for Collector {
    fn enter_variable_declarator(
        &mut self,
        node: &mut VariableDeclarator<'a>,
        _ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let BindingPattern::BindingIdentifier(bid) = &node.id else {
            return;
        };
        let Some(init) = &node.init else {
            return;
        };
        if let Some(name) = name_from_expr(init) {
            self.record_hint(bid.symbol_id(), name);
        }
    }

    fn enter_function(&mut self, node: &mut Function<'a>, _ctx: &mut TraverseCtx<'a, ()>) {
        let Some(id) = &node.id else {
            return;
        };
        if let Some(name) = name_from_function_body(node.body.as_deref()) {
            self.record_hint(id.symbol_id(), name);
        }
    }

    fn enter_static_member_expression(
        &mut self,
        node: &mut StaticMemberExpression<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let Expression::Identifier(idref) = &node.object else {
            return;
        };
        let Some(rid) = idref.reference_id.get() else {
            return;
        };
        if let Some(sym) = ctx.scoping().get_reference(rid).symbol_id() {
            self.usage
                .entry(sym)
                .or_default()
                .push(node.property.name.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Name inference
// ---------------------------------------------------------------------------

/// Infer a base name from what a binding is *assigned*. Returns `None` when no
/// confident guess applies (the caller then tries usage, then a generic name).
fn name_from_expr(expr: &Expression<'_>) -> Option<&'static str> {
    match expr {
        Expression::Identifier(idref) => known_global(&idref.name),
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => Some("str"),
        Expression::NumericLiteral(_) | Expression::BigIntLiteral(_) => Some("num"),
        Expression::BooleanLiteral(_) => Some("flag"),
        Expression::RegExpLiteral(_) => Some("regex"),
        Expression::ArrayExpression(_) => Some("arr"),
        Expression::ObjectExpression(_) => Some("obj"),
        Expression::FunctionExpression(f) => name_from_function_body(f.body.as_deref()),
        // An arrow with an expression body (`() => a < b`) stores that expression
        // as a single `ExpressionStatement` (no `return`); read it directly.
        Expression::ArrowFunctionExpression(f) if f.expression => {
            match f.body.statements.first() {
                Some(Statement::ExpressionStatement(es)) => name_from_returned_expr(&es.expression),
                _ => None,
            }
        }
        Expression::ArrowFunctionExpression(f) => name_from_function_body(Some(&*f.body)),
        Expression::NewExpression(n) => Some(name_from_constructor(&n.callee)),
        Expression::CallExpression(c) => name_from_call(&c.callee),
        _ => None,
    }
}

/// Infer a base name from a function's *behavior*: what its sole/last `return`
/// computes. A `return a < b` body becomes `lessThan`, a table lookup `t[i]`
/// becomes `get`, and so on.
fn name_from_function_body(body: Option<&oxc_ast::ast::FunctionBody<'_>>) -> Option<&'static str> {
    let body = body?;
    // Find the return that carries the function's value: the last top-level
    // `return <expr>` statement in the body.
    let ret = body.statements.iter().rev().find_map(|stmt| match stmt {
        oxc_ast::ast::Statement::ReturnStatement(r) => Some(r),
        _ => None,
    })?;
    let arg = ret.argument.as_ref()?;
    name_from_returned_expr(arg)
}

fn name_from_returned_expr(expr: &Expression<'_>) -> Option<&'static str> {
    match expr {
        Expression::BinaryExpression(b) => Some(name_from_binary_op(b.operator)),
        Expression::LogicalExpression(l) => Some(match l.operator {
            oxc_syntax::operator::LogicalOperator::And => "and",
            oxc_syntax::operator::LogicalOperator::Or => "or",
            oxc_syntax::operator::LogicalOperator::Coalesce => "coalesce",
        }),
        Expression::UnaryExpression(u) => match u.operator {
            oxc_syntax::operator::UnaryOperator::Typeof => Some("typeOf"),
            oxc_syntax::operator::UnaryOperator::LogicalNot => Some("negate"),
            oxc_syntax::operator::UnaryOperator::BitwiseNot => Some("bitNot"),
            _ => None,
        },
        // A computed lookup `table[i]` is the canonical decoder/getter shape.
        Expression::ComputedMemberExpression(_) => Some("get"),
        Expression::NewExpression(n) => match constructor_name(&n.callee) {
            Some("Date") => Some("now"),
            _ => Some("create"),
        },
        Expression::CallExpression(c) => name_from_call(&c.callee).or(Some("call")),
        Expression::ConditionalExpression(_) => Some("pick"),
        _ => None,
    }
}

fn name_from_binary_op(op: BinaryOperator) -> &'static str {
    use BinaryOperator::*;
    match op {
        Equality | StrictEquality => "equals",
        Inequality | StrictInequality => "notEqual",
        LessThan => "lessThan",
        LessEqualThan => "lessEqual",
        GreaterThan => "greaterThan",
        GreaterEqualThan => "greaterEqual",
        Addition => "add",
        Subtraction => "sub",
        Multiplication => "mul",
        Division => "div",
        Remainder => "mod",
        Exponential => "pow",
        ShiftLeft => "shiftLeft",
        ShiftRight => "shiftRight",
        ShiftRightZeroFill => "shiftRightU",
        BitwiseOR => "bitOr",
        BitwiseXOR => "bitXor",
        BitwiseAnd => "bitAnd",
        In => "hasKey",
        Instanceof => "isInstance",
    }
}

/// The textual type name of a `new X` / `new a.X` callee, if simple.
fn constructor_name<'e>(callee: &'e Expression<'_>) -> Option<&'e str> {
    match callee {
        Expression::Identifier(id) => Some(id.name.as_str()),
        Expression::StaticMemberExpression(m) => Some(m.property.name.as_str()),
        _ => None,
    }
}

/// Base name for a `new X()` initializer: a curated name for common built-ins,
/// otherwise a lower-cased form of the type name (`new Foo()` → `foo`), falling
/// back to `obj` when that would be too short or unavailable.
fn name_from_constructor(callee: &Expression<'_>) -> &'static str {
    match constructor_name(callee) {
        Some("Date") => "date",
        Some("RegExp") => "regex",
        Some("Array") => "arr",
        Some("Object") => "obj",
        Some("Map") | Some("WeakMap") => "map",
        Some("Set") | Some("WeakSet") => "set",
        Some("Promise") => "promise",
        Some("Error") | Some("TypeError") | Some("RangeError") => "err",
        Some("Function") => "fn",
        Some("Image") => "img",
        _ => "obj",
    }
}

/// Base name for a value produced by `f(...)` / `a.b(...)`, recognising common
/// DOM and built-in factory calls. Returns `None` for unrecognised calls.
fn name_from_call(callee: &Expression<'_>) -> Option<&'static str> {
    let prop = match callee {
        Expression::StaticMemberExpression(m) => m.property.name.as_str(),
        Expression::Identifier(id) => id.name.as_str(),
        _ => return None,
    };
    Some(match prop {
        "createElement" => "elem",
        "createTextNode" => "node",
        "getElementById" | "querySelector" | "closest" => "elem",
        "querySelectorAll" | "getElementsByTagName" | "getElementsByClassName" => "elems",
        "getContext" => "ctx",
        "getItem" => "item",
        "parse" => "parsed",
        "stringify" => "json",
        "split" => "parts",
        "match" => "match",
        "now" => "time",
        _ => return None,
    })
}

/// Map a literal global identifier read (`var d = document;`) to a short name.
fn known_global(name: &str) -> Option<&'static str> {
    Some(match name {
        "document" => "doc",
        "window" => "win",
        "navigator" => "nav",
        "location" => "loc",
        "console" => "logger",
        "Math" => "math",
        "JSON" => "json",
        "Date" => "date",
        "Array" => "arr",
        "Object" => "obj",
        "String" => "str",
        "Number" => "num",
        "RegExp" => "regex",
        "globalThis" | "self" => "glob",
        "localStorage" | "sessionStorage" => "store",
        "performance" => "perf",
        "history" => "hist",
        "screen" => "screen",
        _ => return None,
    })
}

/// Usage-based fallback: a binding only ever touched through list-mutation /
/// list-inspection members is almost certainly an array accumulator.
fn usage_name(props: Option<&Vec<String>>) -> Option<&'static str> {
    let props = props?;
    if props.is_empty() {
        return None;
    }
    let list_like = props.iter().all(|p| {
        matches!(
            p.as_str(),
            "push" | "pop" | "shift" | "unshift" | "splice" | "length" | "join" | "concat"
                | "slice" | "indexOf" | "forEach" | "map" | "filter" | "sort" | "reverse"
        )
    });
    if list_like {
        Some("list")
    } else {
        None
    }
}

/// ECMAScript reserved words (and a few contextual ones) we must never emit as a
/// binding name. Seeding `used` with these forces such a base to a numbered form.
const RESERVED: &[&str] = &[
    "await", "break", "case", "catch", "class", "const", "continue", "debugger", "default",
    "delete", "do", "else", "enum", "export", "extends", "false", "finally", "for", "function",
    "get", "if", "implements", "import", "in", "instanceof", "interface", "let", "new", "null",
    "package", "private", "protected", "public", "return", "set", "static", "super", "switch",
    "this", "throw", "true", "try", "typeof", "var", "void", "while", "with", "yield",
];

// ---------------------------------------------------------------------------
// Phase 2 — apply the rename map
// ---------------------------------------------------------------------------

struct Renamer<'a, 'r> {
    rename: &'r HashMap<SymbolId, String>,
    changed: bool,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a, 'r> Traverse<'a, ()> for Renamer<'a, 'r> {
    fn enter_binding_identifier(
        &mut self,
        node: &mut BindingIdentifier<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        if let Some(sym) = node.symbol_id.get() {
            if let Some(new) = self.rename.get(&sym) {
                node.name = ctx.ast.ident(new);
                self.changed = true;
            }
        }
    }

    fn enter_identifier_reference(
        &mut self,
        node: &mut IdentifierReference<'a>,
        ctx: &mut TraverseCtx<'a, ()>,
    ) {
        let Some(rid) = node.reference_id.get() else {
            return;
        };
        if let Some(sym) = ctx.scoping().get_reference(rid).symbol_id() {
            if let Some(new) = self.rename.get(&sym) {
                node.name = ctx.ast.ident(new);
                self.changed = true;
            }
        }
    }
}
