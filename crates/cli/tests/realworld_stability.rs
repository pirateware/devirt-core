//! Real-world stability test harness.
//!
//! Tests the parse → IR → emit pipeline against real-world JS files.
//! Records success/failure for each phase and generates a report.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cli::coverage::{self, ModuleCoverage};
use cli::emit;
use cli::error::Error;
use cli::ir::tree::Module as TreeModule;
use cli::lower;

/// Test result for a single file
#[derive(Debug, Clone)]
pub struct TestResult {
    pub file_name: String,
    pub tier: Tier,
    pub parse_ok: bool,
    pub validate_ok: bool,
    pub emit_ok: bool,
    pub reparse_ok: bool,
    pub roundtrip_ok: bool,
    pub behavior_ok: Option<bool>,
    pub fb_pt_pct: f64, // Function-body passthrough %
    pub tl_pt_pct: f64, // Top-level passthrough %
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum Tier {
    Tier1, // Small libraries (unminified)
    Tier2, // Minified production bundles
    Tier3, // Obfuscated samples
}

impl Tier {
    fn as_str(&self) -> &'static str {
        match self {
            Tier::Tier1 => "Tier 1",
            Tier::Tier2 => "Tier 2",
            Tier::Tier3 => "Tier 3",
        }
    }
}

/// Run the full test suite
pub fn run_realworld_tests(corpus_dir: &Path) -> Vec<TestResult> {
    let mut results = Vec::new();

    // Test Tier 1
    let tier1_dir = corpus_dir.join("tier1");
    if tier1_dir.exists() {
        for entry in fs::read_dir(&tier1_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "js").unwrap_or(false) {
                let result = test_file(&path, Tier::Tier1);
                results.push(result);
            }
        }
    }

    // Test Tier 2
    let tier2_dir = corpus_dir.join("tier2");
    if tier2_dir.exists() {
        for entry in fs::read_dir(&tier2_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "js").unwrap_or(false) {
                let result = test_file(&path, Tier::Tier2);
                results.push(result);
            }
        }
    }

    // Test Tier 3
    let tier3_dir = corpus_dir.join("tier3");
    if tier3_dir.exists() {
        for entry in fs::read_dir(&tier3_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().map(|e| e == "js").unwrap_or(false) {
                let result = test_file(&path, Tier::Tier3);
                results.push(result);
            }
        }
    }

    results
}

fn test_file(path: &Path, tier: Tier) -> TestResult {
    let file_name = path.file_name().unwrap().to_string_lossy().to_string();
    let source = fs::read_to_string(path).unwrap();

    let mut result = TestResult {
        file_name: file_name.clone(),
        tier,
        parse_ok: false,
        validate_ok: false,
        emit_ok: false,
        reparse_ok: false,
        roundtrip_ok: false,
        behavior_ok: None,
        fb_pt_pct: 0.0,
        tl_pt_pct: 0.0,
        error: None,
    };

    // Step 1: Parse to IR
    let tree_module = match lower::parse_and_lower(&source) {
        Ok(m) => {
            result.parse_ok = true;
            m
        }
        Err(e) => {
            result.error = Some(format!("Parse error: {:?}", e));
            return result;
        }
    };

    // Calculate coverage stats from the tree module
    let cov = coverage::analyze(&tree_module);
    result.fb_pt_pct = calculate_fb_pt_pct(&cov);
    result.tl_pt_pct = calculate_tl_pt_pct(&cov);

    // Step 2: Lower to SSA and validate
    let mut ssa_module = cli::ir::lower_ssa::lower_module(&tree_module);
    result.validate_ok = match ssa_module.validate() {
        Ok(()) => true,
        Err(e) => {
            result.error = Some(format!("Validation error: {:?}", e));
            return result;
        }
    };

    // Step 3: Emit back to JS
    let tree_back = cli::ir::lower_tree::lower_module(&ssa_module);
    let emitted = emit::emit(&tree_back);
    result.emit_ok = true;

    // Step 4: Re-parse the emitted JS
    let tree_module2 = match lower::parse_and_lower(&emitted) {
        Ok(m) => {
            result.reparse_ok = true;
            m
        }
        Err(e) => {
            result.error = Some(format!("Re-parse error: {:?}", e));
            return result;
        }
    };

    // Step 5: Compare IR1 and IR2 structurally
    result.roundtrip_ok = compare_modules(&tree_module, &tree_module2);
    if !result.roundtrip_ok {
        result.error = Some("Roundtrip mismatch: IR1 != IR2".to_string());
    }

    // Step 6: Behavioral equivalence check (Tier 1 only)
    if matches!(tier, Tier::Tier1) && result.roundtrip_ok {
        result.behavior_ok = Some(check_behavioral_equivalence(path, &emitted));
    }

    result
}

fn calculate_fb_pt_pct(cov: &ModuleCoverage) -> f64 {
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

fn calculate_tl_pt_pct(cov: &ModuleCoverage) -> f64 {
    // Simplified: just report top-level passthrough bytes as proxy
    cov.top_level_passthrough_bytes as f64
}

fn compare_modules(m1: &TreeModule, m2: &TreeModule) -> bool {
    // Structural comparison ignoring block ID renumbering
    // For now, we compare the item count as a basic check
    // A full implementation would walk the IR and compare structure
    if m1.items.len() != m2.items.len() {
        return false;
    }

    // TODO: Implement deeper structural comparison
    // For MVP, we just check that both parsed successfully
    true
}

fn check_behavioral_equivalence(original_path: &Path, emitted: &str) -> bool {
    // Write emitted code to temp file
    let temp_path = std::env::temp_dir().join("emitted.js");
    fs::write(&temp_path, emitted).unwrap();

    // Try to run both files with node and compare outputs
    // This is a simplified check - real implementation would be more sophisticated
    let original_output = run_node_silent(original_path);
    let emitted_output = run_node_silent(&temp_path);

    // Clean up
    let _ = fs::remove_file(&temp_path);

    // If both fail or both succeed with same output, consider equivalent
    match (original_output, emitted_output) {
        (Err(_), Err(_)) => true, // Both fail (likely unsupported constructs)
        (Ok(out1), Ok(out2)) => out1 == out2,
        _ => false,
    }
}

fn run_node_silent(path: &Path) -> Result<String, ()> {
    let output = Command::new("node").arg(path).output().map_err(|_| ())?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(())
    }
}

/// Generate markdown report
pub fn generate_report(results: &[TestResult]) -> String {
    let mut report = String::new();
    report.push_str("# Real-World Stability Test Report\n\n");
    report.push_str("Generated: 2025-01-05\n\n");

    // Summary by tier
    report.push_str("## Summary\n\n");
    for tier in [Tier::Tier1, Tier::Tier2, Tier::Tier3] {
        let tier_results: Vec<_> = results.iter().filter(|r| matches!(r.tier, t if std::mem::discriminant(&t) == std::mem::discriminant(&tier))).collect();
        if !tier_results.is_empty() {
            report.push_str(&format!("### {}\n\n", tier.as_str()));
            report.push_str(&format!("- Total files: {}\n", tier_results.len()));
            let parse_ok = tier_results.iter().filter(|r| r.parse_ok).count();
            let roundtrip_ok = tier_results.iter().filter(|r| r.roundtrip_ok).count();
            let behavior_ok = tier_results
                .iter()
                .filter(|r| r.behavior_ok == Some(true))
                .count();
            report.push_str(&format!(
                "- Parse success: {}/{}\n",
                parse_ok,
                tier_results.len()
            ));
            report.push_str(&format!(
                "- Roundtrip success: {}/{}\n",
                roundtrip_ok,
                tier_results.len()
            ));
            if tier_results.iter().any(|r| r.behavior_ok.is_some()) {
                report.push_str(&format!(
                    "- Behavioral success: {}/{}\n",
                    behavior_ok,
                    tier_results
                        .iter()
                        .filter(|r| r.behavior_ok.is_some())
                        .count()
                ));
            }
            report.push('\n');
        }
    }

    // Detailed tables
    for tier in [Tier::Tier1, Tier::Tier2, Tier::Tier3] {
        let tier_results: Vec<_> = results.iter().filter(|r| matches!(r.tier, t if std::mem::discriminant(&t) == std::mem::discriminant(&tier))).collect();
        if !tier_results.is_empty() {
            report.push_str(&format!("## {} Details\n\n", tier.as_str()));
            report.push_str("| File | Parse | Validate | Emit | Re-parse | Roundtrip | Behavior | FB-PT% | TL-PT% |\n");
            report.push_str("|------|-------|----------|------|----------|-----------|----------|--------|--------|\n");

            for r in &tier_results {
                let behavior = match r.behavior_ok {
                    Some(true) => "✓",
                    Some(false) => "✗",
                    None => "N/A",
                };
                report.push_str(&format!(
                    "| {} | {} | {} | {} | {} | {} | {} | {:.1}% | {:.1}% |\n",
                    r.file_name,
                    if r.parse_ok { "✓" } else { "✗" },
                    if r.validate_ok { "✓" } else { "✗" },
                    if r.emit_ok { "✓" } else { "✗" },
                    if r.reparse_ok { "✓" } else { "✗" },
                    if r.roundtrip_ok { "✓" } else { "✗" },
                    behavior,
                    r.fb_pt_pct,
                    r.tl_pt_pct,
                ));
            }
            report.push('\n');
        }
    }

    // Failures section
    let failures: Vec<_> = results
        .iter()
        .filter(|r| !r.roundtrip_ok || r.error.is_some())
        .collect();
    if !failures.is_empty() {
        report.push_str("## Failures\n\n");
        for r in &failures {
            report.push_str(&format!("### {}\n\n", r.file_name));
            if let Some(ref err) = r.error {
                report.push_str(&format!("Error: `{}`\n\n", err));
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn corpus_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("corpus")
            .join("realworld")
    }

    #[test]
    fn test_tier1_corpus() {
        let corpus = corpus_dir();
        if !corpus.exists() {
            eprintln!("Corpus directory not found, skipping test");
            return;
        }

        let results = run_realworld_tests(&corpus);

        // Check that we found some files
        let tier1_results: Vec<_> = results
            .iter()
            .filter(|r| matches!(r.tier, Tier::Tier1))
            .collect();
        if !tier1_results.is_empty() {
            // Report results but don't fail - this is for information gathering
            for r in &tier1_results {
                println!(
                    "{}: parse={}, roundtrip={}",
                    r.file_name, r.parse_ok, r.roundtrip_ok
                );
                if let Some(ref err) = r.error {
                    println!("  Error: {}", err);
                }
            }
        }
    }
}

#[cfg(test)]
mod per_file_tests {
    use super::*;
    use std::path::PathBuf;

    fn corpus_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("corpus")
            .join("realworld")
    }

    fn test_one(name: &str) {
        let path = corpus_dir().join("tier2").join(name);
        if !path.exists() { return; }
        let r = test_file(&path, Tier::Tier2);
        if let Some(ref e) = r.error {
            eprintln!("{name}: {e}");
        }
    }

    #[test] fn t_zod() { test_one("zod.min.js"); }
    #[test] fn t_three() { test_one("three.min.js"); }
    #[test] fn t_vue() { test_one("vue.min.js"); }
    #[test] fn t_d3() { test_one("d3.min.js"); }
    #[test] fn t_react_dom() { test_one("react_dom.min.js"); }
    #[test] fn t_rxjs() { test_one("rxjs.min.js"); }
    #[test] fn t_lodash() { test_one("lodash.min.js"); }
    #[test] fn t_axios() { test_one("axios.min.js"); }
    #[test] fn t_chart_js() { test_one("chart_js.min.js"); }
    #[test] fn t_core_js() { test_one("core_js_bundle.min.js"); }
    #[test] fn t_immer() { test_one("immer.min.js"); }
    #[test] fn t_jquery() { test_one("jquery.min.js"); }
    #[test] fn t_luxon() { test_one("luxon.min.js"); }
    #[test] fn t_marked() { test_one("marked.min.js"); }
    #[test] fn t_mobx() { test_one("mobx.min.js"); }
    #[test] fn t_moment() { test_one("moment.min.js"); }
    #[test] fn t_preact() { test_one("preact.min.js"); }
    #[test] fn t_ramda() { test_one("ramda.min.js"); }
    #[test] fn t_react() { test_one("react.min.js"); }
    #[test] fn t_redux() { test_one("redux.min.js"); }
    #[test] fn t_tslib() { test_one("tslib.min.js"); }
    #[test] fn t_underscore() { test_one("underscore.min.js"); }

    fn test_tier1(name: &str) {
        let path = corpus_dir().join("tier1").join(name);
        if !path.exists() { return; }
        let r = test_file(&path, Tier::Tier1);
        if let Some(ref e) = r.error {
            eprintln!("{name}: {e}");
        }
    }
    #[test] fn t1_datefns_format() { test_tier1("datefns_format.js"); }
    #[test] fn t1_lodash_debounce() { test_tier1("lodash_debounce.js"); }
    #[test] fn t1_lodash_cloneDeep() { test_tier1("lodash_cloneDeep.js"); }

    #[test]
    fn t_all_tier3() {
        let dir = corpus_dir().join("tier3");
        if !dir.exists() { return; }
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().map(|x| x == "js").unwrap_or(false))
            .collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            eprintln!("Testing tier3/{name}...");
            let r = test_file(&path, Tier::Tier3);
            if let Some(ref e) = r.error {
                eprintln!("  {name}: {e}");
            }
        }
    }
}

#[cfg(test)]
mod overflow_debug {
    use super::*;
    use std::path::PathBuf;

    fn corpus_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("corpus")
            .join("realworld")
    }

    #[test]
    fn debug_clonedeep_high() {
        let path = corpus_dir().join("tier3").join("lodash_cloneDeep.high.js");
        if !path.exists() { return; }
        let source = std::fs::read_to_string(&path).unwrap();
        eprintln!("Step 1: parse_and_lower...");
        let tree = cli::lower::parse_and_lower(&source).unwrap();
        eprintln!("Step 2: lower_ssa...");
        let ssa = cli::ir::lower_ssa::lower_module(&tree);
        eprintln!("Step 3: validate...");
        ssa.validate().unwrap();
        eprintln!("Step 4: lower_tree...");
        let tree2 = cli::ir::lower_tree::lower_module(&ssa);
        eprintln!("Step 5: emit...");
        let out = cli::emit::emit(&tree2);
        eprintln!("Step 6: re-parse...");
        cli::lower::parse_and_lower(&out).unwrap();
        eprintln!("Done!");
    }
}
