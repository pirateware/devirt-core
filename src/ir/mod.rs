//! A CFG / SSA dataflow layer for sound intraprocedural flow transforms.
//!
//! See `SSA_IR_PLAN.md`. The model is: per function body, lower the AST to a
//! minimal CFG that tracks control flow plus the def/use events of *versionable*
//! (non-escaping, non-captured) local symbols; build dominators + SSA over it;
//! and use the resulting reaching-definition facts to drive sound rewrites
//! (const-aggregate member folds, scalar const propagation) applied back onto the
//! original AST by node identity. Anything the lowering can't model ⇒ bail on
//! that function, leaving its AST untouched (partial coverage is always sound).

mod dom;
