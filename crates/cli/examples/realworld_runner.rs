//! Real-world stability test runner binary.
//!
//! Run with: cargo run --example realworld_runner -- ./corpus/realworld

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use ir::blocker::BlockerKind;

fn main() {
    let corpus_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../../corpus/realworld"));

    println!("Real-World Stability Test Runner");
    println!("=================================\n");
    println!("Corpus directory: {}\n", corpus_dir.display());

    let mut results = Vec::new();

    // Test all tiers
    for (tier_name, tier_dir) in [
        ("Tier 1", corpus_dir.join("tier1")),
        ("Tier 2", corpus_dir.join("tier2")),
        ("Tier 3", corpus_dir.join("tier3")),
        ("Tier 4", corpus_dir.join("tier4")),
    ] {
        if !tier_dir.exists() {
            continue;
        }

        println!("Testing {}...", tier_name);

        for entry in fs::read_dir(&tier_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "js").unwrap_or(false) {
                let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                print!("  {}... ", file_name);

                let result = test_file_safely(&path);
                let display_str = match &result {
                    Ok(stats) => format!(
                        "OK (parse={}, fb_pt={:.1}%)",
                        stats.parse_ok, stats.fb_pt_pct
                    ),
                    Err(e) => format!("FAIL: {}", e),
                };
                println!("{}", display_str);
                results.push((tier_name.to_string(), file_name, result));
            }
        }
        println!();
    }

    // Generate report
    generate_and_write_report(&results, &corpus_dir);
}

#[derive(Debug)]
struct TestStats {
    parse_ok: bool,
    validate_ok: bool,
    emit_ok: bool,
    reparse_ok: bool,
    roundtrip_ok: bool,
    fb_pt_pct: f64,
    tl_pt_pct: f64,
    error: Option<String>,
    /// Blocker sets for each PassThrough function in this file.
    pt_blocker_sets: Vec<BTreeSet<BlockerKind>>,
}

fn test_file_safely(path: &Path) -> Result<TestStats, String> {
    let source = fs::read_to_string(path).map_err(|e| e.to_string())?;

    // Use catch_unwind to handle panics gracefully
    let result = std::panic::catch_unwind(|| test_file_inner(&source));

    match result {
        Ok(Ok(stats)) => Ok(stats),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("Panic during test (likely unimplemented feature)".to_string()),
    }
}

fn test_file_inner(source: &str) -> Result<TestStats, String> {
    use cli::coverage;
    use cli::emit;
    use cli::lower;

    let mut stats = TestStats {
        parse_ok: false,
        validate_ok: false,
        emit_ok: false,
        reparse_ok: false,
        roundtrip_ok: false,
        fb_pt_pct: 0.0,
        tl_pt_pct: 0.0,
        error: None,
        pt_blocker_sets: Vec::new(),
    };

    // Step 1: Parse
    let tree_module = match lower::parse_and_lower(source) {
        Ok(m) => {
            stats.parse_ok = true;
            m
        }
        Err(e) => {
            stats.error = Some(format!("{:?}", e));
            return Ok(stats);
        }
    };

    // Calculate coverage
    let cov = coverage::analyze(&tree_module);
    stats.fb_pt_pct = calculate_fb_pt_pct(&cov);
    stats.tl_pt_pct = calculate_tl_pt_pct(&cov);
    stats.pt_blocker_sets = cov
        .functions
        .iter()
        .filter(|f| f.disposition == coverage::Disposition::PassThrough)
        .map(|f| f.blockers.clone())
        .collect();

    // Step 2: Lower to SSA and validate
    let ssa_module = cli::ir::lower_ssa::lower_module(&tree_module);
    match ssa_module.validate() {
        Ok(()) => stats.validate_ok = true,
        Err(e) => {
            stats.error = Some(format!("Validation: {:?}", e));
            return Ok(stats);
        }
    }

    // Step 3: Emit
    let tree_back = cli::ir::lower_tree::lower_module(&ssa_module);
    let emitted = emit::emit(&tree_back);
    stats.emit_ok = true;

    // Step 4: Re-parse
    let tree2 = match lower::parse_and_lower(&emitted) {
        Ok(m) => {
            stats.reparse_ok = true;
            m
        }
        Err(e) => {
            stats.error = Some(format!("Re-parse: {:?}", e));
            return Ok(stats);
        }
    };

    // Step 5: Structural roundtrip check.
    // Compare the post-SSA tree-IR (the form emit actually consumed)
    // against the re-parsed emit, NOT the original `tree_module`.
    // SSA decomposition is intentionally lossy in shape; the gate we
    // care about is "emit re-parses to the same shape it was emitted
    // from."
    if let Some(reason) = cli::roundtrip_check::structural_diff(&tree_back, &tree2) {
        stats.error = Some(format!("Roundtrip mismatch: {reason}"));
    } else {
        stats.roundtrip_ok = true;
    }

    Ok(stats)
}

fn calculate_fb_pt_pct(cov: &cli::coverage::ModuleCoverage) -> f64 {
    let mut total_lowered_stmts = 0usize;
    let mut total_pt_bytes = 0usize;

    for row in &cov.functions {
        total_lowered_stmts += row.lowered_stmts;
        total_pt_bytes += row.passthrough_bytes;
    }

    let fn_denom = total_lowered_stmts as f64 * 30.0 + total_pt_bytes as f64;
    if fn_denom > 0.0 {
        (total_pt_bytes as f64 / fn_denom) * 100.0
    } else {
        0.0
    }
}

fn calculate_tl_pt_pct(cov: &cli::coverage::ModuleCoverage) -> f64 {
    let mut total_lowered_stmts = 0usize;
    let mut total_pt_bytes = 0usize;

    for row in &cov.functions {
        total_lowered_stmts += row.lowered_stmts;
        total_pt_bytes += row.passthrough_bytes;
    }

    let tl_bytes = cov.top_level_passthrough_bytes as f64;
    let tl_denom = tl_bytes + total_lowered_stmts as f64 * 30.0 + total_pt_bytes as f64;
    if tl_denom > 0.0 {
        (tl_bytes / tl_denom) * 100.0
    } else {
        0.0
    }
}

fn generate_and_write_report(
    results: &[(String, String, Result<TestStats, String>)],
    corpus_dir: &Path,
) {
    let mut report = String::new();
    report.push_str("# Real-World Stability Test Report\n\n");
    report.push_str(
        "This report tests the parse → IR → emit pipeline against real-world JavaScript.\n\n",
    );

    // Summary
    report.push_str("## Summary\n\n");

    let total = results.len();
    let parse_ok = results
        .iter()
        .filter(|(_, _, r)| r.as_ref().map(|s| s.parse_ok).unwrap_or(false))
        .count();
    let roundtrip_ok = results
        .iter()
        .filter(|(_, _, r)| r.as_ref().map(|s| s.roundtrip_ok).unwrap_or(false))
        .count();
    let panics = results.iter().filter(|(_, _, r)| r.is_err()).count();

    report.push_str(&format!("- Total files tested: {}\n", total));
    report.push_str(&format!(
        "- Parse success: {}/{} ({:.1}%)\n",
        parse_ok,
        total,
        100.0 * parse_ok as f64 / total as f64
    ));
    report.push_str(&format!(
        "- Roundtrip success: {}/{} ({:.1}%)\n",
        roundtrip_ok,
        total,
        100.0 * roundtrip_ok as f64 / total as f64
    ));
    report.push_str(&format!("- Panics/crashes: {}\n\n", panics));

    // Detailed results by tier
    for tier in ["Tier 1", "Tier 2", "Tier 3", "Tier 4"] {
        let tier_results: Vec<_> = results.iter().filter(|(t, _, _)| t == tier).collect();
        if tier_results.is_empty() {
            continue;
        }

        report.push_str(&format!("## {}\n\n", tier));
        report.push_str(
            "| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |\n",
        );
        report.push_str(
            "|------|-------|----------|------|----------|-----------|--------|--------|-------|\n",
        );

        for (_, file_name, result) in tier_results {
            match result {
                Ok(stats) => {
                    let notes = if stats.fb_pt_pct > 20.0 {
                        "High PT%".to_string()
                    } else if !stats.roundtrip_ok && stats.parse_ok {
                        stats.error.clone().unwrap_or_default()
                    } else {
                        String::new()
                    };

                    report.push_str(&format!(
                        "| {} | {} | {} | {} | {} | {} | {:.1}% | {:.1}% | {} |\n",
                        file_name,
                        if stats.parse_ok { "✓" } else { "✗" },
                        if stats.validate_ok { "✓" } else { "✗" },
                        if stats.emit_ok { "✓" } else { "✗" },
                        if stats.reparse_ok { "✓" } else { "✗" },
                        if stats.roundtrip_ok { "✓" } else { "✗" },
                        stats.fb_pt_pct,
                        stats.tl_pt_pct,
                        notes,
                    ));
                }
                Err(e) => {
                    report.push_str(&format!(
                        "| {} | ✗ | ✗ | ✗ | ✗ | ✗ | 0% | 0% | PANIC: {} |\n",
                        file_name, e,
                    ));
                }
            }
        }
        report.push('\n');
    }

    // Failures section
    let failures: Vec<_> = results
        .iter()
        .filter(|(_, _, r)| {
            r.is_err()
                || (r
                    .as_ref()
                    .map(|s| !s.roundtrip_ok && s.parse_ok)
                    .unwrap_or(false))
        })
        .collect();

    if !failures.is_empty() {
        report.push_str("## Issues Found\n\n");

        for (tier, file_name, result) in failures {
            report.push_str(&format!("### {} - {}\n\n", tier, file_name));
            match result {
                Ok(stats) => {
                    if let Some(ref err) = stats.error {
                        report.push_str(&format!("Error: `{}`\n\n", err));
                    }
                }
                Err(e) => {
                    report.push_str(&format!("Panic: `{}`\n\n", e));
                }
            }
        }
    }

    // Decision rules summary
    report.push_str("## Decision Rules Summary\n\n");
    report.push_str("Based on the test results:\n\n");

    if panics > 0 {
        report.push_str(&format!("- ⚠️ **PANICS DETECTED**: {} file(s) caused panics. These are bugs that need to be fixed immediately.\n", panics));
    }

    let tier1_parse_failures = results
        .iter()
        .filter(|(t, _, r)| t == "Tier 1" && r.as_ref().map(|s| !s.parse_ok).unwrap_or(true))
        .count();

    if tier1_parse_failures > 0 {
        report.push_str(&format!("- ⚠️ **Tier 1 parse failures**: {} file(s) failed to parse. These are likely real bugs in the parser.\n", tier1_parse_failures));
    }

    let high_pt = results
        .iter()
        .filter(|(_, _, r)| r.as_ref().map(|s| s.fb_pt_pct > 20.0).unwrap_or(false))
        .count();

    if high_pt > 0 {
        report.push_str(&format!("- ℹ️ **High passthrough**: {} file(s) have >20% function-body passthrough. These should be investigated to identify dominant unlowered constructs.\n", high_pt));
    }

    report.push_str("\n");

    // Corpus-wide blocker analysis: aggregate all PT function blocker sets.
    let corpus_pt_functions: Vec<cli::coverage::FunctionCoverage> = results
        .iter()
        .filter_map(|(_, _, r)| r.as_ref().ok())
        .flat_map(|s| {
            s.pt_blocker_sets.iter().map(|bs| cli::coverage::FunctionCoverage {
                name: String::new(),
                disposition: cli::coverage::Disposition::PassThrough,
                lowered_stmts: 0,
                passthrough_bytes: 0,
                blockers: bs.clone(),
            })
        })
        .collect();
    let corpus_cov = cli::coverage::ModuleCoverage {
        functions: corpus_pt_functions,
        top_level_passthrough_stmts: 0,
        top_level_passthrough_bytes: 0,
    };
    report.push_str(&cli::coverage::format_blocker_table(&corpus_cov));

    let report_path = corpus_dir.join("STABILITY_REPORT.md");
    if let Some(parent) = report_path.parent() {
        fs::create_dir_all(parent).expect("Failed to create report directory");
    }
    fs::write(&report_path, report).expect("Failed to write report");
    println!("Report written to: {}", report_path.display());
}
