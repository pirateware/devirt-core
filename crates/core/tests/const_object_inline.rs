//! Constant object-property inlining: the obfuscator string-map dictionary.
//! Behavioural value-preservation lives in `tests/equivalence.rs`; these pin the
//! structural result (the dictionary is resolved and removed) and the conservative
//! guards (escape / mutation / non-first-statement → declined).

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn string_map_read_in_persistent_closure_inlined() {
    // The read lives in a closure that is *not* inlined away, so the existing
    // intraprocedural fold cannot reach it — this pass's reason to exist.
    let out = deob(
        "(function(){ var O = { koPtQ: 'Original:' }; var log = function(){ console.log(O.koPtQ); }; log(); })();",
    );
    assert!(out.contains("\"Original:\""), "string not inlined: {out}");
    assert!(!out.contains("koPtQ"), "dictionary survived: {out}");
}

#[test]
fn opaque_predicate_over_two_constants_folds() {
    // `O.a === O.b` with distinct constants is always false; its dead arm must go.
    let out = deob(
        "(function(){ var O = { a: 'x', b: 'y', hit: 'KEPT' }; if (O.a === O.b) { dead(); } else { console.log(O.hit); } })();",
    );
    assert!(out.contains("\"KEPT\""), "taken branch lost: {out}");
    assert!(!out.contains("dead()"), "dead arm survived: {out}");
}

#[test]
fn chained_dictionary_resolves() {
    // `P.c = O.ord` then `P.c.split(...)`: one property hop resolves per round.
    let out = deob(
        "(function(){ var O = { ord: '1|0' }; var P = { c: O.ord }; return P.c.split('|'); })();",
    );
    assert!(
        out.contains("[\n\t\t\"1\",\n\t\t\"0\"\n\t]") || out.contains("[\"1\", \"0\"]"),
        "order array not materialized: {out}"
    );
}

#[test]
fn function_valued_property_is_left_alone() {
    // Only literal-valued props inline; a method stays, so an object can shrink to
    // just its live method rather than being mishandled.
    let out = deob(
        "(function(){ var O = { s: 'HI', f: function(x){ return x; } }; console.log(O.s); return O.f(1); })();",
    );
    assert!(out.contains("\"HI\""), "literal prop not inlined: {out}");
}

#[test]
fn mutated_object_not_inlined() {
    // A reassigned property must keep its read as a real access (no stale value).
    let out = deob("(function(){ var O = { a: 'INIT' }; O.a = 'NEW'; return O.a; })();");
    assert!(out.contains("\"NEW\""), "mutation lost: {out}");
    // The read must stay a real member access, not the stale initial literal.
    assert!(out.contains(".a"), "read replaced by stale value: {out}");
    assert!(!out.contains("return \"INIT\""), "stale initial value inlined: {out}");
}

#[test]
fn escaped_object_not_inlined() {
    // Passed by reference: a callee could read or mutate it, so reads stay.
    let out = deob("(function(){ var O = { a: 'X' }; use(O); return O.a; })();");
    assert!(out.contains("O.a") || out.contains(".a"), "escaped read inlined: {out}");
}

#[test]
fn computed_access_not_inlined() {
    let out = deob("(function(k){ var O = { a: 'X', b: 'Y' }; return O[k]; })('a');");
    assert!(out.contains("a: \"X\"") && out.contains("b: \"Y\""), "object wrongly resolved: {out}");
}
