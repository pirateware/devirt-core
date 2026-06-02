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
    let out = deob("(function(){ var O = { used: 1, deadFn: function(){return 2;}, deadData: 3 }; return O.used; })();");
    assert!(out.contains("used: 1"), "live member lost: {out}");
    assert!(!out.contains("deadFn") && !out.contains("deadData"), "dead members survived: {out}");
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
