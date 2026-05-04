//! Function-local pass infrastructure.
//!
//! Lands the scaffold only: a `FunctionPass` trait, a `PassManager` that
//! drives a pipeline of passes to a fixed point, and hooks for between-pass
//! validation and IR dumping. No real optimization passes — those land in
//! follow-up tickets (constant folding, DCE).
//!
//! Fixed point is *pipeline-level*: passes run in order, the loop repeats
//! until a full round reports no changes. Per-pass fixed-point iteration is
//! deliberately not used because the canonical case against obfuscated input
//! is one pass exposing opportunities for an earlier pass; per-pass
//! convergence would miss those.

// The production pipeline today wires an empty PassManager only: builders
// (`add_pass`, `with_max_iterations`, the dump-sink hook) and the
// `StderrDumpSink` go unused until the first real pass lands. Tests
// exercise the full surface; mirror the `ssa.rs` precedent and silence
// dead-code noise here instead of fabricating shadow uses.
#![allow(dead_code)]

use crate::error::Error;
use crate::ir::ssa::{FuncId, Function, Module, pretty_function};

/// A single function-local pass. Returns `true` iff `func` was mutated.
///
/// Function-local only by design. Inter-procedural work is far enough off
/// that a separate `ModulePass` trait will land alongside the first pass
/// that actually needs it.
pub trait FunctionPass {
    fn name(&self) -> &'static str;
    fn run(&mut self, func: &mut Function) -> Result<bool, Error>;
}

/// Hook for emitting per-pass IR snapshots. The manager invokes this after
/// every pass when a sink is configured. The signature deviates from the
/// ticket's `(pass_name, &Function)` because the SSA pretty-printer needs
/// the module's string interner to resolve `StrId`s; passing
/// `(pass_name, &Module, FuncId)` is the minimum that lets a sink produce
/// useful output.
pub trait DumpSink {
    fn dump(&mut self, pass_name: &str, module: &Module, fid: FuncId);
}

/// Default sink: writes `=== after <pass> ===\n<pretty>\n` to stderr. Kept
/// minimal on purpose — formatting, diffing, and filtering belong to later
/// tickets that have a real reason to want them.
pub struct StderrDumpSink;

impl DumpSink for StderrDumpSink {
    fn dump(&mut self, pass_name: &str, module: &Module, fid: FuncId) {
        eprintln!("=== after {pass_name} ===");
        eprint!("{}", pretty_function(module, fid));
    }
}

pub struct PassManager {
    passes: Vec<Box<dyn FunctionPass>>,
    max_iterations: usize,
    validate_between: bool,
    dump_between: Option<Box<dyn DumpSink>>,
}

impl PassManager {
    /// Default cap of 64 iterations. The cap exists to surface buggy
    /// `changed: true`-forever passes loudly rather than as a real iteration
    /// budget; 64 is far above what a converging pipeline ever needs.
    pub const DEFAULT_MAX_ITERATIONS: usize = 64;

    pub fn new() -> Self {
        Self {
            passes: Vec::new(),
            max_iterations: Self::DEFAULT_MAX_ITERATIONS,
            validate_between: cfg!(debug_assertions),
            dump_between: None,
        }
    }

    pub fn add_pass(&mut self, pass: Box<dyn FunctionPass>) {
        self.passes.push(pass);
    }

    pub fn with_max_iterations(mut self, n: usize) -> Self {
        self.max_iterations = n;
        self
    }

    pub fn with_validate_between(mut self, on: bool) -> Self {
        self.validate_between = on;
        self
    }

    pub fn with_dump_sink(mut self, sink: Box<dyn DumpSink>) -> Self {
        self.dump_between = Some(sink);
        self
    }

    /// Run the configured pipeline on `module.functions[fid]` until fixed
    /// point. Returns the number of outer iterations executed; `0` means the
    /// pipeline was empty and short-circuited without entering the loop.
    pub fn run(&mut self, module: &mut Module, fid: FuncId) -> Result<usize, Error> {
        if self.passes.is_empty() {
            return Ok(0);
        }

        let max_iterations = self.max_iterations;
        let validate_between = self.validate_between;
        let passes = &mut self.passes;
        let dump_between = &mut self.dump_between;

        for iter in 0..max_iterations {
            let mut changed_this_round = false;
            for pass in passes.iter_mut() {
                let changed = {
                    let func = &mut module.functions[fid.0 as usize];
                    pass.run(func)?
                };
                if validate_between && let Err(errs) = module.validate() {
                    return Err(Error::PassValidationFailed {
                        pass: pass.name(),
                        inner: Box::new(Error::Malformed(format!("{errs:#?}"))),
                    });
                }
                if let Some(sink) = dump_between.as_deref_mut() {
                    sink.dump(pass.name(), module, fid);
                }
                changed_this_round |= changed;
            }
            if !changed_this_round {
                return Ok(iter + 1);
            }
        }

        Err(Error::PassFixedPointNotReached {
            iterations: max_iterations,
        })
    }

    /// Convenience: run the pipeline on every function in the module.
    pub fn run_module(&mut self, module: &mut Module) -> Result<(), Error> {
        for i in 0..module.functions.len() {
            self.run(module, FuncId(i as u32))?;
        }
        Ok(())
    }
}

impl Default for PassManager {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// IdentityPass — proves the manager runs and produces zero diffs. Will be
// deleted the moment a real pass lands. `cfg(test)` so it does not ship.
// ============================================================================

#[cfg(test)]
pub mod identity {
    use super::*;

    pub struct IdentityPass;

    impl FunctionPass for IdentityPass {
        fn name(&self) -> &'static str {
            "identity"
        }

        fn run(&mut self, _func: &mut Function) -> Result<bool, Error> {
            Ok(false)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::identity::IdentityPass;
    use super::*;
    use crate::ir::ssa::{BlockId, FunctionBuilder, Module, Terminator, ValueDef};

    fn trivial_module() -> Module {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();
        module
    }

    #[test]
    fn empty_pipeline_runs_with_zero_iterations() {
        let mut module = trivial_module();
        let mut pm = PassManager::new();
        let iters = pm
            .run(&mut module, FuncId(0))
            .expect("empty pipeline must succeed");
        assert_eq!(iters, 0);
    }

    #[test]
    fn empty_pipeline_run_module_succeeds() {
        let mut module = trivial_module();
        let mut pm = PassManager::new();
        pm.run_module(&mut module)
            .expect("empty pipeline must succeed");
    }

    #[test]
    fn identity_pass_only_terminates_after_one_iteration() {
        let mut module = trivial_module();
        let mut pm = PassManager::new();
        pm.add_pass(Box::new(IdentityPass));
        let iters = pm
            .run(&mut module, FuncId(0))
            .expect("identity must succeed");
        assert_eq!(iters, 1);
    }

    struct AlwaysChangesPass;
    impl FunctionPass for AlwaysChangesPass {
        fn name(&self) -> &'static str {
            "always_changes"
        }
        fn run(&mut self, _func: &mut Function) -> Result<bool, Error> {
            Ok(true)
        }
    }

    #[test]
    fn always_changes_pass_hits_iteration_cap() {
        let mut module = trivial_module();
        let mut pm = PassManager::new().with_max_iterations(8);
        pm.add_pass(Box::new(AlwaysChangesPass));
        let err = pm.run(&mut module, FuncId(0)).unwrap_err();
        match err {
            Error::PassFixedPointNotReached { iterations } => assert_eq!(iterations, 8),
            other => panic!("expected PassFixedPointNotReached, got {other:?}"),
        }
    }

    /// Mutates the IR into an invalid state by retargeting the entry block's
    /// terminator at a nonexistent successor.
    struct BreakingPass;
    impl FunctionPass for BreakingPass {
        fn name(&self) -> &'static str {
            "breaking"
        }
        fn run(&mut self, func: &mut Function) -> Result<bool, Error> {
            let entry = func.entry;
            func.blocks[entry.0 as usize].terminator = Terminator::Branch {
                target: BlockId(9999),
                args: vec![],
            };
            Ok(true)
        }
    }

    #[test]
    fn breaking_pass_with_validate_between_yields_validation_failure() {
        let mut module = trivial_module();
        let mut pm = PassManager::new().with_validate_between(true);
        pm.add_pass(Box::new(BreakingPass));
        let err = pm.run(&mut module, FuncId(0)).unwrap_err();
        match err {
            Error::PassValidationFailed { pass, .. } => assert_eq!(pass, "breaking"),
            other => panic!("expected PassValidationFailed, got {other:?}"),
        }
    }

    #[test]
    fn breaking_pass_without_validate_between_does_not_catch() {
        let mut module = trivial_module();
        // Disable validation — the BreakingPass keeps reporting changes,
        // so we expect to hit the iteration cap rather than a validation
        // failure. This pins down that the validator is the only thing
        // that catches the breakage in the previous test.
        let mut pm = PassManager::new()
            .with_validate_between(false)
            .with_max_iterations(4);
        pm.add_pass(Box::new(BreakingPass));
        let err = pm.run(&mut module, FuncId(0)).unwrap_err();
        assert!(matches!(err, Error::PassFixedPointNotReached { .. }));
    }

    /// Pass-name threading: a pass with a distinctive name reports that
    /// name back through `PassValidationFailed`.
    struct NamedBreakingPass {
        name: &'static str,
    }
    impl FunctionPass for NamedBreakingPass {
        fn name(&self) -> &'static str {
            self.name
        }
        fn run(&mut self, func: &mut Function) -> Result<bool, Error> {
            // Same breakage as BreakingPass, but with a configurable name.
            let entry = func.entry;
            func.blocks[entry.0 as usize].terminator = Terminator::Branch {
                target: BlockId(7777),
                args: vec![],
            };
            Ok(true)
        }
    }

    #[test]
    fn validation_failure_threads_pass_name() {
        let mut module = trivial_module();
        let mut pm = PassManager::new().with_validate_between(true);
        pm.add_pass(Box::new(NamedBreakingPass {
            name: "my_custom_pass",
        }));
        let err = pm.run(&mut module, FuncId(0)).unwrap_err();
        match err {
            Error::PassValidationFailed { pass, .. } => assert_eq!(pass, "my_custom_pass"),
            other => panic!("expected PassValidationFailed, got {other:?}"),
        }
    }

    /// `ValueDef` import is only here so that this test crate's accidental
    /// reliance on private items would fail loudly. Keep as a smoke test.
    #[test]
    fn value_def_export_is_visible() {
        let _arg = ValueDef::Arg(0);
    }

    /// Captures the (pass_name, function_pretty_print) pair for every
    /// dump-sink invocation. Used to assert the manager threads pass names
    /// through and dumps after every pass run, including the one that finds
    /// no changes.
    struct CaptureDumpSink {
        log: Rc<RefCell<Vec<(String, String)>>>,
    }
    impl DumpSink for CaptureDumpSink {
        fn dump(&mut self, pass_name: &str, module: &Module, fid: FuncId) {
            self.log
                .borrow_mut()
                .push((pass_name.to_string(), pretty_function(module, fid)));
        }
    }

    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn dump_sink_called_after_each_pass_with_correct_name() {
        let mut module = trivial_module();
        let log: Rc<RefCell<Vec<(String, String)>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = CaptureDumpSink { log: log.clone() };
        let mut pm = PassManager::new()
            .with_dump_sink(Box::new(sink))
            .with_validate_between(false);
        pm.add_pass(Box::new(IdentityPass));
        pm.run(&mut module, FuncId(0)).unwrap();
        let entries = log.borrow();
        assert_eq!(
            entries.len(),
            1,
            "dump_sink should fire exactly once for an IdentityPass-only pipeline"
        );
        assert_eq!(entries[0].0, "identity");
        assert!(
            entries[0].1.contains("fn @f"),
            "dumped pretty-print should include the function header"
        );
    }

    /// Smoke-test that `StderrDumpSink` is constructible and runs against
    /// the manager without panicking. Output goes to stderr; we don't
    /// capture it here.
    #[test]
    fn stderr_dump_sink_runs_without_panic() {
        let mut module = trivial_module();
        let mut pm = PassManager::new()
            .with_dump_sink(Box::new(StderrDumpSink))
            .with_validate_between(false);
        pm.add_pass(Box::new(IdentityPass));
        pm.run(&mut module, FuncId(0)).unwrap();
    }
}
