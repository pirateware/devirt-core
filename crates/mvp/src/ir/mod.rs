//! IR module. The tree-IR is the frontend's output shape; the SSA-IR is the
//! analysis-friendly form. Conversion between them comes in a later milestone.

pub mod disposition;
pub mod free_vars;
pub mod lower_ssa;
pub mod lower_tree;
pub mod pass;
pub mod ssa;
pub mod tree;
