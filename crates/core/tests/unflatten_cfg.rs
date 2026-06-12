//! Control-flow-recovery tests for the CFG/dispatch passes. Structural patterns
//! only — never a specific sample.

use devirt_core::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(r.parse_errors.is_empty(), "parse errors: {:?}", r.parse_errors);
    r.code
}

/// A flattened dispatcher with *relative* state transitions (`s += 1`, `s -= 1`)
/// and a back edge (state 2 ⇄ 3) is recovered into structured control flow via
/// the dispatch substrate — the linear unflattener loops and the absolute-only
/// `cff::build` declines on the relative steps. The function is called with an
/// opaque argument so its body survives constant evaluation, exposing the
/// recovered structure.
#[test]
fn relative_transition_dispatcher_is_structured() {
    let src = r#"
        function f(n) {
          var s = 1, acc = 0, i = 0;
          while (s != 0) {
            switch (s) {
              case 1: acc = 0; i = 0; s += 1; break;
              case 2: if (i < n) { s += 1; } else { s = 0; } break;
              case 3: acc += i; i += 1; s -= 1; break;
            }
          }
          return acc;
        }
        sink(f(externalInput));
    "#;
    let out = deob(src);
    // The switch dispatcher is gone, replaced by a real loop.
    assert!(!out.contains("switch"), "dispatcher not recovered: {out}");
    assert!(out.contains("while"), "loop not recovered: {out}");
    // The recovered body keeps the original work verbatim.
    assert!(out.contains("acc += "), "loop body lost: {out}");
}

/// Soundness guard: a dispatcher whose case writes the state variable in an
/// *unmodeled* way (here, multiplying it by a runtime value) must be left intact —
/// the substrate models only absolute/relative integer steps and declines the
/// rest, so the original switch survives rather than being mis-structured.
#[test]
fn unmodeled_state_write_is_left_intact() {
    let src = r#"
        function g(n) {
          var s = 1, acc = 0;
          while (s != 0) {
            switch (s) {
              case 1: acc += 1; s = s * n + 1; break;
              case 2: acc += 2; s = 0; break;
            }
          }
          return acc;
        }
        sink(g(externalInput));
    "#;
    let out = deob(src);
    // `s = s * n + 1` is not a modeled transition → the dispatcher is untouched.
    assert!(out.contains("switch"), "unmodeled dispatcher wrongly altered: {out}");
}
