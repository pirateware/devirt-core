//! The deobfuscation pass interface.
//!
//! A pass is one AST→AST rewrite. It owns its visitor(s) and, when asked to
//! `run`, performs a full traversal of the program and reports whether it
//! mutated anything. Each pass builds whatever semantic data it needs itself
//! (via `util::build_scoping` or a full `SemanticBuilder`): some passes need
//! only the arena, others need the symbol table. Building it per-pass — rather
//! than threading one shared analysis through the driver — is the reliable way
//! to stay correct across many mutating passes (a mutation invalidates the
//! previous analysis).
//!
//! Object safety: the arena lifetime `'a` lives on the method, not the trait,
//! so `Box<dyn Pass>` works and the driver can hold a heterogeneous pass list.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;

pub trait Pass {
    /// Stable identifier, used in logs and metrics.
    fn name(&self) -> &'static str;

    /// Run one full traversal. Return `true` iff the AST was changed.
    fn run<'a>(&mut self, program: &mut Program<'a>, allocator: &'a Allocator) -> bool;
}
