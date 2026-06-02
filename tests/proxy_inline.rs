//! Proxy-operator inlining tests (standalone and object-method forms).

use js_deobfuscator::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn standalone_operator_proxy_inlines() {
    // Wrapped in a function so the spent proxy isn't a top-level global (which
    // DCE preserves as observable output).
    let out = deob("(function(){ function p(a, b){ return a === b; } sink(p(x, y)); })();");
    assert!(out.contains("x === y"), "standalone proxy not inlined: {out}");
    assert!(!out.contains("function p"), "spent proxy not removed: {out}");
}

#[test]
fn object_method_operator_proxy_inlines() {
    let out = deob(
        "var O = { lt: function(a,b){ return a < b; }, an: function(a,b){ return a & b; } };
         sink(O.lt(x, y), O.an(x, y));",
    );
    assert!(out.contains("x < y"), "object-method proxy (lt) not inlined: {out}");
    assert!(out.contains("x & y"), "object-method proxy (an) not inlined: {out}");
    assert!(!out.contains("O.lt(") && !out.contains("O.an("), "proxy call survived: {out}");
}

#[test]
fn mutated_table_is_not_inlined() {
    // A table whose member is reassigned can't be assumed fixed.
    let out = deob(
        "var O = { lt: function(a,b){ return a < b; } };
         O.lt = function(a,b){ return a > b; };
         sink(O.lt(x, y));",
    );
    assert!(out.contains("O.lt(x, y)"), "mutated table was unsoundly inlined: {out}");
}

#[test]
fn reassigned_object_is_not_inlined() {
    let out = deob(
        "var O = { lt: function(a,b){ return a < b; } };
         O = other;
         sink(O.lt(x, y));",
    );
    assert!(out.contains("O.lt(x, y)"), "reassigned object was unsoundly inlined: {out}");
}
