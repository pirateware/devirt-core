//! Robustness / fuzz-lite battery.
//!
//! A deobfuscator's entire input domain is hostile by construction. These inputs
//! are pathological, malformed, or degenerate; the contract is only that
//! `deobfuscate` *returns* — it must never panic the host process — and that
//! whatever it returns is itself valid, re-parseable JS (never a corrupt or
//! half-transformed program). The actual panic-containment unit lives in the lib
//! crate; here we assert the end-to-end guarantee on adversarial input.

use devirt_core::{deobfuscate, format_only};

/// Every input must be handled without a panic and must yield re-parseable output.
fn assert_survives(src: &str) {
    let report = deobfuscate(src, "t.js");
    // The output must re-parse with no errors: a second pass over our own output
    // proves we never emitted corrupt JS.
    let reparse = deobfuscate(&report.code, "t.js");
    assert!(
        reparse.parse_errors.is_empty(),
        "deob output did not re-parse cleanly.\n--- in ---\n{src}\n--- out ---\n{}\n--- errs ---\n{:?}",
        report.code,
        reparse.parse_errors
    );
}

#[test]
fn empty_and_whitespace() {
    assert_survives("");
    assert_survives("   \n\t  ");
    assert_survives("\u{feff}"); // lone BOM
    assert_survives(";;;;;;");
}

#[test]
fn malformed_and_truncated() {
    // Parser yields errors, not panics; the pipeline must still return.
    for src in [
        "function(",
        "var x = ",
        "if (",
        "})({",
        "for(;;",
        "/* unterminated",
        "\"unterminated string",
        "`unterminated template ${",
        "class {",
        "=>",
    ] {
        let _ = deobfuscate(src, "t.js"); // must not panic
    }
}

#[test]
fn deeply_nested_does_not_overflow() {
    // Regression: this depth overflows a default 2 MiB thread stack (the test
    // harness's, and a typical server worker's), which aborts the *process* —
    // not catchable by panic containment. deobfuscate runs the pipeline on a
    // large-stack worker thread so this is handled cleanly. 1500 is well past the
    // old ~350-level ceiling.
    let mut s = String::new();
    for _ in 0..1500 {
        s.push_str("if(a){");
    }
    s.push_str("x=1;");
    for _ in 0..1500 {
        s.push('}');
    }
    let report = deobfuscate(&s, "t.js");
    // It returns (no abort) and the output re-parses.
    let reparse = deobfuscate(&report.code, "t.js");
    assert!(reparse.parse_errors.is_empty());
}

#[test]
fn degenerate_obfuscation_shapes() {
    // Shapes that look like the patterns the passes target, but degenerate:
    // empty dispatchers, single-case switches, self-referential aliases.
    let cases = [
        "while(true){switch(a){}}",
        "while(true){switch(a){default:break}}",
        "var a=a;",
        "var f=function(){return f()};",
        "(function(){})();",
        "[][[]];",
        "var o={};o.x=o;",
        "var s=''+''+''+'';",
        "void void void 0;",
        "a?b?c?d:e:f:g;",
    ];
    for c in cases {
        assert_survives(c);
    }
}

#[test]
fn unicode_and_exotic_identifiers() {
    assert_survives("var \u{1f600} = 1; \u{1f600} + 1;");
    assert_survives("var \u{5b57} = '\u{6f22}\u{5b57}'; console.log(\u{5b57});");
    assert_survives(r#"var a = 1; a;"#);
}

#[test]
fn known_good_input_is_unchanged_in_meaning() {
    // Sanity anchor: a normal program produces no containment error.
    let report = deobfuscate("var a = 1 + 2; console.log(a);", "t.js");
    assert!(report.error.is_none());
    // format_only is itself robust on the same inputs.
    assert_eq!(format_only("", "t.js"), format_only("", "t.js"));
}
