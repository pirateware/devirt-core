//! `report` — readability metrics for the whole sample corpus. Not a test;
//! a dashboard for tracking the pipeline's effect over time. Metrics are
//! sample-agnostic proxies for "how readable is the output":
//!   - bytes / lines (smaller = less dead weight)
//!   - bracket string accesses `["x"]` (lower = more `.x` member access)
//!   - "opaque name ratio": fraction of identifiers that are 1-2 chars or hex
//!     (lower = more meaningful names)
//!   - "hexrefs": raw (non-distinct) occurrence count of `_0x…`-style identifiers
//!     (lower = less decoder residue; spikes when an undecoded string-array
//!     decoder is referenced throughout the output)

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
        "{:<13} {:>9} {:>9} {:>6}  {:>8} {:>8}  {:>8}  {:>6}",
        "sample", "in_bytes", "out_bytes", "kept%", "brackets", "opaque%", "hexrefs", "rounds"
    );
    for p in entries {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let src = fs::read_to_string(&p).unwrap();
        let r = deobfuscate(&src, &name);

        let kept = r.code.len() * 100 / src.len().max(1);
        let brackets = count_bracket_access(&r.code);
        let opaque = opaque_name_ratio(&r.code);
        let hexrefs = count_hex_refs(&r.code);
        println!(
            "{:<13} {:>9} {:>9} {:>5}% {:>8} {:>7}% {:>8}  {:>6}",
            name,
            src.len(),
            r.code.len(),
            kept,
            brackets,
            opaque,
            hexrefs,
            r.rounds
        );
    }

    report_generated_corpus();
}

/// Profile order mirrors `scripts/gen-samples.js`: each is one javascript-obfuscator
/// (obfuscator.io) technique. Unknown profiles are appended in sorted order.
const PROFILE_ORDER: &[&str] = &[
    "minimal",
    "strarr_base64",
    "strarr_rc4",
    "controlflow",
    "deadcode",
    "numbers_keys",
    "strong",
];

/// Aggregate of the obfuscator.io corpus (`samples/generated/`) by profile.
/// Synthetic but obfuscator.io-faithful, so these rows track per-technique
/// readability the same way the real-sample rows track per-target readability.
fn report_generated_corpus() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("generated");
    let Ok(rd) = fs::read_dir(&dir) else {
        return; // corpus not generated locally; nothing to add
    };

    let mut by_profile: std::collections::BTreeMap<String, ProfileAgg> = Default::default();
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|s| s.to_str()) != Some("js") {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let Some(profile) = profile_of(&name) else {
            continue;
        };
        let src = fs::read_to_string(&p).unwrap();
        let r = deobfuscate(&src, &name);
        let agg = by_profile.entry(profile).or_default();
        agg.files += 1;
        agg.in_bytes += src.len();
        agg.out_bytes += r.code.len();
        agg.brackets += count_bracket_access(&r.code);
        agg.opaque_sum += opaque_name_ratio(&r.code);
        agg.hexrefs += count_hex_refs(&r.code);
        agg.rounds_max = agg.rounds_max.max(r.rounds);
        if !r.converged {
            agg.not_converged += 1;
        }
    }
    if by_profile.is_empty() {
        return;
    }

    println!("\nobfuscator.io corpus (samples/generated/) — aggregated by profile");
    println!(
        "{:<13} {:>5} {:>9} {:>9} {:>6}  {:>8} {:>8}  {:>8}  {:>6}  {:>5}",
        "profile",
        "files",
        "in_bytes",
        "out_bytes",
        "kept%",
        "brackets",
        "opaque%",
        "hexrefs",
        "rounds",
        "conv"
    );

    let mut profiles: Vec<String> = by_profile.keys().cloned().collect();
    profiles.sort_by_key(|p| {
        PROFILE_ORDER
            .iter()
            .position(|x| x == p)
            .unwrap_or(PROFILE_ORDER.len())
    });

    let mut total = ProfileAgg::default();
    for profile in &profiles {
        let a = &by_profile[profile];
        print_agg_row(profile, a);
        total.merge(a);
    }
    print_agg_row("ALL", &total);
}

#[derive(Default)]
struct ProfileAgg {
    files: usize,
    in_bytes: usize,
    out_bytes: usize,
    brackets: usize,
    opaque_sum: usize,
    hexrefs: usize,
    rounds_max: usize,
    not_converged: usize,
}

impl ProfileAgg {
    fn merge(&mut self, o: &ProfileAgg) {
        self.files += o.files;
        self.in_bytes += o.in_bytes;
        self.out_bytes += o.out_bytes;
        self.brackets += o.brackets;
        self.opaque_sum += o.opaque_sum;
        self.hexrefs += o.hexrefs;
        self.rounds_max = self.rounds_max.max(o.rounds_max);
        self.not_converged += o.not_converged;
    }
}

fn print_agg_row(label: &str, a: &ProfileAgg) {
    let kept = a.out_bytes * 100 / a.in_bytes.max(1);
    let opaque = a.opaque_sum / a.files.max(1); // mean per-file ratio
    let conv = if a.not_converged == 0 {
        "ok".to_string()
    } else {
        format!("!{}", a.not_converged)
    };
    println!(
        "{:<13} {:>5} {:>9} {:>9} {:>5}% {:>8} {:>7}% {:>8}  {:>6}  {:>5}",
        label,
        a.files,
        a.in_bytes,
        a.out_bytes,
        kept,
        a.brackets,
        opaque,
        a.hexrefs,
        a.rounds_max,
        conv
    );
}

/// `seed_01_arithmetic__strong.js` -> `strong`. Profiles are the segment after
/// the `__` separator (seed names use single `_`, the join uses `__`).
fn profile_of(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".js")?;
    let (_, profile) = stem.rsplit_once("__")?;
    Some(profile.to_string())
}

fn count_bracket_access(s: &str) -> usize {
    // Count `["` only where it is a string-keyed *member access* (`obj["x"]`),
    // not an array/object literal (`= ["x"]`, `, ["x"]`). The discriminator is
    // the byte immediately before `[`: a member access follows an expression
    // (identifier char, `)`, `]`, or a closing quote) with no space, whereas a
    // literal follows an operator/punctuator/space. Excludes the array-literal
    // false positives that dominate the raw `["` count.
    let b = s.as_bytes();
    (1..b.len().saturating_sub(1))
        .filter(|&i| b[i] == b'[' && b[i + 1] == b'"' && is_member_lhs(b[i - 1]))
        .count()
}

fn is_member_lhs(p: u8) -> bool {
    p.is_ascii_alphanumeric() || matches!(p, b'_' | b'$' | b')' | b']' | b'"' | b'\'')
}

/// Raw (non-distinct) occurrence count of `_0x…`-style hex-named identifiers
/// in the output. Counts every token starting with `_0x` followed by hex digits,
/// regardless of how many times the same name appears — so a single undecoded
/// decoder referenced N times contributes N. Lower is better; ~0 means no
/// decoder residue. Spikes clearly on strarr/numbers/strong profiles where the
/// string-array decoder survives intact.
fn count_hex_refs(s: &str) -> usize {
    let mut count = 0usize;
    let mut cur = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
            cur.push(ch);
        } else {
            if cur.starts_with("_0x") && cur.len() > 3 {
                count += 1;
            }
            cur.clear();
        }
    }
    if cur.starts_with("_0x") && cur.len() > 3 {
        count += 1;
    }
    count
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
