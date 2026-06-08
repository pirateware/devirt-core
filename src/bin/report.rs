//! `report` — a readability dashboard over the local corpus (`samples/`).
//!
//! Not a test; a quick way to see the pipeline's effect on real input. Pull the
//! corpus first (`scripts/pull-corpus.sh`), then:
//!
//!   cargo run --release --bin report            # per-source aggregates over ./samples
//!   cargo run --release --bin report --files    # also list every sample
//!   cargo run --release --bin report <dir>      # score another directory
//!
//! Samples are grouped by their source directory (e.g. `generated`, `real/npm`,
//! `real/tranco`, `real/httparchive`) so each source's scores are visible
//! separately. Two size-normalized proxies for "how readable is the output":
//!   kept%   — output bytes / input bytes (less dead weight => lower)
//!   opaque% — mean fraction of identifiers that are hex (`_0x..`) or 1-2 chars
//!             (more meaningful names => lower)
//!
//! Publishing flags drive the Hugging Face dataset page:
//!   --json <path>   write per-sample rows (incl. input/output code) as JSONL,
//!                   which the HF Dataset Viewer renders as a browsable table
//!   --markdown      print the aggregate scoreboard as a GitHub table (for the
//!                   dataset card / README), instead of the fixed-width table
use devirt_core::deobfuscate;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Agg {
    files: usize,
    in_bytes: usize,
    out_bytes: usize,
    opaque_in_sum: f64,
    opaque_out_sum: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let show_files = args.iter().any(|a| a == "--files");
    let markdown = args.iter().any(|a| a == "--markdown");
    let json_path = flag_value(&args, "--json");
    let root = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .filter(|a| Some(a.as_str()) != json_path.as_deref())
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

    // Optional per-sample JSONL for the HF Dataset Viewer.
    let mut json = json_path
        .as_ref()
        .map(|p| BufWriter::new(File::create(p).expect("create --json file")));

    let mut groups: BTreeMap<String, Agg> = BTreeMap::new();
    let mut total = Agg::default();
    if show_files {
        println!("{:<52} {:>9} {:>9} {:>6} {:>8}", "sample", "in", "out", "kept%", "opaque%");
    }

    for p in &files {
        let Ok(src) = fs::read_to_string(p) else { continue };
        let name = p.file_name().unwrap().to_string_lossy();
        let opaque_in = opaque_ratio(&src);
        let r = deobfuscate(&src, &name);
        let opaque_out = opaque_ratio(&r.code);
        let rel_path = rel(&root, p);
        let source = group_of(&root, p);

        let g = groups.entry(source.clone()).or_default();
        g.files += 1;
        g.in_bytes += src.len();
        g.out_bytes += r.code.len();
        g.opaque_in_sum += opaque_in;
        g.opaque_out_sum += opaque_out;
        total.files += 1;
        total.in_bytes += src.len();
        total.out_bytes += r.code.len();
        total.opaque_in_sum += opaque_in;
        total.opaque_out_sum += opaque_out;

        if let Some(w) = json.as_mut() {
            write_json_row(w, &rel_path, &source, &src, &r.code, opaque_in, opaque_out);
        }

        if show_files {
            println!(
                "{:<52} {:>9} {:>9} {:>5}% {:>6.1}%→{:>5.1}%",
                trunc(&rel_path, 52),
                src.len(),
                r.code.len(),
                pct(r.code.len(), src.len()),
                opaque_in,
                opaque_out,
            );
        }
    }

    if let Some(w) = json.as_mut() {
        w.flush().expect("flush --json file");
        eprintln!("wrote {} rows to {}", total.files, json_path.unwrap());
    }

    if show_files {
        println!();
    }
    if markdown {
        print_markdown(&groups, &total);
    } else {
        println!(
            "{:<24} {:>7} {:>11} {:>11} {:>6} {:>16}",
            "source", "files", "in", "out", "kept%", "opaque% in→out"
        );
        for (name, a) in &groups {
            print_row(name, a);
        }
        print_row("TOTAL", &total);
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned()
}

fn print_row(name: &str, a: &Agg) {
    let n = a.files.max(1) as f64;
    println!(
        "{:<24} {:>7} {:>11} {:>11} {:>5}% {:>6.1}%→{:>5.1}%",
        trunc(name, 24),
        a.files,
        a.in_bytes,
        a.out_bytes,
        pct(a.out_bytes, a.in_bytes),
        a.opaque_in_sum / n,
        a.opaque_out_sum / n,
    );
}

/// GitHub-flavored markdown table for the dataset card.
fn print_markdown(groups: &BTreeMap<String, Agg>, total: &Agg) {
    println!("| source | files | in bytes | out bytes | kept% | opaque% in→out |");
    println!("|---|--:|--:|--:|--:|--:|");
    let row = |name: &str, a: &Agg| {
        let n = a.files.max(1) as f64;
        println!(
            "| `{}` | {} | {} | {} | {}% | {:.1}% → {:.1}% |",
            name, a.files, a.in_bytes, a.out_bytes, pct(a.out_bytes, a.in_bytes),
            a.opaque_in_sum / n, a.opaque_out_sum / n,
        );
    };
    for (name, a) in groups {
        row(name, a);
    }
    row("TOTAL", total);
}

/// One JSONL row per sample. Hand-rolled JSON (no serde dep) — only strings need
/// escaping, and `js_escape` covers the JSON string grammar.
fn write_json_row(
    w: &mut impl Write,
    path: &str,
    source: &str,
    input: &str,
    output: &str,
    opaque_in: f64,
    opaque_out: f64,
) {
    let _ = writeln!(
        w,
        "{{\"path\":\"{}\",\"source\":\"{}\",\"in_bytes\":{},\"out_bytes\":{},\"kept_pct\":{},\"opaque_in\":{:.1},\"opaque_out\":{:.1},\"input\":\"{}\",\"output\":\"{}\"}}",
        json_escape(path),
        json_escape(source),
        input.len(),
        output.len(),
        pct(output.len(), input.len()),
        opaque_in,
        opaque_out,
        json_escape(input),
        json_escape(output),
    );
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            // Skip git internals and the plain seeds (inputs, not obfuscated samples).
            if p.file_name().map(|n| n == ".git" || n == "seeds").unwrap_or(false) {
                continue;
            }
            collect(&p, out);
        } else if p.extension().map(|x| x == "js").unwrap_or(false) {
            out.push(p);
        }
    }
}

/// Source group = the sample's directory relative to root (e.g. `real/npm`).
fn group_of(root: &Path, p: &Path) -> String {
    p.parent()
        .and_then(|d| d.strip_prefix(root).ok())
        .map(|d| d.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".into())
}

fn pct(part: usize, whole: usize) -> usize {
    part * 100 / whole.max(1)
}

fn rel(root: &Path, p: &Path) -> String {
    p.strip_prefix(root).unwrap_or(p).to_string_lossy().into_owned()
}

fn trunc(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let tail: String = s.chars().rev().take(n - 1).collect::<Vec<_>>().into_iter().rev().collect();
        format!("…{}", tail)
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
