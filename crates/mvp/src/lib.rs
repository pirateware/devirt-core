//! Library surface of the js-deobfuscator MVP crate.
//!
//! The CLI (`src/main.rs`) is the primary entry point; the library
//! exists so integration tests under `tests/` (and any future
//! external consumers) can drive the pipeline programmatically without
//! shelling out to the binary.

pub mod emit;
pub mod error;
pub mod ir;
pub mod lower;

#[cfg(test)]
mod tests;

pub const DEMO: &str = r#"
function fibonacci(n) {
        if (n <= 1) return n;
        return fibonacci(n - 1) + fibonacci(n - 2);
    }
"#;

/// Run the full default pipeline (parse → lower → SSA → out-of-SSA →
/// emit). Mirrors the `--via-ssa` path in `main.rs`. Errors are
/// returned rather than printed.
pub fn process(source: &str) -> Result<String, error::Error> {
    let module = lower::parse_and_lower(source)?;
    let mut ssa_module = ir::lower_ssa::lower_module(&module);
    ssa_module
        .validate()
        .map_err(|errs| error::Error::Malformed(format!("SSA validation: {errs:#?}")))?;
    let mut pm = ir::pass::PassManager::new();
    pm.run_module(&mut ssa_module)?;
    let back = ir::lower_tree::lower_module(&ssa_module);
    Ok(emit::emit(&back))
}
