//! `deob [FLAGS] [<file.js>]` — parse, run the fixpoint pipeline, print readable JS.
//!
//! Flags (combinable, any order):
//!   --format        re-print with NO transforms (the "before" side of a diff).
//!   --stdin  / -    read the source from stdin instead of a file argument.
//!   --json          emit the full `Report` as JSON on stdout (nothing else on
//!                   stdout). This is the contract the container server consumes:
//!                   one JSON object per invocation, with `error` set (and `code`
//!                   = the input) when the pipeline declined/panicked. A clean
//!                   exit (0) always means "valid JSON produced"; the server
//!                   treats a non-zero/killed exit as a timeout/OOM.

use std::io::Read;

use js_deobfuscator::{deobfuscate, format_only, Report};

struct Args {
    format: bool,
    json: bool,
    stdin: bool,
    path: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        format: false,
        json: false,
        stdin: false,
        path: None,
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--format" => a.format = true,
            "--json" => a.json = true,
            "--stdin" | "-" => a.stdin = true,
            flag if flag.starts_with("--") => {
                eprintln!("unknown flag: {flag}");
                std::process::exit(2);
            }
            _ if a.path.is_some() => {
                eprintln!("multiple input paths given");
                std::process::exit(2);
            }
            _ => a.path = Some(arg),
        }
    }
    a
}

fn main() {
    let args = parse_args();

    // Resolve the source + a display name (drives source-type detection).
    let (source, name) = if args.stdin {
        let mut s = String::new();
        if let Err(e) = std::io::stdin().lock().read_to_string(&mut s) {
            eprintln!("cannot read stdin: {e}");
            std::process::exit(1);
        }
        (s, "stdin.js".to_string())
    } else {
        let Some(path) = args.path.clone() else {
            eprintln!("usage: deob [--format] [--json] [--stdin] [<file.js>]");
            std::process::exit(2);
        };
        match std::fs::read_to_string(&path) {
            Ok(s) => (s, path),
            Err(e) => {
                eprintln!("cannot read {path}: {e}");
                std::process::exit(1);
            }
        }
    };

    if args.format {
        let code = format_only(&source, &name);
        if args.json {
            print_json(&Report {
                code,
                parse_errors: Vec::new(),
                rounds: 0,
                converged: true,
                changes: Vec::new(),
                error: None,
            });
        } else {
            print!("{code}");
        }
        return;
    }

    let report = deobfuscate(&source, &name);

    if args.json {
        print_json(&report);
        return;
    }

    for e in &report.parse_errors {
        eprintln!("parse: {e}");
    }
    if let Some(err) = &report.error {
        eprintln!("[deob] {err}");
    }
    print!("{}", report.code);
    eprintln!(
        "\n[deob] rounds={} converged={} changes={:?}",
        report.rounds, report.converged, report.changes
    );
}

/// Serialize a [`Report`] to a single JSON object on stdout. `serde_json` handles
/// escaping of arbitrary JS (quotes/newlines/unicode) in `code` correctly.
fn print_json(report: &Report) {
    let value = serde_json::json!({
        "code": report.code,
        "error": report.error,
        "rounds": report.rounds,
        "converged": report.converged,
        "parse_errors": report.parse_errors,
        "changes": report
            .changes
            .iter()
            .map(|(pass, rounds)| serde_json::json!({ "pass": pass, "rounds": rounds }))
            .collect::<Vec<_>>(),
    });
    // Compact one-line JSON — the server passes this through verbatim.
    println!(
        "{}",
        serde_json::to_string(&value).expect("Report JSON is serializable")
    );
}
