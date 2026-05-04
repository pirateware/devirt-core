//! IR module. The tree-IR is the frontend's output shape; the SSA-IR is the
//! analysis-friendly form. Conversion between them comes in a later milestone.

pub mod ssa;
pub mod tree;
