//! Phase 1 behavioral tests + the cross-cutting safety invariant (idempotence).
//! These assert on *behavior shape*, never on a specific sample — the pipeline
//! must stay sample-agnostic.

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

#[test]
fn member_normalization() {
    let out = deob(r#"a["foo"]; a["1bad"]; a["with"]();"#);
    assert!(out.contains("a.foo"));
    assert!(out.contains("a.with()"));
    assert!(out.contains(r#"a["1bad"]"#)); // invalid identifier untouched
}

#[test]
fn member_normalization_optional_chain() {
    // `a?.["foo"]` parses as a ChainElement, not an Expression, so it needs a
    // dedicated hook. The object is an opaque param so the access survives.
    let out = deob(r#"function f(a){ return a?.["foo"]?.["1bad"]; } sink(f);"#);
    assert!(out.contains("?.foo"), "optional chain not normalized: {out}");
    // non-identifier key stays bracketed
    assert!(
        out.contains(r#"?.["1bad"]"#),
        "invalid-ident optional key wrongly converted: {out}"
    );
}

#[test]
fn member_normalization_on_assignment_target() {
    // The LHS of an assignment is a SimpleAssignmentTarget, not an Expression;
    // it must normalize just like read-position member access.
    // (param name may be renamed; assert on the access form, not the name.)
    let out = deob(r#"function g(o){ o["foo"] = o["foo"] || 1; return o; } sink(g);"#);
    assert!(out.contains(".foo = "), "LHS not normalized: {out}");
    assert!(
        !out.contains(r#"["foo"]"#),
        "bracket access remained: {out}"
    );
}

#[test]
fn assigned_once_inline_and_fold() {
    let out = deob("function g(){ var n = (0x10, 256); return 5 + n; } sink(g);");
    assert!(out.contains("261"), "got: {out}");
    assert!(!out.contains("var n"), "declaration should be DCE'd: {out}");
}

#[test]
fn string_concatenation_folds() {
    let out = deob(r#"sink("foo" + "bar" + "baz", "n" + 5);"#);
    assert!(
        out.contains(r#""foobarbaz""#),
        "string concat not folded: {out}"
    );
    assert!(
        out.contains(r#""n5""#),
        "mixed string+number concat not folded: {out}"
    );
}

#[test]
fn void_literal_folds_to_undefined() {
    let out = deob("f(void 0, a === void 0);");
    assert!(out.contains("f(undefined, a === undefined)"), "got: {out}");
    assert!(!out.contains("void 0"), "void 0 survived: {out}");
}

#[test]
fn void_not_folded_when_undefined_is_shadowed() {
    // A local binding named `undefined` shadows the global; rewriting `void 0`
    // to `undefined` here would be unsound.
    let out = deob("function f(undefined){ return void 0; } sink(f);");
    assert!(
        out.contains("void 0"),
        "unsoundly folded shadowed undefined: {out}"
    );
}

#[test]
fn constant_ternary_folds_to_taken_branch() {
    let out = deob("f(true ? 10 : 0, false ? a : b());");
    assert!(
        out.contains("f(10, b())"),
        "constant ternary not folded: {out}"
    );
}

#[test]
fn opaque_predicates() {
    let out = deob("if (!![]) { keep(); } if (![]) { drop(); }");
    assert!(out.contains("keep()"));
    assert!(!out.contains("drop()"), "dead branch survived: {out}");
}

#[test]
fn dead_literal_statements_removed() {
    let out = deob(r#"function g(){ 80214130; true; null; keep(); } sink(g);"#);
    assert!(out.contains("keep()"), "real statement dropped: {out}");
    assert!(
        !out.contains("80214130"),
        "dead numeric literal survived: {out}"
    );
    assert!(
        !out.contains("true;") && !out.contains("null;"),
        "dead literal survived: {out}"
    );
}

#[test]
fn unreachable_after_return() {
    let out = deob("function g(){ return 1; sideEffect(); } sink(g);");
    assert!(
        !out.contains("sideEffect"),
        "unreachable code survived: {out}"
    );
}

#[test]
fn reassigned_var_folds_to_last_dominating_value() {
    // The SSA dataflow pass resolves `return x` to its single reaching
    // definition (`x = 2` dominates the return), so the read folds to 2 and the
    // dead assignments are removed. assigned-once alone bailed on the
    // reassignment; SSA makes this sound by construction.
    let out = deob("function g(){ var x = 1; x = 2; return x; } sink(g);");
    assert!(
        out.contains("return 2"),
        "should fold to the dominating value: {out}"
    );
}

#[test]
fn ambiguous_reassignment_not_folded() {
    // The reaching definition at the return is a phi (1 on the fall-through
    // path, 2 on the branch), so it must NOT fold to either literal.
    let out = deob("function g(c){ var x = 1; if (c) { x = 2; } return x; }");
    assert!(
        !out.contains("return 1") && !out.contains("return 2"),
        "merge wrongly folded: {out}"
    );
}

#[test]
fn nested_const_array_2d_resolves_to_literals() {
    // B2: a provably-immutable 2-D const array read `T[i][j]`, including the
    // cross-function aliased shape (`T = base[k]`), resolves the labels to
    // literals so the dispatcher becomes a plain numeric switch.
    let out = deob(
        "(function(){ \
           var base = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]; \
           var T = base[1]; \
           function run(s){ var acc = 0; while (s != 0) { switch (s) { \
             case T[0][0]: acc += 1; s = T[0][1]; break; \
             case T[1][1]: acc += 4; s = 0; break; } } return acc; } \
           sink(run); })();",
    );
    assert!(
        out.contains("case 5") && out.contains("case 8"),
        "nested/aliased const-array labels not resolved: {out}"
    );
}

#[test]
fn member_written_table_labels_stay_symbolic() {
    // B2 adversarial sibling (sample_11's table shape): the table is built by
    // member writes (`base[i][j] = …`) — i.e. *mutable* — so its element reads
    // must NOT be resolved to literals. `mutable_aggregate_tables` is the gate.
    let out = deob(
        "(function(){ \
           var base = []; \
           for (var i = 0; i < 2; i++) base[i] = new Array(2); \
           base[0][0] = 5; base[0][1] = 6; \
           var T = base[0]; \
           function run(s){ var acc = 0; while (s != 0) { switch (s) { \
             case T[0]: acc += 1; s = T[1]; break; \
             case T[1]: acc += 2; s = 0; break; } } return acc; } \
           sink(run); })();",
    );
    // The mutable table's element reads survive as member accesses; no numeric
    // `case 5`/`case 6` literal appears (that would be an unsound static read of a
    // member-written table).
    assert!(
        !out.contains("case 5") && !out.contains("case 6"),
        "member-written table was unsoundly resolved: {out}"
    );
}

#[test]
fn conditional_nested_init_constant_stays_symbolic() {
    // B1 wall (sample_1): a label constant written by an init function that runs
    // inside a conditionally-reached nested dispatcher case is NOT setup-complete
    // (the dispatcher's state-reachability is not statically proven), so it must
    // stay symbolic. Inlining it would be unsound — a read on an earlier case can
    // observe the pre-init `undefined`.
    let out = deob(
        "(function(){ \
           var L; var out = []; \
           function disp(s){ \
             while (s) { switch (s) { \
               case 1: out.push('' + L); s = 2; break; \
               case 2: init(); s = 3; break; \
               case 3: out.push('' + L); s = 0; break; } } \
             function init(){ L = 9; } } \
           disp(1); sink(out.join(',')); })();",
    );
    // The init write survives (constant not inlined / write not DCE'd).
    assert!(
        out.contains("= 9"),
        "conditional nested-init constant was unsoundly inlined: {out}"
    );
}

#[test]
fn idempotent_on_nested_obfuscation() {
    let src = "function f(){ var a = 1, b = 2; if (!!1) return a + b; }";
    let once = deobfuscate(src, "t.js").code;
    let twice = deobfuscate(&once, "t.js").code;
    assert_eq!(once, twice, "pipeline is not idempotent");
}

#[test]
fn object_reconstruction_folds_split_writes() {
    // `transformObjectKeys` lowers `{a,b}` into `o = {}; o.a = …; o.b = …`.
    // Reconstruction folds the contiguous writes back into the literal.
    let out = deob(r#"function f(){ var o = {}; o.a = "x"; o.b = "y"; return o; } sink(f);"#);
    assert!(out.contains(r#"a: "x""#), "not reconstructed: {out}");
    assert!(out.contains(r#"b: "y""#), "not reconstructed: {out}");
    assert!(!out.contains(".a ="), "split write survived: {out}");
}

#[test]
fn object_reconstruction_exposes_proxy_table() {
    // An operator-proxy table built in split-write form is invisible to
    // proxy-inline until reconstructed; once folded, the call inlines and folds.
    let out =
        deob(r#"function f(){ var t = {}; t.sum = function(a,b){ return a + b; }; return t.sum(2,3); } sink(f);"#);
    assert!(out.contains('5'), "proxy table not inlined+folded: {out}");
    assert!(!out.contains("t.sum"), "proxy survived: {out}");
}
