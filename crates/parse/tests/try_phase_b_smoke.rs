//! Smoke tests for Try Phase B's three lowering shapes:
//! - try+catch+finally  → Item::Function (not PassThroughFunction)
//! - try+finally        → Item::Function
//! - try+catch          → Item::Function (Phase A regression check)
//!
//! Also confirms structural round-trip preserves the shape.

use ir::tree::{Item, Stmt};
use parse::parse_and_lower;

fn lowered(src: &str) -> bool {
    let wrapped = format!("function f() {{\n{src}\n}}");
    let m = parse_and_lower(&wrapped).expect("parse");
    matches!(m.items.first(), Some(Item::Function(_)))
}

fn try_shape_in_first_function(
    src: &str,
) -> Option<(usize, bool, bool)> {
    let wrapped = format!("function f() {{\n{src}\n}}");
    let m = parse_and_lower(&wrapped).ok()?;
    let f = match m.items.first()? {
        Item::Function(f) => f,
        _ => return None,
    };
    fn first_try(stmts: &[Stmt]) -> Option<&Stmt> {
        for s in stmts {
            if matches!(s, Stmt::Try { .. }) {
                return Some(s);
            }
        }
        None
    }
    let s = first_try(&f.body)?;
    if let Stmt::Try { body, catch, finally } = s {
        Some((body.len(), catch.is_some(), finally.is_some()))
    } else {
        None
    }
}

#[test]
fn try_catch_no_finally_lowers() {
    assert!(lowered("try { x = 1; } catch (e) { y = 2; }"));
    let shape = try_shape_in_first_function("try { x = 1; } catch (e) { y = 2; }").unwrap();
    assert_eq!(shape.1, true, "catch should round-trip"); // catch present
    assert_eq!(shape.2, false, "no finally"); // finally absent
}

#[test]
fn try_catch_finally_lowers() {
    assert!(lowered("try { x = 1; } catch (e) { y = 2; } finally { z = 3; }"));
    let shape = try_shape_in_first_function(
        "try { x = 1; } catch (e) { y = 2; } finally { z = 3; }",
    )
    .unwrap();
    assert_eq!(shape.1, true, "catch should round-trip");
    assert_eq!(shape.2, true, "finally should round-trip");
}

#[test]
fn try_finally_no_catch_lowers() {
    assert!(lowered("try { x = 1; } finally { z = 3; }"));
    let shape = try_shape_in_first_function("try { x = 1; } finally { z = 3; }").unwrap();
    assert_eq!(shape.1, false, "no catch");
    assert_eq!(shape.2, true, "finally should round-trip");
}

#[test]
fn nested_try_catch_finally_lowers() {
    assert!(lowered(
        "try { try { x = 1; } finally { y = 2; } } catch (e) { z = 3; }"
    ));
}

#[test]
fn try_with_return_in_body_lowers() {
    // The implicit-throw-and-abrupt-completion approximation: SSA
    // emits Terminator::Return inside try-body; JS runtime handles
    // the "run finally before returning" semantics on emit.
    assert!(lowered("try { return 1; } finally { z = 3; }"));
}

#[test]
fn try_with_throw_in_catch_lowers() {
    // Catch's Terminator::Throw exits the function; JS runtime runs
    // finally before propagating.
    assert!(lowered(
        "try { x = 1; } catch (e) { throw e; } finally { z = 3; }"
    ));
}
