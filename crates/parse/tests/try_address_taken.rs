//! Unit tests for `collect_try_address_taken_names` — the helper that
//! identifies symbols which need memory-resident treatment across the
//! try/catch boundary under structural framing (Option 3).
//!
//! This is Phase A; tests cover try-with-catch, no-finally only.

use parse::lower_ssa::collect_try_address_taken_names;
use parse::parse_and_lower;

fn collect(src: &str) -> std::collections::HashSet<String> {
    // Wrap in a function so the parser sees a function body
    // (PassThroughStmt path doesn't expose the tree-IR statements).
    let wrapped = format!("function _stub() {{\n{src}\n}}");
    let m = parse_and_lower(&wrapped).expect("parse");
    let f = match &m.items[0] {
        ir::tree::Item::Function(f) => f,
        _ => panic!("expected top-level function"),
    };
    collect_try_address_taken_names(&f.body)
}

#[test]
fn empty_try_has_no_address_taken() {
    let names = collect("try { } catch (e) { }");
    assert!(names.is_empty(), "got {names:?}");
}

#[test]
fn assign_in_try_body_is_address_taken() {
    let names = collect("var x = 0; try { x = 1; } catch (e) { }");
    assert!(names.contains("x"), "got {names:?}");
}

#[test]
fn compound_assign_in_try_body_is_address_taken() {
    let names = collect("var x = 0; try { x += 1; } catch (e) { }");
    assert!(names.contains("x"), "got {names:?}");
}

#[test]
fn update_in_try_body_is_address_taken() {
    let names = collect("var x = 0; try { x++; } catch (e) { }");
    assert!(names.contains("x"), "got {names:?}");
}

#[test]
fn assign_outside_try_body_is_not_address_taken() {
    let names = collect("var x = 0; x = 1; try { } catch (e) { }");
    assert!(!names.contains("x"), "got {names:?}");
}

#[test]
fn assign_in_catch_body_is_not_address_taken() {
    // Assignments INSIDE the catch body don't need address-taken treatment
    // for THIS try — they're written in the catch's own scope.
    let names = collect("var x = 0; try { } catch (e) { x = 1; }");
    assert!(!names.contains("x"), "got {names:?}");
}

#[test]
fn try_with_finally_does_not_register() {
    // Phase A: try-with-finally stays in fallback; we don't register
    // address-taken names for it. Phase B will revisit.
    let names = collect("var x = 0; try { x = 1; } catch (e) { } finally { }");
    assert!(names.is_empty(), "got {names:?}");
}

#[test]
fn nested_try_inside_try_body() {
    let names = collect(
        "var x = 0; var y = 0; try { x = 1; try { y = 2; } catch (e) { } } catch (e) { }",
    );
    assert!(names.contains("x"), "outer try-body assignment to x missing: {names:?}");
    assert!(names.contains("y"), "inner try-body assignment to y missing: {names:?}");
}

#[test]
fn assign_inside_function_literal_in_try_body_is_not_taken() {
    // Nested function literals have their own scope.
    let names = collect(
        "var x = 0; try { var f = function() { x = 1; }; } catch (e) { }",
    );
    assert!(
        !names.contains("x"),
        "x is inside a nested function literal; should not register: {names:?}"
    );
}
