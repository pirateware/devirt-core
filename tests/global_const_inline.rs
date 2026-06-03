//! Cross-function write-once constant inlining tests. Structural patterns only.

use js_deobfuscator::deobfuscate;

#[test]
fn inlines_constant_written_in_init_function() {
    // The flattened-VM shape: opcode constants assigned in an init function run at
    // the top, read as switch labels in a later dispatcher. Inlining turns the
    // symbolic `case L:` into numeric labels and lets DCE drop the init function.
    // `run` is exported (not called here), so it can't be fully folded away — the
    // numeric case labels remain observable after inlining.
    let src = r#"
        (function () {
            var L1, L2, STOP;
            function init() { L1 = 11; L2 = 22; STOP = 0; }
            init();
            var run = function (s) {
                var acc = 0;
                while (s != STOP) {
                    switch (s) {
                        case L1: acc += 100; s = L2; break;
                        case L2: acc += 200; s = STOP; break;
                    }
                }
                return acc;
            };
            sink(run);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // Numeric case labels appear (the symbolic ones were inlined away).
    assert!(
        out.contains("case 11") && out.contains("case 22"),
        "constants not inlined into case labels: {out}"
    );
    // The init function's constant writes are inlined out, leaving it empty.
    assert!(
        out.contains("function init() {}"),
        "init constant writes not removed (init not emptied): {out}"
    );
}

#[test]
fn does_not_inline_read_before_init() {
    // A read textually/executionally before the init call must observe undefined,
    // so the constant must NOT be inlined there.
    let src = r#"
        (function () {
            var k;
            var snapshot = "" + k;
            function init() { k = 9; }
            init();
            sink(snapshot, k);
        })();
    "#;
    let out = deobfuscate(src, "t.js").code;
    // `snapshot` captures the pre-init undefined. Inlining the constant would make
    // the write dead (and DCE it) and fold the snapshot to a literal; since a read
    // precedes the init, the pass must decline — so the `= 9` write survives and
    // the snapshot stays a concatenation over the (renamed) binding, not a literal.
    assert!(
        out.contains("= 9"),
        "read-before-init was unsoundly inlined (write removed): {out}"
    );
    assert!(
        out.contains("\"\" +"),
        "snapshot read was unsoundly folded to a literal: {out}"
    );
}
