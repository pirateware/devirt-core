//! Tests for issue #30: modern operator disposition lifts.
//! - `??` (nullish coalescing)
//! - `&&=`, `||=`, `??=` (logical assignment)
//! - `?.` `?.()` `?.[]` (optional chaining)
//! - `delete obj.k` and `delete obj[k]`

use emit::emit;
use ir::tree::Item;
use parse::{lower_ssa, lower_tree, parse_and_lower};

fn assert_lowered(src: &str) {
    let m = parse_and_lower(src).expect("parse_and_lower");
    for item in &m.items {
        if let Item::PassThroughFunction(lit) = item {
            panic!(
                "expected Lowered, got PassThroughFunction (name={:?}, blockers={:?}) for: {src}",
                lit.name, lit.blockers
            );
        }
    }
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

// ----- Nullish coalescing -----

#[test]
fn nullish_coalesce_lowers() {
    assert_lowered("function f(x) { return x ?? 0; }");
}

#[test]
fn nullish_coalesce_chained_lowers() {
    assert_lowered("function f(a, b, c) { return a ?? b ?? c; }");
}

#[test]
fn nullish_coalesce_roundtrips() {
    let out = ssa_roundtrip_emit("function f(x) { return x ?? 5; }");
    // The condition test is `x == null`; the join-block param yields
    // either `x` or `5`. After SSA re-emission the shape is a ternary
    // or sequence — either way, the function is no longer PassThrough.
    assert!(!out.is_empty());
}

// ----- Optional chain -----

#[test]
fn optional_member_lowers() {
    assert_lowered("function f(o) { return o?.x; }");
}

#[test]
fn optional_call_lowers() {
    assert_lowered("function f(o) { return o?.(); }");
}

#[test]
fn optional_index_lowers() {
    assert_lowered("function f(o, k) { return o?.[k]; }");
}

#[test]
fn optional_member_then_call_lowers() {
    // `o?.m()` — the call must keep `this === o`. Verified via SSA
    // emit using CallMethod; the lowered shape will round-trip.
    assert_lowered("function f(o) { return o?.m(); }");
}

#[test]
fn optional_chain_long_lowers() {
    assert_lowered("function f(a) { return a?.b.c?.d; }");
}

#[test]
fn optional_chain_roundtrips() {
    let out = ssa_roundtrip_emit("function f(o) { return o?.x; }");
    assert!(!out.is_empty());
}

// ----- Logical assignment -----

#[test]
fn nullish_assign_ident_lowers() {
    assert_lowered("function f(x) { x ??= 5; return x; }");
}

#[test]
fn or_assign_member_lowers() {
    assert_lowered("function f(o) { o.x ||= 1; return o.x; }");
}

#[test]
fn and_assign_index_lowers() {
    assert_lowered("function f(o, k) { o[k] &&= 2; return o[k]; }");
}

#[test]
fn logical_assign_in_expr_position_lowers() {
    assert_lowered("function f(x) { return (x ??= 1) + 1; }");
}

#[test]
fn logical_assign_roundtrips() {
    let out = ssa_roundtrip_emit("function f(x) { x ??= 5; return x; }");
    assert!(!out.is_empty());
}

// ----- Delete -----

#[test]
fn delete_member_lowers() {
    assert_lowered("function f(o) { return delete o.x; }");
}

#[test]
fn delete_index_lowers() {
    assert_lowered("function f(o, k) { return delete o[k]; }");
}

#[test]
fn delete_bare_ident_still_blocked() {
    // `delete x` of a bare identifier is sloppy-mode-only and stays
    // unsupported.
    let m = parse_and_lower("function f() { var x = 1; return delete x; }")
        .expect("parse_and_lower");
    let mut found_pt = false;
    for item in &m.items {
        if let Item::PassThroughFunction(_) = item {
            found_pt = true;
        }
    }
    assert!(found_pt, "expected PassThroughFunction for `delete x`");
}

#[test]
fn delete_roundtrips() {
    let out = ssa_roundtrip_emit("function f(o) { return delete o.x; }");
    assert!(out.contains("delete"), "expected `delete` in emitted: {out}");
}

// ----- Existing operators (`**`, `in`, `instanceof`) — already lowerable -----

#[test]
fn pow_lowers() {
    assert_lowered("function f(x, y) { return x ** y; }");
}

#[test]
fn in_lowers() {
    assert_lowered("function f(o) { return 'foo' in o; }");
}

#[test]
fn instanceof_lowers() {
    assert_lowered("function f(x) { return x instanceof Error; }");
}
