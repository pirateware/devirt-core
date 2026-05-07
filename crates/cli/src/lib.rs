//! Library surface of the js-deobfuscator CLI crate.
//!
//! The binary (`src/main.rs`) is the primary entry point; the library
//! exists so integration tests under `tests/` (and any future external
//! consumers) can drive the pipeline programmatically without shelling
//! out to the binary. Re-exports `ir`, `parse`, `emit`, `pass` under
//! the same paths the previous `mvp` crate did so test and example
//! call sites only need a crate-name swap.

pub use ::emit;
pub use ::ir::error;

pub mod ir {
    pub use ::ir::{free_vars, pt_free_vars, ssa, tree};
    pub use ::parse::{disposition, lower_ssa, lower_tree};
    pub use ::pass;
}

pub use ::parse as lower;
pub mod coverage;
pub mod roundtrip_check;

pub const DEMO: &str = r#"
function asd() {
    var numbers = [1, 2, 3, 4, 5];

    var doubled = numbers.map(function(n) {
        return n * 2;
    });

    var sum = numbers.reduce(function(acc, n) {
        return acc + n;
    }, 0);

    console.log('Original:', numbers);
    console.log('Doubled:', doubled);
    console.log('Sum:', sum);
}
"#;

/// Run the full default pipeline (parse → lower → SSA → out-of-SSA →
/// emit). Mirrors the `--via-ssa` path in `main.rs`. Errors are
/// returned rather than printed.
pub fn process(source: &str) -> Result<String, error::Error> {
    let module = lower::parse_and_lower(source)?;
    let mut ssa_module = ::parse::lower_ssa::lower_module(&module);
    ssa_module
        .validate()
        .map_err(|errs| error::Error::Malformed(format!("SSA validation: {errs:#?}")))?;
    let mut pm = ::pass::PassManager::new();
    pm.add_module_pass(Box::new(::pass::identity_fn_inline::IdentityFnInlinePass));
    pm.add_module_pass(Box::new(
        ::pass::constructor_fn_inline::ConstructorFnInlinePass,
    ));
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
    pm.run_module(&mut ssa_module)?;
    let back = ::parse::lower_tree::lower_module(&ssa_module);
    Ok(::emit::emit(&back))
}
