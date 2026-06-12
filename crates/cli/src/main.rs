//! `deob [--format] <file.js>` — parse, run the fixpoint pipeline, print readable JS.
//!
//! Flags:
//!   --format   re-print with NO transforms (the "before" side of a diff).

use devirt_core::{deobfuscate, format_only};

struct Args {
    format: bool,
    path: Option<String>,
}

fn parse_args() -> Args {
    let mut a = Args {
        format: false,
        path: None,
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--format" => a.format = true,
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

    let Some(path) = args.path else {
        eprintln!("usage: deob [--format] <file.js>");
        std::process::exit(2);
    };
    let source = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };

    if args.format {
        print!("{}", format_only(&source, &path));
        return;
    }

    let report = deobfuscate(&source, &path);

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
