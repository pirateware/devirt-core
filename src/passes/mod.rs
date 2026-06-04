//! Pass registry. Add new passes here and to `default_pipeline()`.

mod assigned_once;
mod const_fold;
mod dce;
mod dead_object_prop;
mod dead_store;
mod decoder_lift;
mod demultiplex;
mod flatten_else;
mod global_const_inline;
mod math_fingerprint;
mod member_normalize;
mod proxy_inline;
mod pure_eval;
mod rename;
mod sequence_split;
mod unflatten;
mod unflatten_cfg;
mod unflatten_param;
mod unflatten_string_order;

pub use assigned_once::AssignedOnceInline;
pub use const_fold::ConstantFold;
pub use dce::Dce;
pub use dead_object_prop::DeadObjectProp;
pub use dead_store::DeadStore;
pub use decoder_lift::DecoderLift;
pub use demultiplex::Demultiplex;
pub use flatten_else::FlattenElse;
pub use global_const_inline::GlobalConstInline;
pub use math_fingerprint::MathFingerprint;
pub use member_normalize::MemberNormalize;
pub use proxy_inline::ProxyInline;
pub use pure_eval::PureEval;
pub use rename::RenameByRole;
pub use sequence_split::SequenceSplit;
pub use unflatten::Unflatten;
pub use unflatten_cfg::UnflattenCfg;
pub use unflatten_param::UnflattenParam;
pub use unflatten_string_order::UnflattenStringOrder;

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
        // Inline program-wide write-once literal constants defined in an init
        // function and read elsewhere (`case varN:` → `case 19:`). Runs before
        // `unflatten` so symbolic case labels become numeric and the dispatcher
        // becomes a plain numeric switch the unflattener can act on. The literal
        // values it consumes are produced upstream by decoder-lift's base-N
        // arithmetic materialization, so it fires on a later fixpoint round.
        Box::new(GlobalConstInline),
        // Unflatten the integer-state `while(s){switch(s){…}}` control-flow
        // dispatcher back into the straight-line sequence it encodes. Runs early
        // (after sequence-split splits comma chains in cases into clean top-level
        // transition statements) so the revealed code feeds the folds below.
        Box::new(Unflatten::default()),
        // Interprocedural variant: specialize *param-state* dispatchers (the state
        // is a function parameter, selected by a constant entry at the call site)
        // per entry constant, then route calls to the specialization. Runs right
        // after the local unflattener; the linearized bodies feed the folds below.
        Box::new(UnflattenParam::default()),
        // CFG-based recovery for the *looping* dispatchers the linear unflattener
        // declines (a state that conditionally loops back to itself → do/while).
        // Runs after the linear forms so it only sees what they bailed on.
        Box::new(UnflattenCfg),
        // String-array-order dispatcher (Cloudflare CFF): a `for (a = "…".split(…),
        // i = 0; true;) switch (a[i++]) {…}` whose execution order is the static
        // array order. Linearize by walking the constant key sequence. Runs after the
        // integer-state unflatteners (a different state encoding) and, like them,
        // early so the revealed straight-line code feeds the folds below. The order
        // array becomes a literal once GlobalConstInline / member-normalize resolve
        // the `a[k]["split"]` accesses upstream, so it fires on a later fixpoint round.
        Box::new(UnflattenStringOrder),
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
        // Specialize constant-selector calls into an immutable function-array
        // multiplexer (`A[k](C, …)` → a copy of slot k with selector C bound).
        // Runs after the folds that turn selectors constant and before the next
        // round's const-fold/flatten-else/DCE, which collapse the guard chains the
        // bound selector exposes. See `src/passes/demultiplex.rs`.
        Box::new(Demultiplex),
        // Lift after Phase-1 simplification (so decoder args are literals/pure),
        // before DCE (so spent decoders/arrays become dead and get removed).
        Box::new(DecoderLift::default()),
        // Collapse a self-contained, side-effect-free IIFE (an obfuscated VM /
        // "compute a constant the hard way" blob) to the primitive it produces,
        // by running it in the sandbox. Runs after lifting so any decoder calls
        // inside have already resolved, and before DCE so the now-dead binding
        // (`var VM = 42` once its single read is inlined) gets removed.
        Box::new(PureEval::default()),
        // SSA dataflow layer: sound intraprocedural const/copy propagation and
        // const-aggregate index folding. Runs after lifting (so it sees lifted
        // literals) and before DCE (so spent assignments become dead and get
        // removed). See `src/ir/`.
        Box::new(crate::ir::SsaOpt),
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
        // future: nested-decoder lift, string-array-split unflattening, ...
    ]
}
