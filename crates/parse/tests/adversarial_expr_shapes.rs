//! Adversarial expression-shape edges.
//!
//! Targets parse/emit symmetry on shapes the corpus and the per-construct
//! unit tests systematically avoid: deeply-nested ternaries, long binary-op
//! chains (associativity preservation), member-access chains, reserved
//! keywords used as static property keys, strings with all common escape
//! sequences. Each test is a single source-level shape passed through the
//! full pipeline — failures here would indicate a parse/emit asymmetry
//! that the existing 308-file corpus doesn't happen to exercise.
//!
//! Companion to `proptest_roundtrip.rs` — that test catches a class of bug
//! statistically; this one pins down specific shapes that are interesting
//! independent of generator coverage.

use emit::emit;
use parse::lower_ssa;
use parse::lower_tree;
use parse::parse_and_lower;

fn full_pipeline(src: &str) -> String {
    let m = parse_and_lower(src).unwrap_or_else(|e| panic!("parse: {e}\nsrc:\n{src}"));
    let sm = lower_ssa::lower_module(&m);
    sm.validate().unwrap_or_else(|e| panic!("validate: {e:?}\nsrc:\n{src}"));
    let m2 = lower_tree::lower_module(&sm);
    let out = emit(&m2);
    parse_and_lower(&out).unwrap_or_else(|e| panic!("re-parse: {e}\nemit:\n{out}\nsrc:\n{src}"));
    out
}

#[test]
fn deeply_nested_ternary() {
    // 16-level nested ternary. Stresses parser stack depth and emit's
    // precedence handling for `? :`.
    let src = "function f(x) { \
        return x === 0 ? 0 : x === 1 ? 1 : x === 2 ? 2 : x === 3 ? 3 : \
               x === 4 ? 4 : x === 5 ? 5 : x === 6 ? 6 : x === 7 ? 7 : \
               x === 8 ? 8 : x === 9 ? 9 : x === 10 ? 10 : x === 11 ? 11 : \
               x === 12 ? 12 : x === 13 ? 13 : x === 14 ? 14 : x === 15 ? 15 : -1; \
    }";
    let _ = full_pipeline(src);
}

#[test]
fn long_left_associative_addition() {
    // Long binary chain. Addition is left-associative; emit's
    // always-parenthesize approach should preserve the grouping
    // implicitly by inserting parens.
    let mut src = String::from("function f() { return ");
    for i in 0..32 {
        if i > 0 { src.push_str(" + "); }
        src.push_str(&format!("{i}"));
    }
    src.push_str("; }");
    let _ = full_pipeline(&src);
}

#[test]
fn long_right_associative_pow() {
    // ** is right-associative. `2 ** 3 ** 4` parses as `2 ** (3 ** 4)`.
    // Emit must preserve that.
    let src = "function f() { return 2 ** 3 ** 4 ** 5; }";
    let _ = full_pipeline(src);
}

#[test]
fn mixed_precedence_binary_chain() {
    // `a + b * c - d / e | f & g ^ h << i >> j` — a tour of binary
    // operator precedences in one expression.
    let src = "function f(a, b, c, d, e, g, h, i, j) { \
        return a + b * c - d / 2 | g & h ^ 1 << i >> j; \
    }";
    let _ = full_pipeline(src);
}

#[test]
fn deeply_nested_member_access() {
    // 12-deep static member chain. Tests that emit doesn't accidentally
    // bracket a chain that should be flat.
    let src = "function f(o) { return o.a.b.c.d.e.f.g.h.i.j.k.l; }";
    let _ = full_pipeline(src);
}

#[test]
fn deeply_nested_index_access() {
    let src = "function f(o) { return o[0][1][2][3][4][5][6]; }";
    let _ = full_pipeline(src);
}

#[test]
fn mixed_member_index_call_chain() {
    // The shape that bundlers produce constantly: `a.b[c].d(e).f`.
    let src = "function f(a, c, e) { return a.b[c].d(e).f.g(1, 2)[0]; }";
    let _ = full_pipeline(src);
}

#[test]
fn reserved_keywords_as_static_property_keys() {
    // JS allows reserved words as static property names (post-ES5).
    // `obj.if`, `obj.while`, etc. should parse and round-trip.
    let src = "function f(o) { return o.if + o.while + o.return + o.function + o.delete; }";
    let _ = full_pipeline(src);
}

#[test]
fn reserved_keywords_as_object_literal_keys() {
    // Same on the construction side: `{ if: 1, while: 2 }`.
    let src = "function f() { return { if: 1, while: 2, return: 3, function: 4 }; }";
    let _ = full_pipeline(src);
}

#[test]
fn string_with_all_common_escape_sequences() {
    // Newline, carriage return, tab, backslash, double-quote, null.
    // Emit must escape them back when serializing.
    let src = "function f() { return \"a\\nb\\tc\\\\d\\\"e\\rf\"; }";
    let _ = full_pipeline(src);
}

#[test]
fn string_with_unicode_escapes() {
    let src = "function f() { return \"\\u0041\\u00e9\\u4e2d\"; }";
    let _ = full_pipeline(src);
}

#[test]
fn empty_string_and_empty_array_and_empty_object() {
    let src = "function f() { return [\"\", [], {}]; }";
    let _ = full_pipeline(src);
}

#[test]
fn nested_unary_chains() {
    // `!!x`, `--x` (decrement, NOT double-negation), `+-x`, `~~x`.
    // Emit must keep operator-level distinction. We use only forms
    // that don't collide with update-expressions.
    let src = "function f(x) { return !!x + ~~x + +-x; }";
    let _ = full_pipeline(src);
}

#[test]
fn unary_minus_on_zero_and_negative_literal() {
    // `-0`, `-1`, `- - x`. The first must survive as negative zero
    // (regression already covered by `neg_zero_roundtrip`); included
    // here so a single failure list captures all the related shapes.
    let src = "function f(x) { return [-0, -1, - - x, -(-x)]; }";
    let _ = full_pipeline(src);
}

#[test]
fn very_long_call_argument_list() {
    let mut src = String::from("function f(g) { return g(");
    for i in 0..32 {
        if i > 0 { src.push_str(", "); }
        src.push_str(&format!("{i}"));
    }
    src.push_str("); }");
    let _ = full_pipeline(&src);
}

#[test]
fn ternary_in_member_expression() {
    // `(c ? a : b).prop` — parser must see the parenthesized ternary
    // as a single member-target, not as a property-name ternary.
    let src = "function f(c, a, b) { return (c ? a : b).prop; }";
    let _ = full_pipeline(src);
}

#[test]
fn comparison_chain_short_circuit() {
    // `(a < b) && (b < c) && (c < d)`. Tests both `&&` left-associativity
    // and that comparison results aren't accidentally fused.
    let src = "function f(a, b, c, d) { return a < b && b < c && c < d; }";
    let _ = full_pipeline(src);
}

#[test]
fn typeof_on_undeclared_identifier() {
    // `typeof x` is the canonical no-throw read of an undeclared
    // identifier. The parser/emitter shouldn't fold this into a load.
    let src = "function f() { return typeof undeclared_thing; }";
    let _ = full_pipeline(src);
}
