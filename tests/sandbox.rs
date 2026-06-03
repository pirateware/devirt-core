use js_deobfuscator::sandbox::Sandbox;

#[test]
fn evaluates_a_real_base91_style_decoder() {
    let sb = Sandbox::new().expect("sandbox");
    // A self-contained pure decoder closure + a call, exactly the shape we'd
    // extract from a sample. Returns "hi".
    let code = r#"
        (function () {
            var table = "0123456789abcdefghijklmnopqrstuvwxyz";
            function dec(i) { return table[i] + table[i + 1]; }
            return dec(17);
        })()
    "#;
    assert_eq!(sb.eval_to_string(code).as_deref(), Some("hi"));
}

#[test]
fn no_filesystem_access() {
    let sb = Sandbox::new().expect("sandbox");
    // `require` / `process` must not exist → evaluation fails → None.
    assert_eq!(
        sb.eval_to_string("typeof require"),
        Some("undefined".into())
    );
    assert_eq!(
        sb.eval_to_string("typeof process"),
        Some("undefined".into())
    );
}

#[test]
fn infinite_loop_is_interrupted() {
    let sb = Sandbox::new().expect("sandbox");
    // Must return None (interrupted), not hang.
    assert_eq!(sb.eval_to_string("while(true){}"), None);
}
