//! IR module. Tree-IR / SSA-IR types live in the `ir` crate; the
//! lowering passes and disposition decision live in the `parse` crate.
//! This module re-exports both so existing `crate::ir::tree::…`,
//! `crate::ir::lower_ssa::…`, etc. paths inside mvp keep resolving
//! while the pass manager (still mvp-resident) is migrated out.

pub use ::ir::{free_vars, pt_free_vars, ssa, tree};
pub use ::parse::{disposition, lower_ssa, lower_tree};

pub mod pass;
