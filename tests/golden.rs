//! Byte-exact **golden snapshots** of the whole sample corpus — the committed
//! baseline the suite diffs against to catch regressions.
//!
//! The pipeline is deterministic (see `tests/determinism.rs`), so each sample's
//! output is pinned byte-for-byte in `tests/snapshots/<name>.out.js`. Any pass
//! change that moves *any* sample's output fails this test and the PR diff shows
//! exactly what moved — no silent drift across the 13 samples.
//!
//! A companion human-readable scoreboard (`tests/snapshots/SCOREBOARD.md`) tracks
//! the readability metrics (bytes / bracket-access / opaque-name ratio / rounds /
//! convergence) so a reviewer sees the *shape* of a change, not just a byte diff.
//!
//! Updating after an intentional change ("blessing"):
//!
//! ```text
//! UPDATE_SNAPSHOTS=1 cargo test --release --test golden
//! # or: ./scripts/bless.sh
//! ```
//!
//! Review the resulting `tests/snapshots/` diff like any other code change.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use devirt_core::deobfuscate;

fn blessing() -> bool {
    std::env::var("UPDATE_SNAPSHOTS").is_ok_and(|v| v == "1" || v == "true")
}

fn snapshots_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("snapshots")
}

fn sorted_samples() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples");
    let mut out: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("js"))
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(&p).unwrap())
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// First line index (1-based) at which two strings diverge, plus the two lines,
/// for a compact, actionable failure message on large files.
fn first_divergence(expected: &str, actual: &str) -> String {
    for (i, (e, a)) in expected.lines().zip(actual.lines()).enumerate() {
        if e != a {
            return format!(
                "first diff at line {}:\n    expected: {}\n    actual:   {}",
                i + 1,
                truncate(e),
                truncate(a)
            );
        }
    }
    let (el, al) = (expected.lines().count(), actual.lines().count());
    format!(
        "content matches up to line {}, but line counts differ (expected {el}, actual {al})",
        el.min(al)
    )
}

fn truncate(s: &str) -> String {
    if s.len() > 160 {
        format!("{}…", &s[..160])
    } else {
        s.to_string()
    }
}

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: full real-sample corpus. Run with `--features slow-tests` (CI) or `cargo nextest run --run-ignored all`."
)]
fn sample_outputs_match_golden_snapshots() {
    let dir = snapshots_dir();
    fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();

    for (name, src) in sorted_samples() {
        let report = deobfuscate(&src, &name);
        assert!(
            report.parse_errors.is_empty(),
            "{name}: deob produced parse errors: {:?}",
            report.parse_errors
        );
        let path = dir.join(format!("{name}.out.js"));

        if blessing() {
            fs::write(&path, &report.code).unwrap();
            continue;
        }

        match fs::read_to_string(&path) {
            Ok(expected) if expected == report.code => {}
            Ok(expected) => failures.push(format!(
                "{name}: output differs from golden ({} → {} bytes).\n  {}",
                expected.len(),
                report.code.len(),
                first_divergence(&expected, &report.code)
            )),
            Err(_) => failures.push(format!(
                "{name}: golden snapshot missing at {}",
                path.display()
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "golden snapshot mismatches ({} sample(s)). If intended, re-bless:\n  \
         UPDATE_SNAPSHOTS=1 cargo test --release --test golden\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Readability scoreboard — a committed, reviewable metrics table.
// ---------------------------------------------------------------------------

#[test]
#[cfg_attr(
    not(feature = "slow-tests"),
    ignore = "slow: full real-sample corpus. Run with `--features slow-tests` (CI) or `cargo nextest run --run-ignored all`."
)]
fn scoreboard_is_current() {
    let table = render_scoreboard();
    let path = snapshots_dir().join("SCOREBOARD.md");
    fs::create_dir_all(snapshots_dir()).unwrap();

    if blessing() {
        fs::write(&path, &table).unwrap();
        return;
    }

    let expected = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        expected, table,
        "\nSCOREBOARD.md is stale. If the metric changes are intended, re-bless:\n  \
         UPDATE_SNAPSHOTS=1 cargo test --release --test golden\n"
    );
}

fn render_scoreboard() -> String {
    let mut s = String::new();
    s.push_str("# Sample readability scoreboard\n\n");
    s.push_str(
        "Regenerated by `tests/golden.rs` (re-bless with \
         `UPDATE_SNAPSHOTS=1 cargo test --release --test golden`). \
         Tracks the pipeline's effect per sample; `kept%` is output/input bytes \
         (lower = more dead weight removed), `brackets` counts `[\"…\"]` string \
         member access (lower = more `.x`), `opaque%` is the fraction of \
         identifiers that are ≤2 chars or `_0x…` (lower = more meaningful names).\n\n",
    );
    s.push_str(
        "| sample | in_bytes | out_bytes | kept% | brackets | opaque% | rounds | converged |\n",
    );
    s.push_str(
        "|--------|---------:|----------:|------:|---------:|--------:|-------:|:---------:|\n",
    );
    for (name, src) in sorted_samples() {
        let r = deobfuscate(&src, &name);
        let kept = r.code.len() * 100 / src.len().max(1);
        s.push_str(&format!(
            "| {} | {} | {} | {}% | {} | {}% | {} | {} |\n",
            name,
            src.len(),
            r.code.len(),
            kept,
            count_bracket_access(&r.code),
            opaque_name_ratio(&r.code),
            r.rounds,
            if r.converged { "yes" } else { "**no**" },
        ));
    }
    render_generated_corpus(&mut s);
    s
}

/// Profile order mirrors `scripts/gen-samples.js` — each is one
/// javascript-obfuscator (obfuscator.io) technique. Unknown profiles append last.
const PROFILE_ORDER: &[&str] = &[
    "minimal",
    "strarr_base64",
    "strarr_rc4",
    "controlflow",
    "deadcode",
    "numbers_keys",
    "strong",
];

/// Append a per-profile rollup of the obfuscator.io corpus (`samples/generated/`).
/// These are synthetic but obfuscator.io-faithful, and they gate correctness via
/// `manifest.json`; aggregating them here makes them count toward readability too,
/// per technique. `kept%` is byte-weighted (Σout/Σin); `opaque%` is the mean of
/// per-file ratios; `rounds` is the worst case; `converged` flags any non-fixpoint.
fn render_generated_corpus(s: &mut String) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .join("generated");
    let mut by_profile: std::collections::BTreeMap<String, ProfileAgg> = Default::default();
    let Ok(rd) = fs::read_dir(&dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|x| x.to_str()) != Some("js") {
            continue;
        }
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let Some((_, profile)) = name.strip_suffix(".js").and_then(|st| st.rsplit_once("__")) else {
            continue;
        };
        let src = fs::read_to_string(&p).unwrap();
        let r = deobfuscate(&src, &name);
        let a = by_profile.entry(profile.to_string()).or_default();
        a.files += 1;
        a.in_bytes += src.len();
        a.out_bytes += r.code.len();
        a.brackets += count_bracket_access(&r.code);
        a.opaque_sum += opaque_name_ratio(&r.code);
        a.rounds_max = a.rounds_max.max(r.rounds);
        if !r.converged {
            a.not_converged += 1;
        }
    }
    if by_profile.is_empty() {
        return;
    }

    let mut profiles: Vec<String> = by_profile.keys().cloned().collect();
    profiles.sort_by_key(|p| {
        PROFILE_ORDER
            .iter()
            .position(|x| x == p)
            .unwrap_or(PROFILE_ORDER.len())
    });

    s.push_str("\n## obfuscator.io corpus (`samples/generated/`), by profile\n\n");
    s.push_str(
        "Synthetic javascript-obfuscator fixtures (one row per technique, \
         aggregated over all seeds). `kept%` is byte-weighted Σout/Σin; \
         `opaque%` is the mean per-file ratio; `rounds` is the worst case; \
         `converged` is `yes` only if every file reached a fixpoint.\n\n",
    );
    s.push_str(
        "| profile | files | in_bytes | out_bytes | kept% | brackets | opaque% | rounds | converged |\n",
    );
    s.push_str(
        "|---------|------:|---------:|----------:|------:|---------:|--------:|-------:|:---------:|\n",
    );
    let mut total = ProfileAgg::default();
    for profile in &profiles {
        push_agg_row(s, profile, &by_profile[profile]);
        total.merge(&by_profile[profile]);
    }
    push_agg_row(s, "**all**", &total);
}

#[derive(Default)]
struct ProfileAgg {
    files: usize,
    in_bytes: usize,
    out_bytes: usize,
    brackets: usize,
    opaque_sum: usize,
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
        self.rounds_max = self.rounds_max.max(o.rounds_max);
        self.not_converged += o.not_converged;
    }
}

fn push_agg_row(s: &mut String, label: &str, a: &ProfileAgg) {
    let kept = a.out_bytes * 100 / a.in_bytes.max(1);
    let opaque = a.opaque_sum / a.files.max(1);
    s.push_str(&format!(
        "| {} | {} | {} | {} | {}% | {} | {}% | {} | {} |\n",
        label,
        a.files,
        a.in_bytes,
        a.out_bytes,
        kept,
        a.brackets,
        opaque,
        a.rounds_max,
        if a.not_converged == 0 {
            "yes".to_string()
        } else {
            format!("**{} no**", a.not_converged)
        },
    ));
}

fn count_bracket_access(s: &str) -> usize {
    s.matches("[\"").count()
}

/// Fraction (%) of distinct identifier-ish tokens that look obfuscated: length
/// ≤ 2, or hex-style `_0x…`. Mirrors `src/bin/report.rs` so the committed
/// scoreboard and the live dashboard agree.
fn opaque_name_ratio(s: &str) -> usize {
    let mut all: HashSet<String> = HashSet::new();
    let mut opaque: HashSet<String> = HashSet::new();
    let mut cur = String::new();
    let consider = |cur: &mut String, all: &mut HashSet<String>, opaque: &mut HashSet<String>| {
        if cur.is_empty() {
            return;
        }
        let tok = std::mem::take(cur);
        if tok.chars().next().unwrap().is_ascii_digit() {
            return;
        }
        if tok.len() <= 2 || tok.starts_with("_0x") {
            opaque.insert(tok.clone());
        }
        all.insert(tok);
    };
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
