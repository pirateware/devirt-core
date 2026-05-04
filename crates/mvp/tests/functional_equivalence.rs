//! T2 Stream D — Behavioral-equivalence tests via Node shellout.
//!
//! For each program, we:
//!   1. Run the input through `node` to capture (stdout, stderr, exit_code).
//!   2. Push the input through the full deobfuscator pipeline
//!      (parse → lower → SSA → out-of-SSA → emit) to get an output JS string.
//!   3. Run the output through `node` and capture (stdout, stderr,
//!      exit_code).
//!   4. Assert the two captures match byte-for-byte.
//!
//! Gated by the `node-tests` cargo feature so CI without a Node runtime
//! can build and run `cargo test` (without the feature) cleanly. Under
//! default features, this entire file is compiled out — verified by
//! the `#![cfg(feature = "node-tests")]` crate-level gate.
//!
//! Run with:
//!     cargo test --features node-tests --test functional_equivalence
//!
//! Coverage today (Stream A foundation only):
//!   - PassThrough function bodies (class, generator, try/catch)
//!   - Mixed Lowered + PassThrough at module level
//!   - Plain Lowered control flow (arithmetic, while, recursion)
//!
//! Cases that need Stream B (nested closures emitted from OXC) are
//! present but `#[ignore = "needs Stream B"]` so the suite stays green
//! pre-merge.

#![cfg(feature = "node-tests")]

use std::io::Write;
use std::process::{Command, Stdio};

/// Run JS source through Node, returning (stdout, stderr, exit_code).
/// `stdin` is piped to the child process so programs that consume
/// stdin can be tested.
fn run_node(source: &str, stdin: &str) -> (String, String, Option<i32>) {
    let mut child = Command::new("node")
        .arg("--input-type=commonjs")
        .arg("-e")
        .arg(source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn node — is it on PATH?");

    if let Some(mut sin) = child.stdin.take() {
        sin.write_all(stdin.as_bytes())
            .expect("write stdin to node");
    }

    let out = child.wait_with_output().expect("node wait_with_output");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code(),
    )
}

/// Pipe `input` through the deobfuscator and assert that running both
/// the input and the output through Node produces identical
/// (stdout, stderr, exit_code).
///
/// `harness` is a snippet appended to BOTH the input and the output
/// before they're handed to Node. This is how each test exercises
/// its functions — the deobfuscator pipeline today drops top-level
/// non-function statements (an Item::Stmt limitation outside Stream
/// D's scope), so calls to the functions under test must be added by
/// the test driver after pipeline execution rather than baked into
/// the source.
fn assert_functional_equivalence(input: &str, harness: &str, stdin: &str) {
    let output = mvp::process(input).expect("pipeline must produce output");
    let in_full = format!("{input}\n{harness}");
    let out_full = format!("{output}\n{harness}");
    let (in_stdout, in_stderr, in_code) = run_node(&in_full, stdin);
    let (out_stdout, out_stderr, out_code) = run_node(&out_full, stdin);

    // Normalize stderr — Node's error messages can include source
    // positions that legitimately differ between input and output.
    // For the tests we author here, stderr should be empty; surface
    // any divergence via the assertion message.
    assert_eq!(
        in_stdout, out_stdout,
        "stdout mismatch.\n--- input ---\n{input}\n--- output ---\n{output}\n--- input stdout ---\n{in_stdout}\n--- output stdout ---\n{out_stdout}",
    );
    assert_eq!(
        in_code, out_code,
        "exit-code mismatch: input={in_code:?} output={out_code:?}\nstderr in: {in_stderr}\nstderr out: {out_stderr}",
    );
    // stderr equality is asserted last so the more useful messages
    // above fire first.
    assert_eq!(in_stderr, out_stderr, "stderr mismatch");
}

// === Lowered programs (Stream A foundation already supports these) ===

#[test]
fn lowered_arithmetic_and_log() {
    let src = "function add(a, b) { return a + b; }";
    let harness = "console.log(add(2, 3)); console.log(add(10, -7));";
    assert_functional_equivalence(src, harness, "");
}

#[test]
fn lowered_while_loop_sum() {
    let src = r#"
        function sum(n) {
            let i = 0;
            let total = 0;
            while (i < n) {
                i = i + 1;
                total = total + i;
            }
            return total;
        }
    "#;
    let harness = "console.log(sum(10)); console.log(sum(0));";
    assert_functional_equivalence(src, harness, "");
}

#[test]
fn lowered_recursive_fibonacci() {
    let src = r#"
        function fib(n) {
            if (n <= 1) return n;
            return fib(n - 1) + fib(n - 2);
        }
    "#;
    let harness = "console.log(fib(10)); console.log(fib(0)); console.log(fib(1));";
    assert_functional_equivalence(src, harness, "");
}

#[test]
fn lowered_if_else_branches() {
    // Only positive constants: unary minus isn't lowered to SSA yet,
    // and that's outside Stream D's scope to fix.
    let src = r#"
        function classify(x) {
            if (x > 10) return 2;
            if (x > 0) return 1;
            return 0;
        }
    "#;
    let harness =
        "console.log(classify(20)); console.log(classify(5)); console.log(classify(0));";
    assert_functional_equivalence(src, harness, "");
}

// === PassThrough programs (function body contains an unsupported
// construct; pipeline preserves the body verbatim) ===

#[test]
fn passthrough_class_in_body() {
    let src = r#"
        function makeWidget(label) {
            class Widget {
                constructor(l) { this.l = l; }
                describe() { return "widget:" + this.l; }
            }
            return new Widget(label).describe();
        }
    "#;
    let harness = r#"console.log(makeWidget("alpha")); console.log(makeWidget("beta"));"#;
    assert_functional_equivalence(src, harness, "");
}

#[test]
fn passthrough_generator_in_body() {
    let src = r#"
        function collect(n) {
            function* gen() {
                let i = 0;
                while (i < n) { yield i; i++; }
            }
            const out = [];
            for (const v of gen()) out.push(v);
            return out.join(",");
        }
    "#;
    let harness = "console.log(collect(5)); console.log(collect(0));";
    assert_functional_equivalence(src, harness, "");
}

#[test]
fn passthrough_try_catch_in_body() {
    let src = r#"
        function safe(x) {
            try {
                return x.foo.bar;
            } catch (e) {
                return "caught";
            }
        }
    "#;
    let harness = "console.log(safe({ foo: { bar: 7 } })); console.log(safe(null)); console.log(safe(undefined));";
    assert_functional_equivalence(src, harness, "");
}

// === Mixed module: one Lowered and one PassThrough at the top level ===

#[test]
fn mixed_lowered_and_passthrough_module() {
    let src = r#"
        function add(a, b) { return a + b; }
        function listify() {
            class Bag {
                constructor() { this.xs = []; }
                push(v) { this.xs.push(v); return this; }
                join(sep) { return this.xs.join(sep); }
            }
            return new Bag().push(1).push(2).push(3).join("-");
        }
    "#;
    let harness = "console.log(add(40, 2)); console.log(listify());";
    assert_functional_equivalence(src, harness, "");
}

// === Stream B / post-merge: nested closures from OXC ===

#[test]
fn closure_capture_lowered() {
    let src = r#"
        function adder(x) {
            return function (y) { return x + y; };
        }
    "#;
    let harness = "const add5 = adder(5); console.log(add5(3)); console.log(add5(10));";
    assert_functional_equivalence(src, harness, "");
}

#[test]
fn closure_capture_across_passthrough_boundary() {
    let src = r#"
        function outer(label) {
            function inner() {
                class Box { describe() { return label; } }
                return new Box().describe();
            }
            return inner();
        }
    "#;
    let harness = r#"console.log(outer("hello"));"#;
    assert_functional_equivalence(src, harness, "");
}

// === Post-merge regression tests: nested PassThrough capture
// propagation. These exercise the architectural rule that
// PassThrough is opaque to passes but not to scope analysis. The
// rule's load-bearing application: any name a nested PassThrough
// body lexically references must (a) appear with its declaration
// in the outer's emit, (b) preserve every source-level mutation
// that affects the value the body will read at runtime. ===

/// Two-level: Lowered → Lowered → PassThrough capturing from
/// outermost. `c` is PassThrough (generator); `b` is Lowered;
/// `a` binds `x`. Without the fix, `let x = 1` was silently DCE'd
/// because Const-init lets produce no Inst; the PT body's `yield x`
/// then dangled at runtime as ReferenceError.
#[test]
fn nested_passthrough_capture_via_lowered_intermediary() {
    let src = r#"
        function a() {
            let x = 1;
            function b() {
                function* c() { yield x; }
                return c;
            }
            return b;
        }
    "#;
    let harness = "console.log(a()()().next().value);";
    assert_functional_equivalence(src, harness, "");
}

/// Sibling + nested mix: one PassThrough sibling and one
/// PassThrough grandchild via a Lowered intermediary, both
/// capturing the same outer name. The single binding must serve
/// both AllocClosure pin sites without double-renaming.
#[test]
fn nested_passthrough_capture_sibling_and_grandchild() {
    let src = r#"
        function a() {
            let x = 1;
            function* b() { yield x; }
            function c() {
                function* d() { yield x; }
                return d;
            }
            let cgen = c();
            return [b().next().value, cgen().next().value];
        }
    "#;
    let harness = "console.log(JSON.stringify(a()));";
    assert_functional_equivalence(src, harness, "");
}

/// Reassignment-in-outer + PassThrough capture. Inner generator
/// `b` captures `x`; outer reassigns `x` after `b`'s allocation
/// but before the iterator is consumed. PT semantics dictate that
/// the body reads the live value (2), not the snapshot (1). The
/// disposition rule may route `b` to PT (it already is, being a
/// generator); the `x = 2` reassignment must survive the SSA
/// round-trip — the synthesized `StoreProp(GlobalThis, "x", 2)`
/// path does that.
#[test]
fn passthrough_capture_observes_outer_reassignment() {
    let src = r#"
        function a() {
            let x = 1;
            x = 2;
            function* b() { yield x; }
            return b;
        }
    "#;
    let harness = "console.log(a()().next().value);";
    assert_functional_equivalence(src, harness, "");
}

/// Reassign-after-Lowered-closure-allocation: forces the inner
/// closure to PassThrough per closures-disposition-rule (capture
/// is reassigned). The outer's `let y = 10; ... y = 20;` must
/// both survive so the PT body reads 20 at f(1) time.
#[test]
fn passthrough_capture_reassign_forces_inner_passthrough() {
    let src = r#"
        function outer() {
            let y = 10;
            let f = function (x) { return x + y; };
            y = 20;
            return f(1);
        }
    "#;
    let harness = "console.log(outer());";
    assert_functional_equivalence(src, harness, "");
}
