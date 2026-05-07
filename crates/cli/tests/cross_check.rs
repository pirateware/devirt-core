//! Functional cross-check harness — issue #42.
//!
//! For each corpus sample with a declared `<name>.test-inputs.json`,
//! we:
//!   1. Run `input.js` through `node` and record (stdout, stderr,
//!      exit_code).
//!   2. Push `input.js` through the full deobfuscator pipeline
//!      (`cli::process`) to get an output JS string.
//!   3. Run the output through `node` with the same harness/stdin/argv
//!      and record the same triple.
//!   4. Assert the two triples match.
//!
//! Each cross-check invocation runs in its own `node` subprocess so
//! the global env is fresh — there's no cross-contamination between
//! cases or between samples.
//!
//! Gated by the `cross-check` cargo feature so plain
//! `cargo test --workspace` stays fast and works on machines without
//! `node` installed. If `node` is missing when the feature IS on,
//! every sample is skipped with a single clear stderr message rather
//! than mass-failing.
//!
//! Run:
//!     cargo test --features cross-check --test cross_check
//!
//! ## Determinism rule (NOT auto-detected — author's responsibility)
//!
//! Cross-check runs the SAME source twice. Anything that produces a
//! different result on a second run breaks the harness, not the
//! deobfuscator. Samples that use any of:
//!   - `Math.random`, `Date.now`, `new Date()` without a fixed arg
//!   - `setTimeout` / `setInterval` / async I/O
//!   - network, filesystem, env-var-dependent reads
//! must NOT have a `test-inputs.json` (or must set `"skip": true` with
//! a `"skip_reason"`). The harness does not auto-detect these.
//!
//! ## Adding test-inputs for a new sample
//!
//! Place a sibling file next to the sample:
//!     <sample>.test-inputs.json
//!
//! Schema:
//!   {
//!     "skip": false,                    // optional, default false
//!     "skip_reason": "",                // optional; required if skip=true
//!     "timeout_secs": 10,               // optional, default 10
//!     "cases": [
//!       {
//!         "name": "default",            // required, used in failure messages
//!         "harness": "",                // optional JS appended to BOTH input
//!                                       //  and output before running through node
//!         "stdin": "",                  // optional string piped to node's stdin
//!         "args": []                    // optional process.argv tail (strings)
//!       }
//!     ]
//!   }
//!
//! For a sample whose source already drives observable behaviour at
//! module level (e.g. `console.log(fact(5))`), use a single case with
//! empty `harness`. For a sample that defines functions but doesn't
//! call them, supply a `harness` snippet that does the calling — the
//! deobfuscator's emit must agree with the input on what those calls
//! print.
//!
//! Pipeline-failed samples (the deobfuscator can't compile them) MUST
//! NOT have a test-inputs.json — fix the pipeline first or leave the
//! sample uncovered. The harness will fail loudly if a declared sample
//! doesn't compile.

#![cfg(feature = "cross-check")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

const DEFAULT_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Deserialize)]
struct TestInputs {
    #[serde(default)]
    skip: bool,
    #[serde(default)]
    skip_reason: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    cases: Vec<TestCase>,
}

#[derive(Debug, Deserialize)]
struct TestCase {
    name: String,
    #[serde(default)]
    harness: String,
    #[serde(default)]
    stdin: String,
    #[serde(default)]
    args: Vec<String>,
}

/// Locate the workspace root from the cli crate's manifest dir.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Returns true if `node` is on PATH and runnable.
fn node_available() -> bool {
    Command::new("node")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Capture from a single `node` invocation.
#[derive(Debug, PartialEq, Eq)]
struct NodeOutput {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
}

/// Run `source` through node, with `stdin` piped in and `args`
/// supplied as `process.argv` tail. Times out after `timeout`.
///
/// Failure to spawn / wait / wait-with-timeout panics with a clear
/// message — these are environment problems, not test failures.
fn run_node(
    source: &str,
    stdin: &str,
    args: &[String],
    timeout: Duration,
) -> NodeOutput {
    let mut cmd = Command::new("node");
    cmd.arg("--input-type=commonjs").arg("-e").arg(source);
    if !args.is_empty() {
        // Anything after `--` becomes part of process.argv.
        cmd.arg("--");
        for a in args {
            cmd.arg(a);
        }
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn node — is it on PATH?");

    if let Some(mut sin) = child.stdin.take() {
        sin.write_all(stdin.as_bytes())
            .expect("write stdin to node");
    }

    // Poll the child for completion with a hard timeout. wait_timeout
    // would be cleaner but is an external crate; the std-only loop
    // here is good enough for a 10s ceiling.
    let start = Instant::now();
    loop {
        match child.try_wait().expect("try_wait") {
            Some(_) => break,
            None => {
                if start.elapsed() > timeout {
                    // SAFETY: we just observed the child is still
                    // running. Kill is best-effort; failure here means
                    // the process already exited which is fine.
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!(
                        "node exceeded timeout of {}s — sample is non-terminating or too slow",
                        timeout.as_secs(),
                    );
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }

    let out = child
        .wait_with_output()
        .expect("wait_with_output after try_wait");
    NodeOutput {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit_code: out.status.code(),
    }
}

/// Cross-check one (sample, case) pair. Panics with a detailed
/// message on mismatch.
fn check_one(sample: &Path, case: &TestCase, timeout: Duration) {
    let source = std::fs::read_to_string(sample)
        .unwrap_or_else(|e| panic!("read sample {}: {e}", sample.display()));
    let deobf = cli::process(&source).unwrap_or_else(|e| {
        panic!(
            "deobfuscator pipeline failed on {} (case {}): {e:?}",
            sample.display(),
            case.name,
        )
    });

    let in_full = format!("{source}\n{}", case.harness);
    let out_full = format!("{deobf}\n{}", case.harness);
    let in_run = run_node(&in_full, &case.stdin, &case.args, timeout);
    let out_run = run_node(&out_full, &case.stdin, &case.args, timeout);

    if in_run != out_run {
        panic!(
            "cross-check FAILED\n\
             sample : {}\n\
             case   : {}\n\
             --- input source ---\n{source}\n\
             --- deobfuscated source ---\n{deobf}\n\
             --- input run ---\nstdout: {:?}\nstderr: {:?}\ncode: {:?}\n\
             --- output run ---\nstdout: {:?}\nstderr: {:?}\ncode: {:?}\n",
            sample.display(),
            case.name,
            in_run.stdout,
            in_run.stderr,
            in_run.exit_code,
            out_run.stdout,
            out_run.stderr,
            out_run.exit_code,
        );
    }
}

/// Discover every `*.test-inputs.json` under the corpus tree.
fn discover_specs(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && name.ends_with(".test-inputs.json")
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// Map `<sample>.test-inputs.json` → `<sample>.js`.
fn sample_for_spec(spec: &Path) -> PathBuf {
    let s = spec.to_string_lossy();
    let stripped = s
        .strip_suffix(".test-inputs.json")
        .expect("spec path ends with .test-inputs.json");
    PathBuf::from(format!("{stripped}.js"))
}

#[test]
fn cross_check_corpus() {
    if !node_available() {
        eprintln!(
            "node is not available on PATH; skipping cross_check_corpus. \
             Install Node.js to run this suite (cargo test --features cross-check)."
        );
        return;
    }

    let root = workspace_root();
    let corpus_roots = [
        root.join("crates").join("cli").join("coverage_corpus"),
        root.join("corpus"),
    ];

    let mut specs = Vec::new();
    for r in &corpus_roots {
        if r.exists() {
            specs.extend(discover_specs(r));
        }
    }

    assert!(
        !specs.is_empty(),
        "no `*.test-inputs.json` found under any corpus root — \
         cross-check should have at least one declared sample. \
         See crates/cli/tests/cross_check.rs for the schema.",
    );

    let mut ran = 0usize;
    let mut skipped = 0usize;
    for spec in &specs {
        let raw = std::fs::read_to_string(spec)
            .unwrap_or_else(|e| panic!("read {}: {e}", spec.display()));
        let inputs: TestInputs = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("parse {}: {e}", spec.display()));

        if inputs.skip {
            assert!(
                !inputs.skip_reason.is_empty(),
                "{}: skip=true requires a non-empty skip_reason",
                spec.display(),
            );
            eprintln!("SKIP {}: {}", spec.display(), inputs.skip_reason);
            skipped += 1;
            continue;
        }

        let sample = sample_for_spec(spec);
        assert!(
            sample.exists(),
            "spec {} references missing sample {}",
            spec.display(),
            sample.display(),
        );
        let timeout = Duration::from_secs(
            inputs.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
        );
        assert!(
            !inputs.cases.is_empty(),
            "{}: at least one case is required",
            spec.display(),
        );

        for case in &inputs.cases {
            eprintln!("cross-check {} :: {}", sample.display(), case.name);
            check_one(&sample, case, timeout);
            ran += 1;
        }
    }

    eprintln!("cross-check: {ran} case(s) passed, {skipped} sample(s) skipped");
}
