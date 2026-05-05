//! IR module. Tree-IR and SSA-IR types now live in the `ir` crate; this
//! module re-exports them so existing `crate::ir::tree::…` paths inside
//! mvp keep resolving while remaining mvp-local IR-adjacent code (the
//! lowering passes, disposition, pass manager) is migrated out.

pub use ::ir::{free_vars, pt_free_vars, ssa, tree};

pub mod disposition;
pub mod lower_ssa;
pub mod lower_tree;
pub mod pass;
