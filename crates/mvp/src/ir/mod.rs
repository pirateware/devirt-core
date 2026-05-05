//! IR module. Tree-IR / SSA-IR types live in the `ir` crate; the
//! lowering passes and disposition decision live in the `parse` crate;
//! the pass manager lives in the `pass` crate. This module re-exports
//! all three groupings so existing `crate::ir::tree::…`,
//! `crate::ir::lower_ssa::…`, `crate::ir::pass::…` paths inside mvp
//! keep resolving while the remaining mvp surfaces (CLI, coverage,
//! tests) move out.

pub use ::ir::{free_vars, pt_free_vars, ssa, tree};
pub use ::parse::{disposition, lower_ssa, lower_tree};
pub use ::pass as pass;
