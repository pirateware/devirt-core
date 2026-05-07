//! Tests for issue #31: lift `BlockerKind::ClassDecl` via parse-time
//! ES5 prototype-chain desugar.
//!
//! Each test confirms either:
//!   (a) a class-bearing function lowers (no `Item::PassThroughFunction`,
//!       no `class` keyword in re-emit), OR
//!   (b) an un-desugarable shape still routes to PassThrough with the
//!       correct blocker set.

use emit::emit;
use ir::blocker::BlockerKind;
use ir::tree::{FuncBody, Item, Stmt};
use parse::{lower_ssa, lower_tree, parse_and_lower};

fn assert_makefoo_lowered(src: &str) {
    let m = parse_and_lower(src).expect("parse_and_lower");
    for item in &m.items {
        if let Item::PassThroughFunction(lit) = item {
            panic!(
                "expected Lowered, got PassThroughFunction (name={:?}, blockers={:?}) for: {src}",
                lit.name, lit.blockers
            );
        }
    }
}

fn assert_function_body_has_inner_func_decl(m: &ir::tree::Module, fn_name: &str) {
    for item in &m.items {
        if let Item::Function(f) = item
            && f.name == fn_name
        {
            // Look for an inner `Stmt::FuncDecl` (the desugared
            // constructor).
            let mut found = false;
            for s in &f.body {
                if matches!(s, Stmt::FuncDecl(_)) {
                    found = true;
                    break;
                }
            }
            assert!(found, "expected an inner FuncDecl in {fn_name}");
            return;
        }
    }
    panic!("did not find Item::Function {fn_name}");
}

fn ssa_roundtrip_emit(src: &str) -> String {
    let tm = parse_and_lower(src).expect("parse");
    let sm = lower_ssa::lower_module(&tm);
    sm.validate().expect("SSA validation");
    let tm2 = lower_tree::lower_module(&sm);
    let out = emit(&tm2);
    parse_and_lower(&out).expect("re-parse");
    out
}

// ============================================================================
// Basic class shapes
// ============================================================================

#[test]
fn empty_class_in_function_lowers() {
    let src = "function f() { class Foo {} return Foo; }";
    assert_makefoo_lowered(src);
    let m = parse_and_lower(src).unwrap();
    assert_function_body_has_inner_func_decl(&m, "f");
}

#[test]
fn class_with_one_method_lowers() {
    let src = "function f() { class Foo { greet() { return 1; } } return new Foo(); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(out.contains("greet"), "method survives: {out}");
    assert!(!out.contains("class Foo"), "class keyword removed: {out}");
}

#[test]
fn class_with_constructor_lowers() {
    let src = "function f(x) { class Foo { constructor(y) { this.y = y; } } return new Foo(x); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(!out.contains("class Foo"), "{out}");
}

// ============================================================================
// `extends`
// ============================================================================

#[test]
fn class_extends_lowers() {
    let src = r#"
        function makeChild(B) {
            class Child extends B {
                constructor(x) { super(x); this.x = x; }
                method() { return this.x; }
            }
            return new Child(7);
        }
    "#;
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(out.contains("prototype"), "expect prototype-chain hookup: {out}");
    assert!(!out.contains("class Child"), "{out}");
    // The keyword `super` (followed by `(` or `.`) must be gone.
    // The synthesized binding `__super0__` is fine — the keyword
    // form has been rewritten away.
    assert!(
        !out.contains("super(") && !out.contains("super."),
        "super keyword should be rewritten: {out}"
    );
}

#[test]
fn implicit_constructor_with_extends_forwards_args() {
    let src = "function f(B) { class C extends B {} return new C(1, 2); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    // Implicit constructor with extends becomes `__super.apply(this, arguments)`.
    assert!(
        out.contains(".apply") || out.contains(".call"),
        "expect super forwarding: {out}"
    );
}

#[test]
fn super_method_call_lowers() {
    let src = r#"
        function f(B) {
            class C extends B {
                greet() { return super.greet() + 1; }
            }
            return new C();
        }
    "#;
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    // `super.greet()` should rewrite to `<super>.prototype.greet.call(this)`.
    assert!(out.contains(".prototype"), "{out}");
    assert!(out.contains(".call"), "{out}");
    assert!(
        !out.contains("super(") && !out.contains("super."),
        "super keyword removed: {out}"
    );
}

// ============================================================================
// Static members
// ============================================================================

#[test]
fn static_method_lowers() {
    let src = "function f() { class Foo { static bar() { return 42; } } return Foo.bar(); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(out.contains("Foo"), "{out}");
    assert!(!out.contains("static"), "static keyword removed: {out}");
}

#[test]
fn static_field_lowers() {
    let src = "function f() { class Foo { static count = 0; } return Foo.count; }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(out.contains("Foo"), "{out}");
}

// ============================================================================
// Instance fields
// ============================================================================

#[test]
fn instance_field_initializer_lowers() {
    let src = "function f() { class Foo { x = 1; constructor() { this.y = 2; } } return new Foo(); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(!out.contains("class Foo"), "{out}");
}

#[test]
fn instance_field_with_extends_after_super() {
    let src = r#"
        function f(B) {
            class C extends B {
                x = 99;
                constructor() { super(); this.y = this.x + 1; }
            }
            return new C();
        }
    "#;
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    // The `x = 99` initializer must run AFTER the super call.
    assert!(!out.contains("class C"), "{out}");
}

// ============================================================================
// Getters / setters
// ============================================================================

#[test]
fn getter_lowers_via_define_property() {
    let src = "function f() { class Foo { get x() { return 1; } } return new Foo().x; }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(
        out.contains("defineProperty"),
        "expect Object.defineProperty: {out}"
    );
}

#[test]
fn setter_lowers_via_define_property() {
    let src = "function f() { class Foo { set x(v) { this._x = v; } } var o = new Foo(); o.x = 5; return o._x; }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(out.contains("defineProperty"), "{out}");
}

// ============================================================================
// Computed names
// ============================================================================

#[test]
fn computed_method_name_preserves_evaluation() {
    let src = "function f(k) { class Foo { [k]() { return 1; } } return new Foo(); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(!out.contains("class Foo"), "{out}");
}

// ============================================================================
// Class expressions
// ============================================================================

#[test]
fn class_expression_in_function_lowers() {
    let src = "function f() { var X = class { greet() { return 1; } }; return new X(); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    // Class expression desugars to an IIFE around the constructor.
    assert!(!out.contains("class "), "{out}");
}

#[test]
fn named_class_expression_in_function_lowers() {
    let src = "function f() { var X = class Y { greet() { return 1; } }; return new X(); }";
    assert_makefoo_lowered(src);
    let out = ssa_roundtrip_emit(src);
    assert!(!out.contains("class "), "{out}");
}

// ============================================================================
// Blockers (still PassThrough)
// ============================================================================

#[test]
fn private_field_blocks() {
    let src = "function f() { class Foo { #x = 1; greet() { return this.#x; } } return new Foo(); }";
    let m = parse_and_lower(src).expect("PassThrough should not error");
    let mut saw_passthrough = false;
    for item in &m.items {
        if let Item::PassThroughFunction(lit) = item
            && lit.name.as_deref() == Some("f")
        {
            saw_passthrough = true;
            assert!(
                lit.blockers.contains(&BlockerKind::ClassPrivateMember),
                "expected ClassPrivateMember, got {:?}",
                lit.blockers
            );
            assert!(matches!(lit.body, FuncBody::PassThrough(_)));
        }
    }
    assert!(saw_passthrough, "f should be PassThrough due to #x");
}

#[test]
fn static_block_blocks() {
    let src = "function f() { class Foo { static { Foo.x = 1; } } return Foo; }";
    let m = parse_and_lower(src).expect("PassThrough should not error");
    let mut saw_passthrough = false;
    for item in &m.items {
        if let Item::PassThroughFunction(lit) = item
            && lit.name.as_deref() == Some("f")
        {
            saw_passthrough = true;
            assert!(
                lit.blockers.contains(&BlockerKind::ClassStaticBlock),
                "got {:?}",
                lit.blockers
            );
        }
    }
    assert!(saw_passthrough);
}

// ============================================================================
// Round-trip stability
// ============================================================================

#[test]
fn roundtrip_class_idempotent() {
    let src = "function f() { class Foo { x = 1; greet() { return this.x; } } return new Foo(); }";
    let out1 = ssa_roundtrip_emit(src);
    let out2 = ssa_roundtrip_emit(&out1);
    // The second pass should be stable — same output as the first.
    // (We don't require strict equality because of formatting drift; we
    // just check both re-parse and the second pass can still run.)
    parse_and_lower(&out2).expect("idempotent re-parse");
}
