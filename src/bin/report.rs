//! `report` — a small readability dashboard over the local corpus (`samples/`).
//!
//! Not a test; a quick way to see the pipeline's effect on real input. Pull the
//! corpus first (`scripts/pull-corpus.sh`), then:
//!
//!   cargo run --release --bin report            # scores ./samples
//!   cargo run --release --bin report <dir>      # scores another directory
//!
//! For each *.js found, run the deobfuscator and print two size-normalized
//! proxies for "how readable is the output":
//!   kept%   — output bytes / input bytes (less dead weight => lower)
//!   opaque% — fraction of identifiers that are hex (`_0x..`) or 1-2 chars
//!             (more meaningful names => lower)
use devirt_core::deobfuscate;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("samples"));

    let mut files = Vec::new();
    collect(&root, &mut files);
    files.sort();

    if files.is_empty() {
        eprintln!(
            "no .js files under {} — run scripts/pull-corpus.sh first",
            root.display()
        );
        std::process::exit(1);
    }

    println!(
        "{:<44} {:>9} {:>9} {:>6} {:>8}",
        "sample", "in", "out", "kept%", "opaque%"
    );
    let (mut in_sum, mut out_sum) = (0usize, 0usize);
    for p in &files {
        let Ok(src) = fs::read_to_string(p) else {
            continue;
        };
        let name = p.file_name().unwrap().to_string_lossy();
        let r = deobfuscate(&src, &name);
        in_sum += src.len();
        out_sum += r.code.len();
        println!(
            "{:<44} {:>9} {:>9} {:>5}% {:>7.1}%",
            trunc(&rel(&root, p), 44),
            src.len(),
            r.code.len(),
            pct(r.code.len(), src.len()),
            opaque_ratio(&r.code),
        );
    }
    println!(
        "{:<44} {:>9} {:>9} {:>5}%",
        format!("TOTAL ({} files)", files.len()),
        in_sum,
        out_sum,
        pct(out_sum, in_sum),
    );
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Skip git internals and the plain seeds (inputs, not obfuscated samples).
            if p.file_name()
                .map(|n| n == ".git" || n == "seeds")
                .unwrap_or(false)
            {
                continue;
            }
            collect(&p, out);
        } else if p.extension().map(|x| x == "js").unwrap_or(false) {
            out.push(p);
        }
    }
}

fn pct(part: usize, whole: usize) -> usize {
    part * 100 / whole.max(1)
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root)
        .unwrap_or(p)
        .to_string_lossy()
        .into_owned()
}

fn trunc(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("…{}", &s[s.len() - (n - 1)..])
    }
}

/// Percentage of identifier tokens that look machine-generated: a `_0x…` hex
/// name, or 1-2 characters long. A coarse but stable readability proxy.
fn opaque_ratio(code: &str) -> f64 {
    let (mut total, mut opaque) = (0usize, 0usize);
    let bytes = code.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if is_ident_start(c) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_ident_part(bytes[i]) {
                i += 1;
            }
            let tok = &code[start..i];
            total += 1;
            if tok.starts_with("_0x") || tok.starts_with("0x") || tok.len() <= 2 {
                opaque += 1;
            }
        } else {
            i += 1;
        }
    }
    if total == 0 {
        0.0
    } else {
        opaque as f64 * 100.0 / total as f64
    }
}

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b'$'
}

fn is_ident_part(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'$'
}
