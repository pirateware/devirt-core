//! Pass registry. Add new passes here and to `default_pipeline()`.

mod assigned_once;
mod const_fold;
mod dce;
mod dead_object_prop;
mod dead_store;
mod decoder_lift;
mod flatten_else;
mod math_fingerprint;
mod member_normalize;
mod proxy_inline;
mod rename;
mod sequence_split;

pub use assigned_once::AssignedOnceInline;
pub use const_fold::ConstantFold;
pub use dce::Dce;
pub use dead_object_prop::DeadObjectProp;
pub use dead_store::DeadStore;
pub use decoder_lift::DecoderLift;
pub use flatten_else::FlattenElse;
pub use math_fingerprint::MathFingerprint;
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
        Box::new(AssignedOnceInline),
        // Fingerprint algebraic operator-proxy functions (`fn(a,b)` that is really
        // `a+b`, `a^b`, …) by running them in the sandbox and matching a single
        // operator across a battery, then rewrite calls to that operator. Runs
        // before ProxyInline so a polynomial proxy collapses to one operator
        // instead of being inlined verbatim (which only expands it).
        Box::new(MathFingerprint::default()),
        // Fold single-use temps back into their one read (`var t = decode(5);
        // obj[t]` → `obj[decode(5)]`), removing indirection and — the real win —
        // putting expressions back next to each other so proxy-inline, decoder-
        // lift, and the const folds below reach further. Runs before them so the
        // exposed work is picked up in the same round. See `src/ir/inline.rs`.
        Box::new(crate::ir::InlineSingleUse),
        // Collapse operator-proxy wrappers (a OP b) before lifting, so decoder
        // calls hidden inside proxy expansions become visible.
        Box::new(ProxyInline::default()),
        // Lift after Phase-1 simplification (so decoder args are literals/pure),
        // before DCE (so spent decoders/arrays become dead and get removed).
        Box::new(DecoderLift::default()),
        // SSA dataflow layer: sound intraprocedural const/copy propagation and
        // const-aggregate index folding. Runs after lifting (so it sees lifted
        // literals) and before DCE (so spent assignments become dead and get
        // removed). See `src/ir/`.
        Box::new(crate::ir::SsaOpt::default()),
        Box::new(Dce::default()),
        // Remove pure stores to variables that are now read-never (spent
        // decoder-index arithmetic chains).
        Box::new(DeadStore::default()),
        // Drop never-accessed pure members of a non-escaping object literal
        // (uncalled methods left in a mixed proxy/data table after inlining).
        Box::new(DeadObjectProp::default()),
        // De-nest: drop the `else` of an `if` whose consequent always exits.
        // Runs after DCE so dead branches are already gone, and in the loop so
        // `else if` ladders unwind one level per round.
        Box::new(FlattenElse::default()),
        // Rename last: decoder-lift's name-based analysis sees original names
        // first, and we only rename what survives DCE.
        Box::new(RenameByRole::default()),
        // future: Unflatten, ProxyInline, nested-decoder lift, ...
    ]
}
