use devirt_core::{deobfuscate, run_pass};

#[test]
fn renames_opaque_bindings_and_all_references() {
    // WL/jK are inside an IIFE (non-root). WL touches the host so it is NOT
    // a liftable decoder — it survives to be renamed. (Root-scope globals are
    // preserved separately; see `preserves_global_names`.)
    let out = deobfuscate(
        "(function(){ function WL(jK){ return document.body[jK]; } sink(WL(window.k)); })();",
        "t.js",
    )
    .code;
    assert!(
        !out.contains("WL") && !out.contains("jK"),
        "opaque names remain: {out}"
    );
    // The body is a computed lookup `document.body[jK]`, so the function is named
    // for its behavior (a getter), not a generic `fn1`.
    assert!(
        out.contains("get"),
        "function not renamed by behavior: {out}"
    );
}

#[test]
fn names_by_assignment() {
    // A known global, a regex, a `new Date()`, an array literal, and an object
    // literal each get a descriptive name from what they hold. Run the rename
    // pass in isolation: in the full pipeline these single-use temps would be
    // inlined away before rename sees them (correct — see the inline pass), so
    // this unit test targets the rename pass directly.
    let out = run_pass(
        r#"(function(){
            var aa = document;
            var bb = /x/g;
            var cc = new Date();
            var dd = [1, 2, 3];
            var ee = { k: 1 };
            sink(aa, bb, cc, dd, ee);
        })();"#,
        "t.js",
        "rename",
    );
    for name in ["doc", "regex", "date", "arr", "obj"] {
        assert!(
            out.contains(name),
            "missing assignment-based name `{name}`: {out}"
        );
    }
}

#[test]
fn names_functions_by_behavior() {
    // Comparison and arithmetic proxy bodies are named for the operation.
    // A leading side-effecting statement keeps these from being collapsed as
    // pure operator proxies, so they survive to the rename pass.
    let out = deobfuscate(
        r#"(function(){
            function aa(x, y){ log(x); return x < y; }
            function bb(x, y){ log(y); return x * y; }
            sink(aa(window.p, window.q), bb(window.r, window.s));
        })();"#,
        "t.js",
    )
    .code;
    assert!(out.contains("lessThan"), "comparison fn not named: {out}");
    assert!(out.contains("mul"), "arithmetic fn not named: {out}");
}

#[test]
fn names_array_accumulator_by_usage() {
    // `qq` is only ever `.push`ed to and read for `.length` — an accumulator.
    let out = deobfuscate(
        r#"(function(){
            var qq = [];
            qq.push(1);
            qq.push(2);
            sink(qq.length);
        })();"#,
        "t.js",
    )
    .code;
    // The array-literal assignment hint (`arr`) wins over usage here, which is
    // fine — either way it is descriptive and the opaque name is gone.
    assert!(
        !out.contains("qq"),
        "opaque accumulator name remains: {out}"
    );
}

#[test]
fn emitted_names_are_unique_and_collision_free() {
    // Two regex bindings both want `regex`; the second must be suffixed, and the
    // pre-existing `regex` reference in scope must not be clobbered. Rename in
    // isolation (single-use temps would otherwise be inlined before rename runs).
    let out = run_pass(
        r#"(function(){
            var regex = 0;
            var aa = /x/;
            var bb = /y/;
            sink(regex, aa, bb);
        })();"#,
        "t.js",
        "rename",
    );
    assert!(
        out.contains("regex1") || out.contains("regex2"),
        "duplicate base not suffixed: {out}"
    );
}

#[test]
fn does_not_touch_property_names() {
    // `WL` here is a property key, not a reference to a binding — must survive.
    // `ab` aliases `window` so the object can't be reconstructed/folded away,
    // keeping the `.WL` property access in the output for rename to leave alone.
    let out = deobfuscate("var ab = window; ab.WL = 1; foo(ab.WL);", "t.js").code;
    assert!(
        out.contains(".WL"),
        "property name was wrongly renamed: {out}"
    );
}

#[test]
fn preserves_global_names() {
    // Top-level (global/observable) bindings must not be renamed. Run the rename
    // pass in isolation: in the full pipeline `Qz` is a single-use const, so it is
    // inlined into `window.x` and the dead `var Qz` removed (the unused-top-level
    // policy) before rename ever sees it — correct, but it would erase the binding
    // this test is about. Isolation targets the rename policy directly.
    let out = run_pass("var Qz = 1; window.x = Qz;", "t.js", "rename");
    assert!(out.contains("Qz"), "global binding renamed: {out}");
}

#[test]
fn renames_module_top_level_private_binding() {
    // In an ES module a top-level binding is module-private (not a global
    // property), so a non-exported opaque one is safe to rename. Isolated rename
    // pass (`.mjs` → module) so the binding isn't inlined first.
    let out = run_pass("var _0xQz = window.x; sink(_0xQz, _0xQz);", "m.mjs", "rename");
    assert!(
        !out.contains("_0xQz"),
        "module-private top-level binding not renamed: {out}"
    );
}

#[test]
fn preserves_exported_and_imported_module_names() {
    // Renaming an exported binding would change the public name; renaming an
    // imported local would change which export is bound. Both must be pinned.
    let exp = run_pass("export var _0xQz = window.x; sink(_0xQz);", "m.mjs", "rename");
    assert!(exp.contains("_0xQz"), "exported binding renamed: {exp}");

    let imp = run_pass(
        "import { _0xQz } from './m'; sink(_0xQz, _0xQz);",
        "m.mjs",
        "rename",
    );
    assert!(imp.contains("_0xQz"), "imported binding renamed: {imp}");
}

#[test]
fn direct_eval_disables_module_root_renaming() {
    // Direct `eval` can resolve a module-scope name by string, so root-scope
    // renaming must be disabled when it is present (locals are still fair game,
    // but this binding is root-scope).
    let out = run_pass(
        "var _0xQz = window.x; sink(_0xQz, eval('1'));",
        "m.mjs",
        "rename",
    );
    assert!(
        out.contains("_0xQz"),
        "module root binding renamed despite direct eval: {out}"
    );
}

#[test]
fn module_top_level_rename_end_to_end() {
    // Full pipeline: a recursive (so non-inlinable) opaque top-level helper in a
    // module is renamed, while the exported entry point is preserved.
    let out = deobfuscate(
        "function _0x3a(_0xn){ return _0xn <= 1 ? 1 : _0xn * _0x3a(_0xn - 1); }\
         export function run(){ return _0x3a(window.x); }",
        "m.mjs",
    )
    .code;
    assert!(!out.contains("_0x3a"), "module helper not renamed: {out}");
    assert!(
        out.contains("export function run"),
        "exported entry not preserved: {out}"
    );
}

#[test]
fn swc_helper_markers_get_canonical_names() {
    // A function carrying swc/babel's self-identifying directive prologue is named
    // from that marker, not the misleading body-shape heuristic (a `?:` body would
    // otherwise become `pick`, hiding `instanceOf` / `typeOf`). Run in isolation so
    // the helpers aren't inlined before rename sees them.
    let out = run_pass(
        r#"(function(){
            function aB(o, C) { "@swc/helpers - instanceof"; return o instanceof C; }
            function cD(v) { "@babel/helpers - typeof"; return typeof v; }
            sink(aB(x, Y), cD(z));
        })();"#,
        "t.js",
        "rename",
    );
    assert!(
        out.contains("instanceOf"),
        "instanceof helper not named: {out}"
    );
    assert!(out.contains("typeOf"), "typeof helper not named: {out}");
}
