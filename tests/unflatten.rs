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
    assert!(
        !out.contains("switch") && !out.contains("while"),
        "not unflattened: {out}"
    );
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
    assert!(
        !out.contains("switch"),
        "compound-transition not unflattened: {out}"
    );
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn mul_transition_collapses() {
    // Multiplicative transition: s *= K (1 -> 3 -> 12, exit at 12).
    let out = deob(
        "var s = 1; while (s != 12) { switch (s) { \
         case 1: a(); s *= 3; break; case 3: b(); s *= 4; break; } }",
    );
    assert!(
        !out.contains("switch"),
        "mul-transition not unflattened: {out}"
    );
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn cyclic_mul_transition_is_kept() {
    // `s *= 1` revisits the same state forever — a real loop, must NOT linearize.
    let out = deob(
        "var s = 1; while (s) { switch (s) { \
         case 1: a(); s *= 1; break; } }",
    );
    assert!(
        out.contains("switch"),
        "cyclic mul-transition wrongly unflattened: {out}"
    );
}

#[test]
fn symbolic_const_labels_collapse() {
    // Write-once integer constants used as labels/transitions/init resolve.
    let out = deob(
        "var A = 1, B = 2, Z = 0; var s = A; while (s) { switch (s) { \
         case A: a(); s = B; break; case B: b(); s = Z; break; } }",
    );
    assert!(
        !out.contains("switch"),
        "symbolic-label machine not unflattened: {out}"
    );
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn fall_through_case_collapses() {
    // Case 1 has no `break`/transition: control falls through into case 2's body
    // (textual successor), so the run order is a(), b(). Then 2 -> 0 exits.
    let out = deob(
        "var s = 1; while (s) { switch (s) { \
         case 1: a(); case 2: b(); s = 0; break; } }",
    );
    assert!(
        !out.contains("switch") && !out.contains("while"),
        "fall-through not unflattened: {out}"
    );
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "fall-through order wrong: {out}");
}

#[test]
fn fall_through_chain_to_jump_target_collapses() {
    // Jump into a fall-through group: 1 -> a(); jump to 2. 2 falls into 3.
    // Order: a(), then (entering at 2) b(), c().
    let out = deob(
        "var s = 1; while (s) { switch (s) { \
         case 1: a(); s = 2; break; case 2: b(); case 3: c(); s = 0; break; } }",
    );
    assert!(!out.contains("switch"), "not unflattened: {out}");
    let ia = out.find("a()").unwrap();
    let ib = out.find("b()").unwrap();
    let ic = out.find("c()").unwrap();
    assert!(ia < ib && ib < ic, "order wrong: {out}");
}

#[test]
fn fall_through_off_end_is_kept() {
    // The last case falls through with no textual successor: falling off the end
    // of the switch is not a state transition we can follow — must decline.
    let out = deob(
        "var s = 1; while (s) { switch (s) { \
         case 1: a(); s = 2; break; case 2: b(); } }",
    );
    assert!(
        out.contains("switch"),
        "fall-off-end wrongly unflattened: {out}"
    );
}

#[test]
fn real_do_while_loop_is_kept() {
    // A genuine loop (revisits state 1) must NOT be linearized.
    let out = deob(
        "var s = 1; do { switch (s) { \
         case 1: a(); s = 2; break; case 2: b(); s = 1; break; } } while (s);",
    );
    assert!(
        out.contains("switch"),
        "cyclic do-while wrongly unflattened: {out}"
    );
}

#[test]
fn string_state_dispatcher_collapses() {
    // The dispatch state is a string; transitions are `s = "label"`. Run order
    // "a" -> "b" -> exit.
    let out = deob(
        "var s = \"a\"; while (s) { switch (s) { \
         case \"a\": a(); s = \"b\"; break; case \"b\": b(); s = \"\"; break; } }",
    );
    assert!(
        !out.contains("switch") && !out.contains("while"),
        "string-state machine not unflattened: {out}"
    );
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn string_state_neq_terminal_collapses() {
    // Non-falsy string exit: `while (s != "done")`.
    let out = deob(
        "var s = \"x\"; while (s != \"done\") { switch (s) { \
         case \"x\": a(); s = \"y\"; break; case \"y\": b(); s = \"done\"; break; } }",
    );
    assert!(!out.contains("switch"), "not unflattened: {out}");
    let (ia, ib) = (out.find("a()").unwrap(), out.find("b()").unwrap());
    assert!(ia < ib, "order wrong: {out}");
}

#[test]
fn boolean_state_dispatcher_collapses() {
    // Boolean state: start true, case true does work and flips to false; exit on
    // `while (s)` going false.
    let out = deob(
        "var s = true; while (s) { switch (s) { \
         case true: a(); s = false; break; } }",
    );
    assert!(
        !out.contains("switch"),
        "boolean-state not unflattened: {out}"
    );
    assert!(out.contains("a()"), "body lost: {out}");
}

#[test]
fn cyclic_string_state_is_kept() {
    // A string-state machine that revisits "a" is a real loop — must NOT linearize.
    let out = deob(
        "var s = \"a\"; while (s) { switch (s) { \
         case \"a\": a(); s = \"b\"; break; case \"b\": b(); s = \"a\"; break; } }",
    );
    assert!(
        out.contains("switch"),
        "cyclic string-state wrongly unflattened: {out}"
    );
}

#[test]
fn string_state_external_use_is_kept() {
    // `s` (string) is read after the loop — not a private dispatch var, keep it.
    let out = deob(
        "var s = \"a\"; while (s) { switch (s) { case \"a\": a(); s = \"\"; break; } } sink(s);",
    );
    assert!(
        out.contains("switch"),
        "string dispatcher wrongly removed: {out}"
    );
}

#[test]
fn semantic_string_switch_is_kept() {
    // A real (non-CFF) switch on an externally-supplied string: the discriminant
    // is a parameter, not a dispatch-only local with literal transitions. Must NOT
    // be touched.
    let out = deob(
        "function f(ev) { switch (ev) { case \"open\": a(); break; case \"close\": b(); break; } } f(x);",
    );
    assert!(
        out.contains("switch"),
        "semantic string switch wrongly unflattened: {out}"
    );
}

#[test]
fn param_state_dispatcher_specializes() {
    // State is a function parameter, selected by a constant entry at the call site.
    // The machine must specialize per entry and the switch must vanish.
    let out = deob(
        "(function(){ \
           var f = function self(s, o) { while (s != 9) { switch (s) { \
             case 1: { o.v += 10; s = 2; break; } case 2: { o.v += 5; s = 9; break; } } } }; \
           var o = { v: 0 }; f(1, o); sink(o.v); })();",
    );
    assert!(
        !out.contains("switch"),
        "param-state switch survived: {out}"
    );
    assert!(out.contains("__pf_"), "no specialization emitted: {out}");
}
