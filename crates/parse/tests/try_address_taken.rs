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
fn try_catch_finally_registers_try_body_writes() {
    // Phase B: try+catch+finally lowers through real SSA. Try-body
    // writes are address-taken (catch and finally can read them).
    let names = collect("var x = 0; try { x = 1; } catch (e) { } finally { }");
    assert!(names.contains("x"), "got {names:?}");
}

#[test]
fn try_catch_finally_registers_catch_body_writes() {
    // Phase B: with finally present, catch-body writes are
    // address-taken too (finally can read them).
    let names = collect("var x = 0; try { } catch (e) { x = 1; } finally { }");
    assert!(names.contains("x"), "got {names:?}");
}

#[test]
fn try_finally_no_catch_registers_try_body_writes() {
    // Phase B: try+finally without catch is now valid.
    let names = collect("var x = 0; try { x = 1; } finally { }");
    assert!(names.contains("x"), "got {names:?}");
}

#[test]
fn try_catch_no_finally_does_not_register_catch_writes() {
    // Phase A invariant preserved: without finally, catch-body writes
    // don't register (they're in catch's own scope, no consumer).
    let names = collect("var x = 0; try { } catch (e) { x = 1; }");
    assert!(!names.contains("x"), "got {names:?}");
}

#[test]
fn try_finally_does_not_register_finally_body_writes() {
    // Finally-body's own writes don't register — no consumer reads
    // finally's mutations from outside.
    let names = collect("var x = 0; try { } catch (e) { } finally { x = 1; }");
    assert!(!names.contains("x"), "got {names:?}");
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
