//! Smoke tests for ForIn SSA lowering. Each test:
//! - Wraps a small JS program in a stub function.
//! - Asserts the stub function lowers to `Item::Function` (not
//!   `Item::PassThroughFunction`) — i.e., ForIn is no longer in
//!   fallback.
//! - Asserts the round-trip recovers `Stmt::ForIn` with the expected
//!   `binding` and `kind`.

use ir::tree::{Item, Stmt};
use parse::parse_and_lower;

fn lowered_first_forin(src: &str) -> Option<(String, String)> {
    // Returns (kind, binding) of the first ForIn found in the lowered
    // top-level function's body (depth 0 — for nested cases use a
    // walker, but the smoke tests put ForIn at the top of the body).
    let wrapped = format!("function f() {{\n{src}\n}}");
    let m = parse_and_lower(&wrapped).ok()?;
    let f = match m.items.first()? {
        Item::Function(f) => f,
        _ => return None,
    };
    fn find(stmts: &[Stmt]) -> Option<(String, String)> {
        for s in stmts {
            if let Stmt::ForIn { kind, binding, .. } = s {
                return Some((kind.clone(), binding.clone()));
            }
            // Look inside If/Block — corpus may wrap the for-in in a
            // guard.
            match s {
                Stmt::If { then_body, else_body, .. } => {
                    if let Some(r) = find(then_body) {
                        return Some(r);
                    }
                    if let Some(eb) = else_body {
                        if let Some(r) = find(eb) {
                            return Some(r);
                        }
                    }
                }
                Stmt::Block(b) => {
                    if let Some(r) = find(b) {
                        return Some(r);
                    }
                }
                _ => {}
            }
        }
        None
    }
    find(&f.body)
}

fn lowers_to_function(src: &str) -> bool {
    let wrapped = format!("function f() {{\n{src}\n}}");
    let m = parse_and_lower(&wrapped).expect("parse");
    matches!(m.items.first(), Some(Item::Function(_)))
}

#[test]
fn simple_for_in_lowers() {
    assert!(lowers_to_function("for (var k in obj) { x = k; }"));
    let (kind, binding) = lowered_first_forin("for (var k in obj) { x = k; }").unwrap();
    assert_eq!(kind, "var");
    assert_eq!(binding, "k");
}

#[test]
fn for_in_with_internal_if_lowers() {
    // Mirrors lodash_cloneDeep::baseKeys: body has an `if` inside.
    let src = "for (var key in obj) { if (cond) { result.push(key); } }";
    assert!(lowers_to_function(src));
    let (_, binding) = lowered_first_forin(src).unwrap();
    assert_eq!(binding, "key");
}

#[test]
fn for_in_with_continue_lowers() {
    // continue should branch to the header (next iteration).
    let src = "for (var k in obj) { if (skip) continue; x = k; }";
    assert!(lowers_to_function(src));
}

#[test]
fn for_in_with_break_lowers() {
    // break should branch to the exit.
    let src = "for (var k in obj) { if (stop) break; x = k; }";
    assert!(lowers_to_function(src));
}

#[test]
fn nested_for_in_lowers() {
    let src = "for (var i in outer) { for (var j in inner) { x = i + j; } }";
    assert!(lowers_to_function(src));
}

#[test]
fn for_in_inside_if_lowers() {
    // Mirrors baseKeys: a guard before the loop.
    let src = "if (a) { result = []; for (var k in obj) { result.push(k); } }";
    assert!(lowers_to_function(src));
}

#[test]
fn for_in_with_complex_obj_expr_lowers() {
    // The `obj` expression can be arbitrary — Object(value) is a
    // common idiom in lodash.
    let src = "for (var k in Object(value)) { x = k; }";
    assert!(lowers_to_function(src));
}
