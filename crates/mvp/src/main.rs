//! js-deobfuscator MVP CLI.
//!
//! Pipeline:
//!     JS source --[oxc_parser]--> OXC AST --[lower]--> tree-IR --[emit]--> JS source
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
//!     cargo run                    # runs the built-in demo
//!     cargo run -- path/to/file.js # processes the given file

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
use crate::ir::tree;
use crate::lower::parse_and_lower;

pub(crate) const DEMO: &str = r#"
function fibonacci(n) {
        if (n <= 1) return n;
        return fibonacci(n - 1) + fibonacci(n - 2);
    }
"#;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let source = if args.len() >= 2 {
        match fs::read_to_string(&args[1]) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error reading {}: {e}", args[1]);
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

    println!("\n===== IR =====");
    print!("{}", tree::dump(&module));

    println!("===== EMITTED =====");
    print!("{}", emit(&module));

    ExitCode::SUCCESS
}
