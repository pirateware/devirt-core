//! Control-flow unflattening tests. Structural shape only; soundness (value
//! preservation, decline-to-transform) lives in tests/equivalence.rs.

use js_deobfuscator::deobfuscate;

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
fn single_state_dispatcher_collapses() {
    // The sample_12 shape: a one-case dispatcher around a single effect.
    let out = deob("var s = 1; while (s) { switch (s) { case 1: sink(1); s = 0; break; } }");
    assert!(out.contains("sink(1)"), "body lost: {out}");
    assert!(!out.contains("while"), "loop survived: {out}");
    assert!(!out.contains("switch"), "switch survived: {out}");
}

#[test]
fn multi_state_chain_emits_in_order() {
    // States run 1 → 2 → 3; the emitted statements must appear in that order.
    let out = deob(
        "var s = 1; while (s) { switch (s) { \
         case 2: b(); s = 3; break; \
         case 1: a(); s = 2; break; \
         case 3: c(); s = 0; break; } }",
    );
    assert!(
        !out.contains("while") && !out.contains("switch"),
        "dispatcher survived: {out}"
    );
    let ia = out.find("a()").expect("a missing");
    let ib = out.find("b()").expect("b missing");
    let ic = out.find("c()").expect("c missing");
    assert!(ia < ib && ib < ic, "statements out of run order: {out}");
}

#[test]
fn dispatcher_with_external_state_use_is_kept() {
    // `s` is also read after the loop, so it is not a private dispatch var: the
    // construct must be left intact.
    let out = deob("var s = 1; while (s) { switch (s) { case 1: a(); s = 0; break; } } sink(s);");
    assert!(out.contains("switch"), "dispatcher wrongly removed: {out}");
}

#[test]
fn cyclic_dispatcher_is_kept() {
    // A revisited state is a real loop, not a linear chain — must not unflatten.
    let out = deob(
        "var s = 1; while (s) { switch (s) { case 1: a(); s = 2; break; case 2: b(); s = 1; break; } }",
    );
    assert!(
        out.contains("switch"),
        "cyclic dispatcher wrongly unflattened: {out}"
    );
}

// --- Generalized loop shapes (redesign) -----------------------------------

#[test]
fn while_neq_terminal_collapses() {
    // Non-falsy exit: `while (s != K)` exits when s == K.
    let out = deob(
        "var s = 1; while (s != 9) { switch (s) { \
         case 1: a(); s = 2; break; case 2: b(); s = 9; break; } }",
    );
    assert!(!out.contains("switch") && !out.contains("while"), "not unflattened: {out}");
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn do_while_neq_collapses() {
    // do-while runs the body before testing; exit when s == K.
    let out = deob(
        "var s = 1; do { switch (s) { \
         case 1: a(); s = 2; break; case 2: b(); s = 7; break; } } while (s != 7);",
    );
    assert!(!out.contains("switch"), "do-while not unflattened: {out}");
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn compound_transition_collapses() {
    // Relative transitions: s += K (1 -> 5 -> 13, exit at 13).
    let out = deob(
        "var s = 1; while (s != 13) { switch (s) { \
         case 1: a(); s += 4; break; case 5: b(); s += 8; break; } }",
    );
    assert!(!out.contains("switch"), "compound-transition not unflattened: {out}");
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn symbolic_const_labels_collapse() {
    // Write-once integer constants used as labels/transitions/init resolve.
    let out = deob(
        "var A = 1, B = 2, Z = 0; var s = A; while (s) { switch (s) { \
         case A: a(); s = B; break; case B: b(); s = Z; break; } }",
    );
    assert!(!out.contains("switch"), "symbolic-label machine not unflattened: {out}");
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn real_do_while_loop_is_kept() {
    // A genuine loop (revisits state 1) must NOT be linearized.
    let out = deob(
        "var s = 1; do { switch (s) { \
         case 1: a(); s = 2; break; case 2: b(); s = 1; break; } } while (s);",
    );
    assert!(out.contains("switch"), "cyclic do-while wrongly unflattened: {out}");
}
