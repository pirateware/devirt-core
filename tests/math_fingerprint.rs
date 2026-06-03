//! Math-proxy fingerprinting tests. Structural patterns only — never a specific
//! sample. The identities used (`(a^b)+2*(a&b) == a+b`, `a+~b+1 == a-b`) are real
//! integer identities, so a correct fingerprint must recover the operator.

use js_deobfuscator::deobfuscate;

fn deob(src: &str) -> String {
    let r = deobfuscate(src, "t.js");
    assert!(
        r.parse_errors.is_empty(),
        "parse errors: {:?}",
        r.parse_errors
    );
    r.code
}

#[test]
fn collapses_addition_polynomial() {
    // The full-adder identity for `a + b`. Every operand use is bitwise, so the
    // proxy is `ToInt32(a) + ToInt32(b)` and collapses to the coercion-faithful
    // `(x | 0) + (y | 0)`, which stays correct even for non-integer arguments
    // (a bare `x + y` would not — `add("a","b")` is 0, not "ab").
    let src = r#"
        (function () {
            function add(a, b) { return (a ^ b) + 2 * (a & b); }
            sink(add(x, y));
        })();
    "#;
    let out = deob(src);
    assert!(
        out.contains("sink((x | 0) + (y | 0))"),
        "addition proxy not collapsed to the coerced form: {out}"
    );
    assert!(!out.contains("add("), "dead proxy not removed: {out}");
}

#[test]
fn collapses_subtraction_and_keeps_operand_order() {
    // Full-subtractor identity `(a^b) - 2*(~a&b) == a - b`, fully bitwise so it is
    // `ToInt32(a) - ToInt32(b)` and collapses to `(x | 0) - (y | 0)`. Non-
    // commutative: operands must stay (x, y), not swap. (A raw-operand form like
    // `a + ~b + 1` is only `a - b` for integers — e.g. `sub("5",2)` is "5-31" —
    // and the pass now correctly refuses to collapse it.)
    let src = r#"
        (function () {
            function sub(a, b) { return (a ^ b) - 2 * (~a & b); }
            sink(sub(x, y));
        })();
    "#;
    let out = deob(src);
    assert!(
        out.contains("sink((x | 0) - (y | 0))"),
        "subtraction proxy mis-collapsed: {out}"
    );
}

#[test]
fn collapses_swapped_subtraction() {
    // Body computes `b - a`; the call `r(x, y)` must become `y - x`.
    let src = r#"
        (function () {
            function r(a, b) { return b - a; }
            sink(r(x, y));
        })();
    "#;
    let out = deob(src);
    assert!(
        out.contains("sink(y - x)"),
        "swapped subtraction wrong: {out}"
    );
}

#[test]
fn collapses_xor_and_bitand() {
    let src = r#"
        (function () {
            function x(a, b) { return (a | b) - (a & b); }
            function n(a, b) { return ~(~a | ~b); }
            sink(x(p, q));
            sink(n(p, q));
        })();
    "#;
    let out = deob(src);
    // (a|b)-(a&b) == a^b ; ~(~a|~b) == a&b
    assert!(
        out.contains("sink(p ^ q)"),
        "xor proxy not collapsed: {out}"
    );
    assert!(
        out.contains("sink(p & q)"),
        "and proxy not collapsed: {out}"
    );
}

#[test]
fn collapses_unary_bitwise_not() {
    // `-(a|0) - 1 == ~a` for all inputs (the `|0` coerces, matching `~`'s own
    // ToInt32). A raw `-a - 1` is only `~a` for numbers — `bnot("a")` is NaN, not
    // -1 — so the pass now refuses to collapse the raw form.
    let src = r#"
        (function () {
            function bnot(a) { return -(a | 0) - 1; }
            sink(bnot(z));
        })();
    "#;
    let out = deob(src);
    assert!(
        out.contains("sink(~z)"),
        "unary bitwise-not proxy not collapsed: {out}"
    );
}

// --- Adversarial: must NOT replace ---------------------------------------

#[test]
fn does_not_replace_near_miss_on_negatives() {
    // Equals `a + b` only for non-negative `a`; the battery includes negatives,
    // so this must never be mistaken for `+`.
    let src = r#"
        (function () {
            function f(a, b) { return a < 0 ? a : a + b; }
            sink(f(x, y));
        })();
    "#;
    let out = deob(src);
    // Must not become plain `x + y`; the value-distinguishing conditional survives.
    assert!(
        !out.contains("sink(x + y)"),
        "near-miss proxy was wrongly collapsed: {out}"
    );
    assert!(out.contains("x < 0 ? x"), "near-miss logic lost: {out}");
}

#[test]
fn does_not_replace_impure_function() {
    // Body has a member write — not arithmetic-shaped, so never a candidate, and
    // its side effect must survive.
    let src = r#"
        (function () {
            function f(a, b) { window.tag = a; return a + b; }
            sink(f(x, y));
        })();
    "#;
    let out = deob(src);
    // The member-write side effect must survive and the call must not collapse.
    assert!(
        out.contains("window.tag"),
        "impure side effect dropped: {out}"
    );
    assert!(
        !out.contains("sink(x + y)"),
        "impure call was wrongly collapsed: {out}"
    );
}

#[test]
fn does_not_replace_reassigned_name() {
    // `add` is reassigned, so a call may not bind to the body we fingerprinted.
    let src = r#"
        (function () {
            function add(a, b) { return (a ^ b) + 2 * (a & b); }
            if (cond) { add = realAdd; }
            sink(add(x, y));
        })();
    "#;
    let out = deob(src);
    assert!(
        out.contains("add("),
        "reassigned proxy was wrongly collapsed: {out}"
    );
}

#[test]
fn does_not_reorder_effectful_args_for_swapped_op() {
    // Swapped subtraction with side-effecting args would reorder g()/h(); the
    // pass must decline and leave the call intact.
    let src = r#"
        (function () {
            function r(a, b) { return b - a; }
            sink(r(g(), h()));
        })();
    "#;
    let out = deob(src);
    // Argument order g() then h() must be preserved; never rewritten to `h() - g()`.
    assert!(
        out.contains("(g(), h())"),
        "effectful swapped args were reordered: {out}"
    );
    assert!(
        !out.contains("h() - g()"),
        "effectful swapped args were reordered: {out}"
    );
}

#[test]
fn leaves_genuine_decoder_alone() {
    // A string-table decoder is not arithmetic-shaped; fingerprinting must ignore
    // it (decoder-lift handles it). It must still be lifted, not mangled.
    let src = r#"
        (function () {
            var arr = ["lo", "HELLO", "wo"];
            function d(i) { return arr[i]; }
            sink(d(1));
        })();
    "#;
    let out = deob(src);
    assert!(
        out.contains("\"HELLO\""),
        "decoder not lifted / mangled: {out}"
    );
}
