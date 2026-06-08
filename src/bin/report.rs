//! `report` — readability metrics for the whole sample corpus. Not a test;
//! a dashboard for tracking the pipeline's effect over time. Metrics are
//! sample-agnostic proxies for "how readable is the output":
//!   - bytes / lines (smaller = less dead weight)
//!   - bracket string accesses `["x"]` (lower = more `.x` member access)
//!   - "opaque name ratio": fraction of identifiers that are 1-2 chars or hex
//!     (lower = more meaningful names)

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use devirt_core::deobfuscate;

fn main() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    let mut entries: Vec<_> = fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("js"))
        .collect();
    entries.sort();

    println!(
        "{:<13} {:>9} {:>9} {:>6}  {:>8} {:>8}  {:>6}",
        "sample", "in_bytes", "out_bytes", "kept%", "brackets", "opaque%", "rounds"
    );
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let src = fs::read_to_string(&p).unwrap();
        let r = deobfuscate(&src, &name);

        let kept = r.code.len() * 100 / src.len().max(1);
        let brackets = count_bracket_access(&r.code);
        let opaque = opaque_name_ratio(&r.code);
        println!(
            "{:<13} {:>9} {:>9} {:>5}% {:>8} {:>7}% {:>6}",
            name,
            src.len(),
            r.code.len(),
            kept,
            brackets,
            opaque,
            r.rounds
        );
    }
}

fn count_bracket_access(s: &str) -> usize {
    // crude: count `["` occurrences (string-keyed member access)
    s.matches("[\"").count()
}

/// Fraction (%) of distinct identifier-ish tokens that look obfuscated:
/// length ≤ 2, or hex-style `_0x...`.
fn opaque_name_ratio(s: &str) -> usize {
    let mut all: HashSet<String> = HashSet::new();
    let mut opaque: HashSet<String> = HashSet::new();
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            cur.push(ch);
        } else {
            consider(&mut cur, &mut all, &mut opaque);
        }
    }
    consider(&mut cur, &mut all, &mut opaque);
    if all.is_empty() {
        0
    } else {
        opaque.len() * 100 / all.len()
    }
}

fn consider(cur: &mut String, all: &mut HashSet<String>, opaque: &mut HashSet<String>) {
    if cur.is_empty() {
        return;
    }
    let tok = std::mem::take(cur);
    if tok.chars().next().unwrap().is_ascii_digit() {
        return; // numeric literal
    }
    let is_opaque = tok.len() <= 2 || tok.starts_with("_0x");
    if is_opaque {
        opaque.insert(tok.clone());
    }
    all.insert(tok);
}
