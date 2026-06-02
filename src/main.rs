//! `deob <file.js>`            — parse, run the fixpoint pipeline, print readable JS.
//! `deob --format <file.js>`   — parse and re-print with NO transforms (the
//!                               "before" side of a diff, same formatter).

use js_deobfuscator::{deobfuscate, format_only};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (format, path) = match args.get(1).map(String::as_str) {
        Some("--format") => (true, args.get(2)),
        _ => (false, args.get(1)),
    };

    let Some(path) = path else {
        eprintln!("usage: deob [--format] <file.js>");
        std::process::exit(2);
    };

    let source = std::fs::read_to_string(path).unwrap_or_else(|e| {
        eprintln!("cannot read {path}: {e}");
        std::process::exit(1);
    });

    if format {
        print!("{}", format_only(&source, path));
        return;
    }

    let report = deobfuscate(&source, path);
    for e in &report.parse_errors {
        eprintln!("parse: {e}");
    }
    print!("{}", report.code);

    eprintln!(
        "\n[deob] rounds={} converged={} changes={:?}",
        report.rounds, report.converged, report.changes
    );
}
