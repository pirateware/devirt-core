//! js-deobfuscator MVP CLI.
//!
//! Pipeline (default, `--via-ssa`):
//!     JS source
//!       └─ oxc_parser ─▶ OXC AST
//!            └─ lower ─▶ tree-IR
//!                 └─ lower_ssa ─▶ SSA-IR
//!                      └─ lower_tree ─▶ tree-IR
//!                           └─ emit ─▶ JS source
//!
//! Pipeline (`--no-ssa`, kept for comparison):
//!     JS source ─[parser]─▶ AST ─[lower]─▶ tree-IR ─[emit]─▶ JS source
//!
//! Subset of JS supported (anything outside it returns a structured
//! `Error::Unsupported` rather than silently producing wrong IR):
//!   - number / string / boolean / null / undefined literals
//!   - identifiers, member access (.x, [x])
//!   - binary + unary operators
//!   - function calls
//!   - let / const / plain `=` assignment
//!   - function declarations
//!   - if / else, while, return, block statements
//!
//! Run:
//!     cargo run                          # demo input via SSA pipeline
//!     cargo run -- path/to/file.js       # processes the file via SSA
//!     cargo run -- --no-ssa file.js      # bypass SSA round-trip
//!     cargo run -- --via-ssa file.js     # explicit (default)

use std::env;
use std::fs;
use std::process::ExitCode;

use cli::DEMO;
use cli::coverage;
use cli::emit::emit;
use cli::ir::pass::PassManager;
use cli::ir::{lower_ssa, lower_tree, ssa, tree};
use cli::lower::parse_and_lower;

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut via_ssa = true;
    let mut coverage_stats = false;
    args.retain(|a| match a.as_str() {
        "--via-ssa" => {
            via_ssa = true;
            false
        }
        "--no-ssa" => {
            via_ssa = false;
            false
        }
        "--coverage-stats" => {
            coverage_stats = true;
            false
        }
        _ => true,
    });

    let source = if !args.is_empty() {
        match fs::read_to_string(&args[0]) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error reading {}: {e}", args[0]);
                return ExitCode::FAILURE;
            }
        }
    } else {
        DEMO.to_string()
    };

    println!("===== INPUT =====");
    print!("{source}");
    if !source.ends_with('\n') {
        println!();
    }

    let module = match parse_and_lower(&source) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("\nerror: {e}");
            return ExitCode::FAILURE;
        }
    };

    if coverage_stats {
        let cov = coverage::analyze(&module);
        println!("\n===== COVERAGE =====");
        print!("{}", coverage::format_table(&cov));
        return ExitCode::SUCCESS;
    }

    println!("\n===== TREE-IR =====");
    print!("{}", tree::dump(&module));

    let final_module = if via_ssa {
        let mut ssa_module = lower_ssa::lower_module(&module);
        if let Err(errs) = ssa_module.validate() {
            eprintln!("\ninternal error: SSA module failed validation: {errs:#?}");
            return ExitCode::FAILURE;
        }
        // Pass pipeline runs between SSA build and out-of-SSA. RefineArith
        // first (specialises *Generic into typed variants), then ConstFold
        // (which only folds typed arithmetic). The manager's fixed-point
        // loop handles the refine→fold→refine cascade.
        let mut pm = PassManager::new();
        pm.add_module_pass(Box::new(::pass::identity_fn_inline::IdentityFnInlinePass));
        pm.add_module_pass(Box::new(
            ::pass::constructor_fn_inline::ConstructorFnInlinePass,
        ));
        pm.add_module_pass(Box::new(::pass::partial_eval::PartialEvalPass));
        pm.add_module_pass(Box::new(::pass::dead_fn_elim::DeadFnElimPass));
        pm.add_pass(Box::new(::pass::refine_purity::RefinePurityPass));
        pm.add_pass(Box::new(::pass::escape_analysis::EscapeAnalysisPass));
        pm.add_pass(Box::new(
            ::pass::refine_forin_effect::RefineForInEffectPass,
        ));
        pm.add_pass(Box::new(::pass::refine_arith::RefineArithPass));
        pm.add_pass(Box::new(::pass::const_fold::ConstFoldPass));
        pm.add_pass(Box::new(::pass::gvn::GvnPass));
        pm.add_pass(Box::new(::pass::branch_fold::BranchFoldPass));
        pm.add_pass(Box::new(::pass::copy_prop::CopyPropPass));
        pm.add_pass(Box::new(::pass::forward_loads::ForwardLoadsPass));
        pm.add_pass(Box::new(::pass::licm::LicmPass));
        pm.add_pass(Box::new(::pass::dce::DcePass));
        if let Err(e) = pm.run_module(&mut ssa_module) {
            eprintln!("\ninternal error: pass pipeline failed: {e}");
            return ExitCode::FAILURE;
        }
        println!("===== SSA-IR =====");
        print!("{}", ssa::pretty_module(&ssa_module));
        let back = lower_tree::lower_module(&ssa_module);
        println!("\n===== TREE-IR (post-SSA) =====");
        print!("{}", tree::dump(&back));
        back
    } else {
        module
    };

    println!("===== EMITTED =====");
    print!("{}", emit(&final_module));

    ExitCode::SUCCESS
}
