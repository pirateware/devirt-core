//! Tests for parse-time desugaring of non-simple function parameters
//! (issues #29 and #45). DefaultParam, DestructuringParam, and
//! TryCatchDestructuringParam all lower into simple-identifier params
//! plus a body prologue. RestParam (issue #45) lowers via
//! `InstKind::RestArgs` at function entry; nested array/object rest in
//! destructuring lowers via parse-time desugaring (slice / spread+delete).

use emit::emit;
use ir::tree::{Item, Stmt};
use parse::{lower_ssa, lower_tree, parse_and_lower};

fn first_fn(src: &str) -> ir::tree::IrFunction {
    let m = parse_and_lower(src).expect("parse_and_lower");
    for item in m.items {
        if let Item::Function(f) = item {
            return f;
        }
    }
    panic!("no top-level Item::Function (likely PassThroughFunction): src={src}");
}

fn assert_top_is_lowered(src: &str) {
    let m = parse_and_lower(src).expect("parse_and_lower");
    let mut saw_lowered = false;
    for item in &m.items {
        match item {
            Item::Function(_) => saw_lowered = true,
            Item::PassThroughFunction(lit) => panic!(
                "expected Lowered, got PassThroughFunction (name={:?}, blockers={:?}) for: {src}",
                lit.name, lit.blockers
            ),
            _ => {}
        }
    }
    assert!(saw_lowered, "no top-level function found in: {src}");
}

fn ssa_roundtrip_emit(src: &str) -> String {
    let tm = parse_and_lower(src).expect("parse");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    let tm2 = lower_tree::lower_module(&sm);
    let out = emit(&tm2);
    parse_and_lower(&out).expect("re-parse");
    out
}

// --------------------------------------------------------------------
// DefaultParam
// --------------------------------------------------------------------

#[test]
fn default_param_lowers_to_function() {
    assert_top_is_lowered("function f(x = 1) { return x; }");
}

#[test]
fn default_param_emits_undefined_check_prologue() {
    let f = first_fn("function f(x = 1) { return x; }");
    assert_eq!(f.params, vec!["x"]);
    // First stmt must be the synthesized `if (x === undefined) x = 1;`.
    match &f.body[0] {
        Stmt::If { cond, then_body, else_body } => {
            assert!(else_body.is_none());
            assert_eq!(then_body.len(), 1);
            // cond shape: x === undefined
            match cond {
                ir::tree::Expr::Binary { op, .. } => {
                    assert_eq!(*op, ir::tree::BinOp::StrictEq);
                }
                other => panic!("expected Binary, got {other:?}"),
            }
        }
        other => panic!("expected Stmt::If prologue, got {other:?}"),
    }
}

#[test]
fn default_param_referencing_earlier_param() {
    // `b = a + 1` references the earlier-bound `a`. Evaluation order:
    // the prologue for `b` runs after `a` is bound, so this works.
    let src = "function f(a, b = a + 1) { return a + b; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn default_param_with_side_effecting_default() {
    let src = "function f(x = g()) { return x; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn default_param_in_arrow() {
    let src = "var f = (x = 1) => x;";
    // Arrow becomes `Expr::Func` inside an expression; the top-level
    // statement isn't a function but an assignment. We just check it
    // round-trips through the SSA pipeline cleanly.
    ssa_roundtrip_emit(src);
}

#[test]
fn default_param_in_arrow_expression_body() {
    // Expression-body arrow with a default param must convert to a
    // Stmts body so the prologue has somewhere to live.
    let src = "var f = (x = 5) => x + 1;";
    ssa_roundtrip_emit(src);
}

// --------------------------------------------------------------------
// DestructuringParam (object)
// --------------------------------------------------------------------

#[test]
fn object_destructuring_param_lowers() {
    assert_top_is_lowered("function f({a, b}) { return a + b; }");
}

#[test]
fn object_destructuring_synthesizes_p0_param_and_let_bindings() {
    let f = first_fn("function f({a, b}) { return a + b; }");
    assert_eq!(f.params, vec!["__p0"]);
    // Body should start with let-bindings for a and b.
    let names: Vec<String> = f.body.iter().filter_map(|s| match s {
        Stmt::Let { name, .. } => Some(name.clone()),
        _ => None,
    }).collect();
    assert!(names.contains(&"a".to_string()), "missing `a` let-binding in {names:?}");
    assert!(names.contains(&"b".to_string()), "missing `b` let-binding in {names:?}");
}

#[test]
fn nested_object_destructuring_param() {
    let src = "function f({a: {b}}) { return b; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn destructuring_with_inner_default() {
    // `function f({a = 1})` — destructure with per-property default.
    let src = "function f({a = 1}) { return a; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn destructuring_with_outer_default() {
    // `function f({a, b} = {})` — the param itself defaults to `{}`.
    let src = "function f({a, b} = {}) { return a; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

// --------------------------------------------------------------------
// DestructuringParam (array)
// --------------------------------------------------------------------

#[test]
fn array_destructuring_param_lowers() {
    assert_top_is_lowered("function f([a, b]) { return a + b; }");
}

#[test]
fn array_destructuring_with_elision() {
    let src = "function f([a, , c]) { return a + c; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn array_destructuring_uses_index_loads() {
    let f = first_fn("function f([a, b]) { return a + b; }");
    assert_eq!(f.params, vec!["__p0"]);
    // Find the let-binding for `a`; its init must be Index(__p0, 0).
    let init_for_a = f.body.iter().find_map(|s| match s {
        Stmt::Let { name, init: Some(e) } if name == "a" => Some(e.clone()),
        _ => None,
    }).expect("missing let a");
    match init_for_a {
        ir::tree::Expr::Index { obj, index } => {
            assert!(matches!(*obj, ir::tree::Expr::Ident(ref n) if n == "__p0"));
            assert!(matches!(*index, ir::tree::Expr::Number(n) if n == 0.0));
        }
        other => panic!("expected Index, got {other:?}"),
    }
}

// --------------------------------------------------------------------
// TryCatchDestructuringParam
// --------------------------------------------------------------------

#[test]
fn catch_destructuring_lowers() {
    // The function itself has no param blockers; the try/catch is
    // inside the body. The disposition was previously rejecting this
    // function.
    assert_top_is_lowered(
        "function f() { try { g(); } catch ({message}) { return message; } }",
    );
}

#[test]
fn catch_destructuring_round_trip() {
    let src = "function f() { try { g(); } catch ({message}) { return message; } }";
    ssa_roundtrip_emit(src);
}

// --------------------------------------------------------------------
// RestParam now lowers (issue #45) via InstKind::RestArgs at entry.
// --------------------------------------------------------------------

#[test]
fn rest_param_lowers_through_ssa() {
    let src = "function f(...args) { return args.length; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn rest_param_after_positional_lowers() {
    let src = "function f(a, b, ...rest) { return rest[0]; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn rest_param_emits_dotdotdot_in_header() {
    let src = "function f(a, ...rest) { return rest; }";
    let out = ssa_roundtrip_emit(src);
    assert!(out.contains("...rest"), "missing rest token in emit:\n{out}");
}

#[test]
fn array_rest_in_destructuring_lowers() {
    let src = "function f([a, b, ...tail]) { return tail; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

#[test]
fn object_rest_in_destructuring_lowers() {
    let src = "function f({ a, b, ...rest }) { return rest; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}

// --------------------------------------------------------------------
// Combined: multiple non-simple params + default referencing earlier
// --------------------------------------------------------------------

#[test]
fn mixed_default_and_destructuring_round_trip() {
    let src = "function f(x = 1, {a, b}, [p, q] = [0, 0]) { return x + a + b + p + q; }";
    assert_top_is_lowered(src);
    ssa_roundtrip_emit(src);
}
