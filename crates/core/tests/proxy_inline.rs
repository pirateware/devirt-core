//! Proxy-operator inlining tests (standalone and object-method forms).

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(
        r.parse_errors.is_empty(),
        "parse errors: {:?}",
        r.parse_errors
    );
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn standalone_operator_proxy_inlines() {
    // Wrapped in a function so the spent proxy isn't a top-level global (which
    // DCE preserves as observable output).
    let out = deob("(function(){ function p(a, b){ return a === b; } sink(p(x, y)); })();");
    assert!(
        out.contains("x === y"),
        "standalone proxy not inlined: {out}"
    );
    assert!(
        !out.contains("function p"),
        "spent proxy not removed: {out}"
    );
}

#[test]
fn object_method_operator_proxy_inlines() {
    let out = deob(
        "var O = { lt: function(a,b){ return a < b; }, an: function(a,b){ return a & b; } };
         sink(O.lt(x, y), O.an(x, y));",
    );
    assert!(
        out.contains("x < y"),
        "object-method proxy (lt) not inlined: {out}"
    );
    assert!(
        out.contains("x & y"),
        "object-method proxy (an) not inlined: {out}"
    );
    assert!(
        !out.contains("O.lt(") && !out.contains("O.an("),
        "proxy call survived: {out}"
    );
}

#[test]
fn assignment_initialized_table_inlines() {
    // The table is established by a single `O = {…}` assignment (param/var
    // pre-declared), the form real VM decoders use — still soundly inlinable.
    let out =
        deob("(function(O){ O = { lt: function(a,b){ return a < b; } }; sink(O.lt(x, y)); })();");
    assert!(
        out.contains("x < y"),
        "assignment-init table not inlined: {out}"
    );
    assert!(!out.contains("O.lt("), "proxy call survived: {out}");
}

#[test]
fn apply_proxy_inlines() {
    let out = deob(
        "(function(){ var O = { ap: function(f, x){ return f(x); } }; sink(O.ap(g, y)); })();",
    );
    assert!(out.contains("g(y)"), "apply proxy not inlined: {out}");
    assert!(!out.contains("O.ap("), "apply proxy call survived: {out}");
}

#[test]
fn call_and_apply_invocation_forms_inline() {
    let out = deob(
        "(function(){ var O = { lt: function(a,b){ return a < b; }, sub: function(a,b){ return a - b; } };
           sink(O.lt.call(null, x, y), O.sub.apply(null, [m, n])); })();",
    );
    assert!(out.contains("x < y"), ".call form not inlined: {out}");
    assert!(out.contains("m - n"), ".apply form not inlined: {out}");
    assert!(
        !out.contains(".call(") && !out.contains(".apply("),
        "invocation survived: {out}"
    );
}

#[test]
fn call_with_impure_receiver_is_not_inlined() {
    // The receiver `boom()` is evaluated then discarded; dropping it would lose
    // its side effect, so the inline must be declined.
    let out = deob(
        "(function(){ var O = { lt: function(a,b){ return a < b; } }; sink(O.lt.call(boom(), x, y)); })();",
    );
    assert!(
        out.contains(".call(boom()"),
        "impure receiver was unsoundly dropped: {out}"
    );
}

#[test]
fn apply_proxy_with_method_arg_is_not_inlined() {
    // Substituting `obj.m` into callee position would re-bind `this` to obj.
    let out = deob(
        "(function(obj){ var O = { ap: function(f, x){ return f(x); } }; sink(O.ap(obj.m, y)); })(t);",
    );
    assert!(
        out.contains(".ap("),
        "receiver-binding apply was unsoundly inlined: {out}"
    );
}

#[test]
fn function_argument_inlines_without_panic() {
    // A function arg resets its clone's scope_id; inlining on *exit* keeps the
    // walker from descending into it, so the call inlines instead of being
    // skipped. The param is single-use, so the function is moved to one site.
    let out = deob(
        "var O = { eq: function(a,b){ return a === b; } };
         sink(O.eq(function(){ return 1; }, g));",
    );
    assert!(!out.contains("O.eq("), "function-arg call not inlined: {out}");
    assert!(out.contains("=== g"), "body not substituted: {out}");
}

#[test]
fn call_forwarder_with_function_arg_inlines() {
    // The obfuscator's self-defending call proxy: `f(a, b)` forwarder invoked as
    // `P.uPWCB(fn, ctx, cb)` → `fn(ctx, cb)`, even with a function callback arg.
    let out = deob(
        "var P = { uPWCB: function(f, a, b){ return f(a, b); } };
         var r = P.uPWCB(host, this, function(){ return 7; }); sink(r);",
    );
    assert!(!out.contains("uPWCB"), "call proxy survived: {out}");
    assert!(out.contains("host(this, function"), "forward not applied: {out}");
}

#[test]
fn mutated_table_is_not_inlined() {
    // A table whose member is reassigned can't be assumed fixed.
    let out = deob(
        "var O = { lt: function(a,b){ return a < b; } };
         O.lt = function(a,b){ return a > b; };
         sink(O.lt(x, y));",
    );
    assert!(
        out.contains("O.lt(x, y)"),
        "mutated table was unsoundly inlined: {out}"
    );
}

#[test]
fn reassigned_object_is_not_inlined() {
    let out = deob(
        "var O = { lt: function(a,b){ return a < b; } };
         O = other;
         sink(O.lt(x, y));",
    );
    assert!(
        out.contains("O.lt(x, y)"),
        "reassigned object was unsoundly inlined: {out}"
    );
}
