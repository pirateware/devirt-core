use js_deobfuscator::deobfuscate;

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
    assert!(!out.contains("WL") && !out.contains("jK"), "opaque names remain: {out}");
    assert!(out.contains("fn1"), "function not renamed by role: {out}");
}

#[test]
fn does_not_touch_property_names() {
    // `WL` here is a property key, not a reference to a binding — must survive.
    let out = deobfuscate("var ab = {}; ab.WL = 1; foo(ab.WL);", "t.js").code;
    assert!(out.contains(".WL"), "property name was wrongly renamed: {out}");
}

#[test]
fn preserves_global_names() {
    // Top-level (global/observable) bindings must not be renamed.
    let out = deobfuscate("var Qz = 1; window.x = Qz;", "t.js").code;
    assert!(out.contains("Qz"), "global binding renamed: {out}");
}
