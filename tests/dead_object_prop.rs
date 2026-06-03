//! Dead object-property elimination tests. Escape analysis must default to keep.

use js_deobfuscator::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn unaccessed_pure_members_removed() {
    // The SSA dataflow pass now folds the single live `O.used` read to its
    // constant and DCE collapses the whole table — a strictly stronger result
    // than pruning dead members in place. The value is preserved (see
    // `tests/equivalence.rs::dead_object_prop_removal_preserves_value`).
    let out = deob("(function(){ var O = { used: 1, deadFn: function(){return 2;}, deadData: 3 }; return O.used; })();");
    assert!(out.contains("return 1"), "live read not folded to its constant: {out}");
    assert!(
        !out.contains("deadFn") && !out.contains("deadData") && !out.contains("used"),
        "dead table survived: {out}"
    );
}

#[test]
fn assignment_initialized_table_pruned() {
    // The table is stored into a pre-declared param via a single `O = {…}`. SSA
    // resolves `O.used` through that single reaching object literal and folds it.
    let out = deob("(function(O){ O = { used: 1, deadFn: function(){return 2;} }; return O.used; })();");
    assert!(out.contains("return 1"), "live read not folded to its constant: {out}");
    assert!(!out.contains("deadFn") && !out.contains("used"), "dead table survived: {out}");
}

#[test]
fn twice_assigned_object_not_pruned() {
    let out = deob("(function(O){ O = { a: 1, b: 2 }; O = other; return O.a; })();");
    assert!(out.contains("b: 2"), "object with two writes was pruned: {out}");
}

#[test]
fn computed_access_disables_removal() {
    let out = deob("(function(k){ var O = { a: 1, b: 2 }; return O[k]; })('a');");
    assert!(out.contains("a: 1") && out.contains("b: 2"), "computed-access object was pruned: {out}");
}

#[test]
fn pass_by_reference_disables_removal() {
    let out = deob("(function(){ var O = { a: 1, b: 2 }; use(O); return O.a; })();");
    assert!(out.contains("b: 2"), "escaped object was pruned: {out}");
}

#[test]
fn member_write_disables_removal() {
    let out = deob("(function(){ var O = { a: 1, b: 2 }; O.b = 5; return O.a; })();");
    assert!(out.contains("b: 2"), "mutated object was pruned: {out}");
}

#[test]
fn impure_value_member_kept() {
    let out = deob("(function(){ var O = { a: 1, dead: makeThing() }; return O.a; })();");
    assert!(out.contains("dead: makeThing()"), "impure-valued member wrongly dropped: {out}");
}

#[test]
fn getter_kept() {
    let out = deob("(function(){ var O = { a: 1, get g(){ return side(); } }; return O.a; })();");
    assert!(out.contains("get g"), "accessor wrongly dropped: {out}");
}
