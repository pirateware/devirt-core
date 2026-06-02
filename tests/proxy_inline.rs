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
fn assignment_initialized_table_inlines() {
    // The table is established by a single `O = {…}` assignment (param/var
    // pre-declared), the form real VM decoders use — still soundly inlinable.
    let out = deob(
        "(function(O){ O = { lt: function(a,b){ return a < b; } }; sink(O.lt(x, y)); })();",
    );
    assert!(out.contains("x < y"), "assignment-init table not inlined: {out}");
    assert!(!out.contains("O.lt("), "proxy call survived: {out}");
}

#[test]
fn function_argument_does_not_panic_and_is_skipped() {
    // Splicing a cloned function arg would reset its scope_id and crash the
    // walker; such calls must be left intact, not inlined.
    let out = deob(
        "var O = { eq: function(a,b){ return a === b; } };
         sink(O.eq(function(){ return 1; }, g));",
    );
    assert!(out.contains("O.eq("), "function-arg call should be left intact: {out}");
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
