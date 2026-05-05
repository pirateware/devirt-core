//! IR types and analyses.
//!
//! - `tree`: tree-IR (frontend output / final emission shape).
//! - `ssa`: SSA-IR (analysis-friendly form).
//! - `free_vars`: free-variable analysis over both forms.
//! - `pt_free_vars`: free-variable analysis over PassThrough source slices.

pub mod error;
pub mod free_vars;
pub mod pt_free_vars;
pub mod ssa;
pub mod tree;
