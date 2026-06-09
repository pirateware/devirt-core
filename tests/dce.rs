//! Dead-code-elimination tests. Structural patterns only — never a specific sample.

use devirt_core::deobfuscate;

/// A function declaration whose only remaining reference is the self-memoizing
/// write inside its own body (the obfuscator string-array accessor, after every
/// call site has been inlined away) is dead and must be removed. `symbol_is_unused`
/// counts that write as a use, so DCE keys on *read* references instead.
#[test]
fn removes_self_memoizing_string_array_accessor_after_inlining() {
    // Classic shape: array accessor + indexer + rotation IIFE, calls everywhere.
    // Once `d(i)` calls are inlined to literals, `arr`/`d`/the IIFE all go dead.
    let src = r#"
        (function () {
            var arr = ["log", "Hello"];
            function d(i) { return arr[i]; }
            (function (a, b) { while (true) { if (a === b) break; arr.push(arr.shift()); } })(0, 0);
            console[d(0)](d(1));
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("console.log(\"Hello\")"), "not inlined: {out}");
    // The accessor is now unreferenced (its only call site was inlined) — gone.
    // (The rotation IIFE is a side-effecting statement and is soundly kept.)
    assert!(!out.contains("function d("), "dead accessor kept: {out}");
}

/// The bare self-memoization idiom in isolation: a function that only reassigns
/// itself and is never read. Nothing observes it, so the whole declaration is
/// inert and removable.
#[test]
fn removes_function_only_referenced_by_its_own_self_write() {
    let src = r#"
        function f() {
            var a = [1, 2, 3];
            f = function () { return a; };
            return a;
        }
        sink(1 + 1);
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(
        !out.contains("function f"),
        "dead self-writing fn kept: {out}"
    );
    assert!(out.contains("sink(2)"), "live code lost: {out}");
}

/// A bare string-literal statement in the *statement list* is inert dead code
/// (often the pure-RHS debris dead-store leaves behind), and is dropped. The
/// directive prologue lives in the body's separate `directives` field, so this
/// never touches a real `"use strict"`.
#[test]
fn drops_dead_string_literal_statement_but_keeps_directive() {
    let src = r#"
        "use strict";
        sink(1);
        "dead noise";
        sink(2);
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("\"use strict\""), "directive dropped: {out}");
    assert!(!out.contains("\"dead noise\""), "dead string kept: {out}");
    assert!(
        out.contains("sink(1)") && out.contains("sink(2)"),
        "live code lost: {out}"
    );
}

/// Soundness guard: a function that *is* read (called) elsewhere must survive,
/// even though it also self-reassigns. The read references pin it alive. Two
/// calls returning a non-constant (`g()`) keep the pipeline from folding it away,
/// isolating the DCE read-reference check we actually want to exercise.
#[test]
fn keeps_self_reassigning_function_that_is_still_called() {
    let src = r#"
        function f() {
            f = function () { return 7; };
            return g();
        }
        sink(f());
        sink(f());
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(out.contains("function f"), "live fn removed: {out}");
}

/// Regression: an *expression-bodied* arrow (`() => 7`) is stored by oxc as a
/// one-statement function body whose lone `ExpressionStatement` is the implicit
/// return. DCE's dead-literal prune must not treat that statement as discardable
/// noise — doing so emptied the arrow to the invalid `() =>`. The body's value is
/// returned, so it is kept (and folded), and the result stays syntactically valid.
#[test]
fn keeps_literal_body_of_expression_arrow() {
    let src = r#"
        window.f = () => !1;
        window.g = () => 3 + 4;
        window.h = () => !0;
        window.k = () => 7;
    "#;
    let out = deobfuscate(src, "t.js").code;
    assert!(!out.contains("=> ;") && !out.contains("=>;"), "emptied arrow body: {out}");
    assert!(out.contains("() => false"), "!1 body lost/mis-folded: {out}");
    assert!(out.contains("() => true"), "!0 body lost/mis-folded: {out}");
    assert!(out.contains("() => 7"), "numeric body lost/mis-folded: {out}");
}
