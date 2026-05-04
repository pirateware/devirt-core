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

mod emit;
mod error;
mod ir;
mod lower;

#[cfg(test)]
mod tests;

use std::env;
use std::fs;
use std::process::ExitCode;

use crate::emit::emit;
use crate::ir::pass::PassManager;
use crate::ir::{lower_ssa, lower_tree, ssa, tree};
use crate::lower::parse_and_lower;

pub(crate) const DEMO: &str = r#"
function fibonacci(n) {
        if (n <= 1) return n;
        return fibonacci(n - 1) + fibonacci(n - 2);
    }
"#;

fn main() -> ExitCode {
    let mut args: Vec<String> = env::args().skip(1).collect();
    let mut via_ssa = true;
    args.retain(|a| match a.as_str() {
        "--via-ssa" => {
            via_ssa = true;
            false
        }
        "--no-ssa" => {
            via_ssa = false;
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

    println!("\n===== TREE-IR =====");
    print!("{}", tree::dump(&module));

    let final_module = if via_ssa {
        let mut ssa_module = lower_ssa::lower_module(&module);
        if let Err(errs) = ssa_module.validate() {
            eprintln!("\ninternal error: SSA module failed validation: {errs:#?}");
            return ExitCode::FAILURE;
        }
        // Pass pipeline runs between SSA build and out-of-SSA. Empty for
        // now — real passes (constant folding, DCE) land in follow-up
        // tickets. The empty pipeline must short-circuit cleanly.
        let mut pm = PassManager::new();
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
