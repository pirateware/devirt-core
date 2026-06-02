//! Pass registry. Add new passes here and to `default_pipeline()`.

mod assigned_once;
mod const_fold;
mod dce;
mod dead_store;
mod decoder_lift;
mod member_normalize;
mod proxy_inline;
mod rename;
mod sequence_split;

pub use assigned_once::AssignedOnceInline;
pub use const_fold::ConstantFold;
pub use dce::Dce;
pub use dead_store::DeadStore;
pub use decoder_lift::DecoderLift;
pub use member_normalize::MemberNormalize;
pub use proxy_inline::ProxyInline;
pub use rename::RenameByRole;
pub use sequence_split::SequenceSplit;

use crate::pass::Pass;

/// The ordered default pipeline. Order matters: cheap syntactic normalizers
/// first (they expose structure), then the dataflow-ish core that compounds in
/// the fixpoint loop (fold → inline → fold → DCE → ...).
pub fn default_pipeline() -> Vec<Box<dyn Pass>> {
    vec![
        Box::new(MemberNormalize::default()),
        // Split statement-level comma chains into one statement per effect, so
        // later passes (fold, inline, DCE) see individual assignments.
        Box::new(SequenceSplit::default()),
        Box::new(ConstantFold::default()),
        Box::new(AssignedOnceInline::default()),
        // Collapse operator-proxy wrappers (a OP b) before lifting, so decoder
        // calls hidden inside proxy expansions become visible.
        Box::new(ProxyInline::default()),
        // Lift after Phase-1 simplification (so decoder args are literals/pure),
        // before DCE (so spent decoders/arrays become dead and get removed).
        Box::new(DecoderLift::default()),
        Box::new(Dce::default()),
        // Remove pure stores to variables that are now read-never (spent
        // decoder-index arithmetic chains).
        Box::new(DeadStore::default()),
        // Rename last: decoder-lift's name-based analysis sees original names
        // first, and we only rename what survives DCE.
        Box::new(RenameByRole::default()),
        // future: Unflatten, ProxyInline, nested-decoder lift, ...
    ]
}
