//! String-array-order dispatcher unflattening tests (the Cloudflare CFF shape).
//!
//! Structural shape + decline behavior. Value-preservation (the real soundness
//! net) lives in tests/equivalence.rs (`string_order_*`).

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

// --- should-fire ----------------------------------------------------------

#[test]
fn split_order_chain_collapses_in_array_order() {
    // The canonical CFF shape. Array order "2|0|1" means the bodies must appear in
    // the order c2, c0, c1 — NOT the textual case order.
    let out = deob(
        "(function () { \
           for (var a = \"2|0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { \
               case \"0\": b0(); continue; \
               case \"1\": b1(); continue; \
               case \"2\": b2(); continue; \
             } \
             break; \
           } \
         })();",
    );
    assert!(!out.contains("switch"), "switch survived: {out}");
    assert!(!out.contains("split"), "order array survived: {out}");
    let i2 = out.find("b2()").expect("b2 missing");
    let i0 = out.find("b0()").expect("b0 missing");
    let i1 = out.find("b1()").expect("b1 missing");
    assert!(i2 < i0 && i0 < i1, "statements out of array order: {out}");
}

#[test]
fn trailing_break_in_block_body_collapses() {
    // The exact s5/s6 body shape: `{ switch (...) {...} break; }` — the break is the
    // loop exit once the cursor runs off the array end.
    let out = deob(
        "(function () { \
           for (var a = \"0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { case \"0\": x(); continue; case \"1\": y(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(!out.contains("switch"), "switch survived: {out}");
    let ix = out.find("x()").unwrap();
    let iy = out.find("y()").unwrap();
    assert!(ix < iy, "order wrong: {out}");
}

#[test]
fn return_terminal_stops_the_walk() {
    // A `return` case stops the chain; later array keys must NOT be emitted.
    let out = deob(
        "(function () { \
           for (var a = \"0|1|2\".split(\"|\"), i = 0; true;) \
             switch (a[i++]) { \
               case \"0\": x(); continue; \
               case \"1\": return done(); \
               case \"2\": never(); continue; \
             } \
         })();",
    );
    assert!(!out.contains("switch"), "switch survived: {out}");
    assert!(
        out.contains("x()") && out.contains("done()"),
        "missing: {out}"
    );
    assert!(!out.contains("never()"), "dead tail emitted: {out}");
}

#[test]
fn param_cursor_assignment_form_collapses() {
    // The real sample shape: `a`/`i` are params, the for-init *assigns* (not declares)
    // them, and they are dispatch-only.
    let out = deob(
        "(function () { \
           (function (a, i) { \
             for (a = \"1|0\".split(\"|\"), i = 0; true;) { \
               switch (a[i++]) { case \"0\": p(); continue; case \"1\": q(); continue; } \
               break; \
             } \
           })(); \
         })();",
    );
    assert!(!out.contains("switch"), "switch survived: {out}");
    let iq = out.find("q()").unwrap();
    let ip = out.find("p()").unwrap();
    assert!(iq < ip, "array order (1|0) not honored: {out}");
}

#[test]
fn for_semicolon_no_test_collapses() {
    // `for (init; ; )` — empty test — is also a never-exit loop.
    let out = deob(
        "(function () { \
           for (var a = \"0\".split(\"|\"), i = 0; ; ) { \
             switch (a[i++]) { case \"0\": only(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(!out.contains("switch"), "switch survived: {out}");
    assert!(out.contains("only()"), "body lost: {out}");
}

#[test]
fn literal_array_form_collapses() {
    // A pre-split literal array of keys (what const-fold may produce) works too.
    let out = deob(
        "(function () { \
           for (var a = [\"1\", \"0\"], i = 0; true;) { \
             switch (a[i++]) { case \"0\": m(); continue; case \"1\": n(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(!out.contains("switch"), "switch survived: {out}");
    let in_ = out.find("n()").unwrap();
    let im = out.find("m()").unwrap();
    assert!(in_ < im, "array order not honored: {out}");
}

// --- adversarial: must NOT fire ------------------------------------------

#[test]
fn mutated_order_array_is_kept() {
    // `a.push(...)` mutates the order array — it is no longer a static chain.
    let out = deob(
        "(function () { \
           var a = \"0|1\".split(\"|\"); \
           a.push(\"2\"); \
           var i = 0; \
           for (; true;) { \
             switch (a[i++]) { case \"0\": x(); continue; case \"1\": y(); continue; case \"2\": z(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(
        out.contains("switch"),
        "mutated-array dispatcher wrongly collapsed: {out}"
    );
}

#[test]
fn index_reset_midloop_is_kept() {
    // A case resets the cursor (`i = 0`): not a monotonic walk → must decline. The
    // body reads/writes `i`, which also trips the dispatch-only gate.
    let out = deob(
        "(function () { \
           for (var a = \"0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { case \"0\": x(); i = 0; continue; case \"1\": y(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(
        out.contains("switch"),
        "index-reset dispatcher wrongly collapsed: {out}"
    );
}

#[test]
fn non_constant_order_array_is_kept() {
    // The order array is a runtime value (a member read), not a constant.
    let out = deob(
        "(function (src) { \
           for (var a = src.keys.split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { case \"0\": x(); continue; case \"1\": y(); continue; } \
             break; \
           } \
         })(window);",
    );
    assert!(
        out.contains("switch"),
        "non-constant-array dispatcher wrongly collapsed: {out}"
    );
}

#[test]
fn default_case_is_kept() {
    // A `default` clause is a fallback path we don't model.
    let out = deob(
        "(function () { \
           for (var a = \"0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { case \"0\": x(); continue; case \"1\": y(); continue; default: d(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(
        out.contains("switch"),
        "default-bearing dispatcher wrongly collapsed: {out}"
    );
}

#[test]
fn body_reading_cursor_is_kept() {
    // A case body reads the order array (`a.length`) — it can observe dispatch state,
    // so emitting it linearly would change behavior.
    let out = deob(
        "(function () { \
           for (var a = \"0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { case \"0\": log(a.length); continue; case \"1\": y(); continue; } \
             break; \
           } \
         })();",
    );
    assert!(
        out.contains("switch"),
        "cursor-reading dispatcher wrongly collapsed: {out}"
    );
}

#[test]
fn order_array_used_after_loop_is_kept() {
    // `a` is read after the loop — not dispatch-only.
    let out = deob(
        "(function () { \
           for (var a = \"0|1\".split(\"|\"), i = 0; true;) { \
             switch (a[i++]) { case \"0\": x(); continue; case \"1\": y(); continue; } \
             break; \
           } \
           sink(a); \
         })();",
    );
    assert!(
        out.contains("switch"),
        "escaping-array dispatcher wrongly collapsed: {out}"
    );
}

#[test]
fn real_test_exit_is_kept() {
    // A loop with a *real* test (`i < a.length`) can exit before the chain ends —
    // not a never-exit `for(;;)`. Must decline.
    let out = deob(
        "(function () { \
           for (var a = \"0|1\".split(\"|\"), i = 0; i < a.length;) { \
             switch (a[i++]) { case \"0\": x(); continue; case \"1\": y(); continue; } \
           } \
         })();",
    );
    assert!(
        out.contains("switch"),
        "real-test dispatcher wrongly collapsed: {out}"
    );
}
