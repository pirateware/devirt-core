//! Whole-program differential equivalence (the end-to-end soundness gate).
//!
//! `tests/equivalence.rs` runs small expression-returning snippets in a bare
//! sandbox. This complements it: each fixture is a *whole program* with top-level
//! code and `console` output, run through the permissive browser-host shim
//! (`sandbox::behavior_signature`) both before and after deobfuscation. The
//! observable behavior — terminal outcome plus the exact console output sequence —
//! must match. This exercises the transforms holistically (top-level statements,
//! output ordering, multi-feature interaction) the way a real sample would.

use devirt_core::deobfuscate;
use devirt_core::sandbox::{behavior_signature, RunOutcome};
use std::time::Duration;

/// Deobfuscate `src`, then require the original and the output to share a behavior
/// signature (skipping the non-comparable timed-out outcome).
fn assert_whole_program_equivalent(src: &str) {
    let budget = Duration::from_secs(5);
    let original = behavior_signature(src, budget);
    if original.outcome == RunOutcome::TimedOut {
        panic!("fixture did not terminate under the harness: {src}");
    }
    let deob = deobfuscate(src, "t.js");
    assert!(
        deob.parse_errors.is_empty(),
        "deob parse errors: {:?}",
        deob.parse_errors
    );
    let after = behavior_signature(&deob.code, budget);
    assert_eq!(
        original, after,
        "behavior changed by deobfuscation.\n--- src ---\n{src}\n--- deob ---\n{}",
        deob.code
    );
}

#[test]
fn string_array_base64_decoder_cff_program() {
    // String array (in an IIFE) + indexed/base64 decoder + a linear CFF state
    // machine + member-access obfuscation. Exercises the harness base64 polyfill.
    assert_whole_program_equivalent(
        r#"
        var _0x4f2a = (function () {
          return ['log', 'V2VsY29tZQ==', 'cGxheWVy'];
        })();
        function _0x1d(idx) { idx = idx - 0x0; return _0x4f2a[idx]; }
        var App = (function () {
          var s = 0x1, out = [];
          while (s) {
            switch (s) {
              case 1: out['push'](atob(_0x1d(0x1))); s = 2; break;
              case 2: out['push'](atob(_0x1d(0x2))); s = 0; break;
            }
          }
          return out['join'](', ');
        })();
        console['log'](App);
        "#,
    );
}

#[test]
fn object_state_dispatcher_program() {
    assert_whole_program_equivalent(
        r#"
        var ctx = { s: 1, acc: 0 };
        while (ctx.s) {
          switch (ctx.s) {
            case 1: ctx.acc += 10; ctx.s = 2; break;
            case 2: ctx.acc += 20; ctx.s = 3; break;
            case 3: ctx.acc *= 2; ctx.s = 0; break;
          }
        }
        console.log("result:", ctx.acc);
        "#,
    );
}

#[test]
fn decoders_and_value_eval_program() {
    assert_whole_program_equivalent(
        r#"
        var t = ['Hello', 'World'];
        function d(i) { return t[i]; }
        var parts = [d(0), d(1)];
        var cfg = JSON.parse('{"sep":", ","end":"!"}');
        var msg = parts.join(cfg.sep) + cfg.end;
        console.log(msg.toUpperCase());
        console.log(`len=${msg.length}`);
        "#,
    );
}

#[test]
fn single_iter_and_opaque_predicate_program() {
    assert_whole_program_equivalent(
        r#"
        var out = [];
        while (!![]) {
          if ((function () { return 7 * 7; }()) === 49) {
            out.push('ok');
          } else {
            out.push('dead');
          }
          break;
        }
        var f = new Function('a', 'b', 'return a + b');
        out.push(String(f(2, 3)));
        console.log(out.join('|'));
        "#,
    );
}

#[test]
fn throwing_program_signature_matches() {
    // A program that throws must deobfuscate to one that throws the same way.
    assert_whole_program_equivalent(
        r#"
        console.log('before');
        var x = (function () { return 10; })();
        if (x > 5) { throw new TypeError('boom'); }
        console.log('after');
        "#,
    );
}

#[test]
fn host_global_in_try_catch_is_not_misfolded() {
    // Under the host shim `window` is bound (the permissive proxy), so the `try`
    // succeeds. A deobfuscator that evaluated this in its bare sandbox — where
    // `window` is unbound — would have the `catch` swallow a sandbox-only
    // ReferenceError and fold to 99, diverging from the real host.
    assert_whole_program_equivalent(
        r#"
        var out = (function () { try { return window.someKey; } catch (e) { return 99; } })();
        console.log(typeof out);
        "#,
    );
}

#[test]
fn typeof_host_global_is_not_misfolded() {
    assert_whole_program_equivalent(r#"console.log((function () { return typeof window; })());"#);
}

#[test]
fn async_function_call_is_not_inlined() {
    // `a()` yields a Promise; folding it to the body value (21) would make
    // `a().then(…)` a `TypeError` (numbers have no `.then`) — a different outcome.
    assert_whole_program_equivalent(
        r#"
        async function a() { return 21; }
        a().then(function (v) { console.log('resolved', v * 2); });
        console.log('sync');
        "#,
    );
}

#[test]
fn generator_call_is_not_inlined() {
    // `g()` yields an iterator, not the body value.
    assert_whole_program_equivalent(
        r#"
        function* g() { return 5; }
        var it = g();
        console.log(typeof it.next);
        "#,
    );
}
