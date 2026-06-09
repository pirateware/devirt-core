//! Object-state lowering (src/passes/objstate_lower.rs). Structural shape only;
//! value preservation and the escape/read-before-init/capture declines live in
//! tests/equivalence.rs.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    assert!(r.converged, "did not converge");
    r.code
}

#[test]
fn object_state_dispatcher_is_recovered() {
    let out = deob(
        "var ctx = { s: 1, acc: 0 }; \
         while (ctx.s) { switch (ctx.s) { \
           case 1: ctx.acc += inp(); ctx.s = 2; break; \
           case 2: ctx.acc += 5; ctx.s = 0; break; } } \
         sink(ctx.acc);",
    );
    assert!(!out.contains("switch"), "object-state dispatcher survived: {out}");
    assert!(!out.contains("while"), "loop survived: {out}");
}

#[test]
fn escaping_state_object_is_kept() {
    let out = deob(
        "var ctx = { s: 1, a: 0 }; leak(ctx); \
         while (ctx.s) { switch (ctx.s) { case 1: ctx.a += 1; ctx.s = 0; break; } } sink(ctx.a);",
    );
    assert!(out.contains("switch"), "escaping object wrongly lowered: {out}");
}
