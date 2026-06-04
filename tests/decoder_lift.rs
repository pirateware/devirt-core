//! Decoder-lifting tests. Structural patterns only — never a specific sample.

use js_deobfuscator::deobfuscate;

#[test]
fn lifts_string_array_decoder_with_rotation() {
    // Classic shape: array + indexer + (no-op) rotation IIFE, calls everywhere.
    let src = r#"
        (function () {
            var arr = ["log", "Hello", "warn"];
            function d(i) { return arr[i]; }
            (function (a, b) { while (true) { if (a === b) break; arr.push(arr.shift()); } })(0, 0);
            console[d(0)](d(1));
            console[d(2)]("x");
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("console.log(\"Hello\")"), "got: {out}");
    assert!(out.contains("console.warn(\"x\")"), "got: {out}");
}

#[test]
fn numeric_decoder_result_stays_numeric() {
    // A decoder that returns a *number* must be spliced as a numeric literal,
    // not a string — otherwise `dec(i) + 1` flips from arithmetic to concat.
    let src = r#"
        (function () {
            var tbl = [10, 20, 30];
            function d(i) { return tbl[i]; }
            sink(d(1) + 1);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // 20 + 1 must fold to 21 (numeric), never "201" (string concat) or "20".
    assert!(
        out.contains("sink(21)"),
        "numeric decode mis-typed as string: {out}"
    );
    assert!(
        !out.contains("\"20\""),
        "spliced a string literal for a number: {out}"
    );
}

#[test]
fn does_not_lift_impure_decoder() {
    // "Decoder" touches the host (window) → must not be evaluated/lifted.
    let src = r#"
        (function () {
            function d(i) { return window.secret[i]; }
            foo(d(0));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The host access must survive and no string literal may replace the call.
    assert!(
        out.contains("window") && out.contains("secret"),
        "impure decoder body lost: {out}"
    );
    assert!(
        !out.contains("\"\"") && !out.contains("foo(\""),
        "impure call was wrongly lifted: {out}"
    );
}

#[test]
fn does_not_lift_mutating_library_calls() {
    // `arr.push(arr.shift())` mutates a library array; it must NOT be folded to
    // its (length-preserving, deterministic) return value, which would destroy
    // the rotation and corrupt every subsequent decode.
    let src = r#"
        (function () {
            var arr = ["log", "Hello", "warn"];
            function d(i) { return arr[i]; }
            (function (a, b) { while (true) { if (a === b) break; arr.push(arr.shift()); } })(1, 1);
            console[d(0)](d(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("arr.push(arr.shift())"),
        "rotation was wrongly lifted: {out}"
    );
    assert!(
        out.contains("console.log(\"Hello\")"),
        "decode corrupted: {out}"
    );
}

#[test]
fn lifts_computed_index_into_pure_array() {
    // `tbl[K.a]` is a pure read into a library array — safe to lift.
    let src = r#"
        (function () {
            var tbl = ["alpha", "beta", "gamma"];
            var K = { a: 0, b: 2 };
            sink(tbl[K.a], tbl[K.b]);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("\"alpha\"") && out.contains("\"gamma\""),
        "index read not lifted: {out}"
    );
}

#[test]
fn lifts_decoder_nested_in_a_function_scope() {
    // The Tier-C/D shape: the decoder + table live inside a *function body*, which
    // `collect_top_level` never descends into (unlike an expression-statement
    // IIFE). Scoped lifting must still resolve `dec(K)` to its literal.
    let src = r#"
        function setup() {
            var tbl = ["alpha", "beta", "gamma"];
            function dec(i) { return tbl[i]; }
            sink(dec(0), dec(2));
        }
        setup();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("\"alpha\"") && out.contains("\"gamma\""),
        "nested decoder not lifted: {out}"
    );
    assert!(
        !out.contains("dec("),
        "decoder call survived in nested scope: {out}"
    );
}

#[test]
fn lifts_decoder_nested_in_var_assigned_iife() {
    // Exactly the DataDome wall: the library lives inside `var X = (() => {…})()`,
    // a VariableDeclaration whose initializer `collect_top_level` cannot reach.
    let src = r#"
        var R = (function () {
            var tbl = ["createElement", "appendChild"];
            function dec(i) { return tbl[i]; }
            return win[dec(0)](dec(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("win.createElement"),
        "computed member not resolved: {out}"
    );
    assert!(
        out.contains("\"appendChild\""),
        "argument decode not lifted: {out}"
    );
    assert!(!out.contains("dec("), "decoder call survived: {out}");
}

#[test]
fn does_not_lift_scoped_decoder_over_member_mutated_table() {
    // The hazard scoped lifting widens (plan §5.3 / §8): the table is mutated by a
    // member write, which the name-based "assigned once" count is blind to. The
    // symbol-precise escape gate must refuse to bake it, leaving the decoder calls
    // in place rather than resolving them against a stale table.
    let src = r#"
        function run() {
            var tbl = ["a", "b", "c"];
            function dec(i) { return tbl[i]; }
            tbl[1] = "Z";
            sink(dec(0), dec(1));
        }
        run();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The decoder calls must survive unresolved: the table is mutated, so a baked
    // ["a","b","c"] would mis-resolve dec(1) (whose slot is overwritten to "Z").
    assert!(
        out.contains("dec(0)") && out.contains("dec(1)"),
        "decoder over a member-mutated table must be left in place: {out}"
    );
    assert!(
        out.contains("tbl[1] = \"Z\""),
        "the mutation must survive: {out}"
    );
}

#[test]
fn lifts_memoized_private_cache_decoder() {
    // The sample_7 shape: a string-array decoder wrapped in a private memo cache.
    // `ft` writes the free `fu` (its own cache), which the base purity gate rejects;
    // the private-cache relaxation must recognise the memo idiom and lift `ft(k)` to
    // the literal it computes, then DCE removes the spent decoder/table/cache.
    let src = r#"
        (function () {
            var fu = {};
            var fv = ["alpha", "beta", "gamma"];
            function fs(s) { return s.toUpperCase(); }
            function ft(t) { return void 0 === fu[t] ? (fu[t] = fs(fv[t])) : fu[t]; }
            sink(ft(0), ft(2), ft(0));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The memo decode fully resolves (warm-cache repeat included) and the spent
    // scaffolding — decoder body, string table, cache — is gone after DCE.
    assert!(
        out.contains(r#"sink("ALPHA", "GAMMA", "ALPHA")"#),
        "memo decode not fully lifted: {out}"
    );
    assert!(
        !out.contains("toUpperCase") && !out.contains("==="),
        "spent memo decoder/table not removed: {out}"
    );
}

#[test]
fn does_not_lift_memo_decoder_with_observed_cache() {
    // The cache is read outside the decoder (`probe = fu[0]`), so populating it is
    // an observable side effect. The decoder calls must be left in place rather than
    // resolved against a cache that the lift would never warm.
    let src = r#"
        (function () {
            var fu = {};
            var fv = ["alpha", "beta"];
            function fs(s) { return s.toUpperCase(); }
            function ft(t) { return void 0 === fu[t] ? (fu[t] = fs(fv[t])) : fu[t]; }
            var probe = fu[0];
            sink(ft(0), ft(1), probe);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The cache is observed outside the decoder, so it must NOT lift: the string
    // table stays un-decoded (lowercase, not "ALPHA") and the decoder body survives.
    assert!(
        out.contains("\"alpha\"") && !out.contains("\"ALPHA\""),
        "decoder over an observed cache must be left in place (un-decoded): {out}"
    );
    assert!(
        out.contains("toUpperCase"),
        "decoder body must survive when the cache is observed: {out}"
    );
}

#[test]
fn lifts_through_bind_accessor_alias() {
    // C3: a decoder aliased through `.bind()` (`g = dec.bind(null)`). `dec` is a
    // pure, `this`-free decoder, so `g(i)` is observationally `dec(i)` and the call
    // through the bound alias must lift to the decoded literal.
    let src = r#"
        (function () {
            var tbl = ["alpha", "beta", "gamma"];
            function dec(i) { return tbl[i]; }
            var g = dec.bind(null);
            sink(g(0), g(2));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("\"alpha\"") && out.contains("\"gamma\""),
        "bound accessor-alias decode not lifted: {out}"
    );
    assert!(!out.contains("g("), "bound alias call survived: {out}");
}

#[test]
fn does_not_lift_reassigned_bind_alias() {
    // Adversarial: the bound alias `g` is reassigned to something else, so a baked
    // `g = dec.bind(null)` would be stale at the call site. The `unstable` gate must
    // drop it and leave the calls in place.
    let src = r#"
        (function () {
            var tbl = ["alpha", "beta", "gamma"];
            function dec(i) { return tbl[i]; }
            var g = dec.bind(null);
            g = other;
            sink(g(0), g(2));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The alias is reassigned, so it must not be lifted: the calls must NOT be
    // decoded into `sink("alpha", "gamma")`, and the reassignment must survive.
    assert!(
        !out.contains(r#"sink("alpha", "gamma")"#),
        "reassigned bound alias must not be lifted: {out}"
    );
    assert!(out.contains("other"), "reassignment lost: {out}");
}

#[test]
fn removes_nested_spent_rotation_iife() {
    // C4: the Cloudflare shape — the array rotation IIFE is *nested* inside a larger
    // wrapper IIFE, and the rotated table holder (`hold`) is read ONLY by the
    // rotation (every decode site was already lifted). The rotation is then
    // unobservable and must be removed; the next DCE round drops `hold`/`arr`.
    let src = r#"
        (function () {
            var arr = ["log", "Hello", "warn"];
            function hold() { var a = arr; hold = function () { return a; }; return hold(); }
            (function (wrap) {
                (function (get, k, list) {
                    for (list = get(); true;) {
                        if (k === 0) break;
                        list.push(list.shift());
                    }
                })(hold, 0);
                console[d(0)](d(1));
            })();
            function d(i) { return arr[i]; }
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The decode resolves and the nested rotation (push/shift) must be gone.
    assert!(
        out.contains("console.log(\"Hello\")"),
        "decode not lifted: {out}"
    );
    assert!(
        !out.contains("push(") && !out.contains("shift("),
        "nested spent rotation IIFE was not removed: {out}"
    );
}

#[test]
fn keeps_rotation_when_table_still_read() {
    // Adversarial: the table holder is still read by live application code
    // (`escape(hold)`), so the rotation is observable and MUST be kept — removing it
    // would change the rotated array the live read observes.
    let src = r#"
        (function () {
            var arr = ["log", "Hello", "warn"];
            function hold() { return arr; }
            (function (wrap) {
                (function (get, k, list) {
                    for (list = get(); true;) {
                        if (k === 0) break;
                        list.push(list.shift());
                    }
                })(hold, 0);
                window.leak = hold;
            })();
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // `hold` escapes to the host, so the rotation that mutates its array is
    // observable and must survive.
    assert!(
        out.contains("push(") && out.contains("shift("),
        "rotation over a still-read table must be kept: {out}"
    );
}

#[test]
fn lifts_small_array_returning_decoder() {
    // B3: a pure decoder that returns a small literal array materializes to an
    // array literal (the EvalValue::Arr splice). `unpack` builds `[1,2,3]` from a
    // packed string; the call must become `[1, 2, 3]`.
    let src = r#"
        (function () {
            function unpack(s) { return s.split(",").map(function (x) { return +x; }); }
            sink(unpack("1,2,3"));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("[1, 2, 3]") || out.contains("[\n\t\t1,"),
        "small array decoder not materialized: {out}"
    );
    assert!(
        !out.contains("unpack("),
        "array decoder call survived: {out}"
    );
}

#[test]
fn does_not_materialize_huge_array() {
    // B3 bloat cap: a pure decoder returning a large array must NOT be expanded
    // (materializing thousands of ints as a literal is net-negative — issues/09).
    // The call must survive rather than splice a giant array literal.
    let src = r#"
        (function () {
            function build(n) { var a = []; for (var i = 0; i < n; i++) a.push(i); return a; }
            sink(build(500));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("build(500)"),
        "huge array must not be materialized (cap): {out}"
    );
}

#[test]
fn does_not_lift_decoder_through_this_global_detection() {
    // B3/C2 sound decline (TIGHTENED): the global-object idiom (`this` /
    // `new Function("return this")()`) is now modeled as the frozen sandbox global
    // and is admissible *only where pure-relative-to-host*. A decoder that detects
    // the global and then reads a genuinely *non-deterministic* value off it
    // (`Math.random()`) must NOT lift — the double-eval determinism guard (two
    // back-to-back evals must agree) refuses it. (The earlier `g.TextDecoder` form
    // was wrong as an adversary: under QuickJS-relative soundness the host-property
    // branch is unreachable in the sandbox and both sides agree, so the
    // global-detector idiom itself is a *legitimate* lift — see
    // `lifts_decoder_through_pure_global_detection`. The real must-decline is a
    // genuinely non-deterministic value, locked in here.)
    let src = r#"
        (function () {
            var tbl = ["AAAA", "BBBB"];
            function pick(i) {
                var g = (function () { try { return new Function("return this")(); } catch (e) {} return this; })();
                var rnd = g.Math.random() < 2 ? g.Math.random() : 0;
                return tbl[i] + rnd;
            }
            sink(pick(0), pick(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // The decoder chain must survive unresolved (the random read is non-deterministic,
    // so the determinism guard declines); no decode literal may replace the calls.
    assert!(
        out.contains("pick(0)") && out.contains("pick(1)"),
        "non-deterministic decoder must be left in place: {out}"
    );
}

#[test]
fn does_not_lift_impure_call_preamble_binding() {
    // B3 adversarial: a decoder whose value derives from an *impure* call (one that
    // mutates outer state) must NOT be lifted. `bump` increments a shared counter —
    // lifting a decode that consumes its result would drop that side effect. The
    // decoder must stay in place. (`bump` writes a free name, so it never classifies
    // pure; `tick`/`dec` are poisoned transitively.)
    let src = r#"
        (function () {
            var counter = 0;
            function bump() { counter++; return counter; }
            var tick = bump();
            var tbl = ["a", "b", "c"];
            function dec(i) { return tbl[(i + tick) % 3]; }
            sink(dec(0));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("dec(0)"),
        "decoder over an impure-call binding must be left in place: {out}"
    );
}

#[test]
fn lifts_decoder_through_pure_global_detection() {
    // B3 global-object idiom used *pure-relative-to-host*. The decoder detects the
    // global via the standard idiom (`new Function("return this")()` / `this`) and
    // reads only a *standard built-in* (`String.fromCharCode`) off it — deterministic
    // and identical on every conforming host, and present in the QuickJS sandbox. So
    // it is a legitimate lift: `this`/`Function("return this")()` are modeled as the
    // frozen sandbox global rather than poisoning the purity gate, and the decoder
    // resolves under the double-eval determinism guard.
    let src = r#"
        (function () {
            var tbl = [72, 73];
            function dec(i) {
                var g = (function () { try { return new Function("return this")(); } catch (e) {} return this; })();
                var SC = (g && g.String) || String;
                return SC.fromCharCode(tbl[i]);
            }
            sink(dec(0), dec(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("\"H\"") && out.contains("\"I\""),
        "pure global-detector decoder not resolved: {out}"
    );
    assert!(!out.contains("dec("), "decoder call survived: {out}");
}

#[test]
fn lifts_object_keyed_arguments() {
    // Argument is a member of a pure object literal — still fully evaluable.
    let src = r#"
        (function () {
            var K = { a: 0, b: 1 };
            var tbl = ["alpha", "beta"];
            function d(i) { return tbl[i]; }
            sink(d(K.a), d(K.b));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        out.contains("\"alpha\"") && out.contains("\"beta\""),
        "got: {out}"
    );
}
