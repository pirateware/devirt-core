//! SSA-form intermediate representation.
//!
//! Block params (no phi nodes), inlined-constant operands, separate
//! Value/Instruction ID spaces joined by `ValueDef`, bitflags-based effect
//! lattice. The textual format produced by `pretty_function` is the binding
//! spec for snapshot tests; deviations are bugs.

// The binary doesn't yet consume this module; conversion and emit will land
// in a later milestone. Tests exercise the full surface, so we silence
// dead-code noise instead of papering over with shadow uses.

use bitflags::bitflags;
use lasso::{Rodeo, Spur};
use std::fmt::Write as _;

// ============================================================================
// IDs
// ============================================================================

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Value(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct InstId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct BlockId(pub u32);

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub struct FuncId(pub u32);

pub type StrId = Spur;

// ============================================================================
// Constants and operands
// ============================================================================

#[derive(Clone, Copy, Debug)]
pub enum Constant {
    Number(f64),
    Int32(i32),
    Bool(bool),
    String(StrId),
    Null,
    Undefined,
    /// A reference to another function in the module. Produced by the SSA
    /// converter for direct function-name bindings; the printer renders it as
    /// `@name`.
    Func(FuncId),
    /// Sentinel for the JS global object. Produced by the unbound-identifier
    /// lowering path so that `console.log` and friends become structured
    /// `LoadProp(GlobalThis, "console")` instead of a panic. Treat this as
    /// opaque: no folding against other constants. The out-of-SSA pass
    /// recognizes it as a load-root and emits the bare property name.
    GlobalThis,
    /// Sentinel for the function's `this` binding. Each function has
    /// its own `this` established by the call site (method receiver,
    /// `new` target, lexical capture for arrows). Treated as opaque:
    /// equivalent only to itself, no folding. The out-of-SSA pass
    /// renders it as the literal `this` token. Correctness depends
    /// on the function-boundary invariant (see `tree::Expr::This`).
    This,
}

#[derive(Clone, Copy, Debug)]
pub enum Operand {
    Value(Value),
    Const(Constant),
    /// Reference to a closure capture, by index into the enclosing
    /// `Function`'s captures list. Only valid inside a function whose
    /// SSA body was produced as the body of a lowered inner function;
    /// resolved at emit time against the outer scope's value flowing
    /// into the matching `AllocClosure` capture slot.
    Capture(usize),
}

// ============================================================================
// Effects
// ============================================================================

bitflags! {
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct Effect: u8 {
        const PURE        = 0;
        const READS_HEAP  = 1 << 0;
        const WRITES_HEAP = 1 << 1;
        const MAY_THROW   = 1 << 2;
        const MAY_DIVERGE = 1 << 3;
        const OPAQUE      = 0b1111;
    }
}

impl Effect {
    /// An unused instruction is removable iff its effect contains none of
    /// these. Pure reads (READS_HEAP without write/throw/diverge) are safely
    /// dead-code-eliminated.
    pub fn removable_when_unused(self) -> bool {
        !self.intersects(Effect::WRITES_HEAP | Effect::MAY_THROW | Effect::MAY_DIVERGE)
    }
}

/// Three-tier classification of property access purity. Sits between the
/// generic `Effect` lattice (which describes the result) and the syntactic
/// instruction kind (which only knows "load" vs "store"). A pass that proves
/// a tighter bound on a specific access site bumps the access up the tiers;
/// the frontend defaults to the most pessimistic tier the access could
/// require.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Purity {
    /// No observable effect beyond a heap read (LoadProp/LoadIndex) or a
    /// heap write (StoreProp/StoreIndex). Implies the proven absence of any
    /// getter/setter, Proxy trap, or coercion that could throw or diverge.
    /// Almost never produced by the frontend; reserved for analyses that can
    /// prove the access target is a plain own data property.
    Pure,
    /// Heap read/write that may throw but cannot otherwise diverge or mutate
    /// other heap state. The canonical use is global-object accesses
    /// (`Math`, `console`, etc.): under sloppy-mode-equivalent semantics
    /// they read/write the global record and may throw on undefined, but
    /// they don't trigger arbitrary user code. CSE across multiple `Throws`
    /// reads of the same key is sound when no intervening instruction has
    /// `WRITES_HEAP`.
    Throws,
    /// Full opacity: getter, setter, Proxy trap, or any access for which
    /// the frontend lacks evidence of narrower behavior. Blocks reordering
    /// and analysis.
    Opaque,
}

impl Purity {
    fn load_effect(self) -> Effect {
        match self {
            Purity::Pure => Effect::READS_HEAP,
            Purity::Throws => Effect::READS_HEAP | Effect::MAY_THROW,
            Purity::Opaque => Effect::OPAQUE,
        }
    }

    fn store_effect(self) -> Effect {
        match self {
            Purity::Pure => Effect::WRITES_HEAP,
            Purity::Throws => Effect::WRITES_HEAP | Effect::MAY_THROW,
            Purity::Opaque => Effect::OPAQUE,
        }
    }

    /// Suffix used by the pretty-printer for non-default tiers. `Opaque` is
    /// the historical default and renders without a suffix to keep existing
    /// snapshots stable.
    fn suffix(self) -> &'static str {
        match self {
            Purity::Pure => "_pure",
            Purity::Throws => "_throws",
            Purity::Opaque => "",
        }
    }
}

// ============================================================================
// Instructions
// ============================================================================

/// Key in an `ObjectWithSpread` entry. Static keys are string IDs;
/// computed keys are operands; spread entries have no key (represented
/// by `None` in the `keys` vec of `ObjectWithSpread`).
#[derive(Clone, Debug)]
pub enum SpreadObjectKey {
    /// `{ foo: val }` — string-interned static key.
    Static(StrId),
    /// `{ [expr]: val }` — computed key whose value is an operand.
    Computed(Operand),
}

#[derive(Clone, Debug)]
pub struct Instruction {
    pub kind: InstKind,
}

#[derive(Clone, Debug)]
pub enum InstKind {
    // Generic arithmetic — frontend always emits these.
    AddGeneric {
        lhs: Operand,
        rhs: Operand,
    },
    SubGeneric {
        lhs: Operand,
        rhs: Operand,
    },
    MulGeneric {
        lhs: Operand,
        rhs: Operand,
    },
    DivGeneric {
        lhs: Operand,
        rhs: Operand,
    },
    ModGeneric {
        lhs: Operand,
        rhs: Operand,
    },
    /// `lhs ** rhs`. Calls ToNumber on both operands; throws on
    /// BigInt/Number mixing. No typed fast-path yet.
    PowGeneric {
        lhs: Operand,
        rhs: Operand,
    },

    // Bitwise binary ops. All call ToInt32 (or ToUint32 for UShr)
    // on both operands and may throw on Symbol/BigInt coercion.
    BitAnd {
        lhs: Operand,
        rhs: Operand,
    },
    BitOr {
        lhs: Operand,
        rhs: Operand,
    },
    BitXor {
        lhs: Operand,
        rhs: Operand,
    },
    Shl {
        lhs: Operand,
        rhs: Operand,
    },
    Shr {
        lhs: Operand,
        rhs: Operand,
    },
    UShr {
        lhs: Operand,
        rhs: Operand,
    },

    // Typed fast-paths — emitted only by the typed-specializer pass.
    AddNumber {
        lhs: Operand,
        rhs: Operand,
    },
    SubNumber {
        lhs: Operand,
        rhs: Operand,
    },
    MulNumber {
        lhs: Operand,
        rhs: Operand,
    },
    AddInt32 {
        lhs: Operand,
        rhs: Operand,
    },
    SubInt32 {
        lhs: Operand,
        rhs: Operand,
    },
    MulInt32 {
        lhs: Operand,
        rhs: Operand,
    },

    ConcatStr {
        lhs: Operand,
        rhs: Operand,
    },

    // Unary arithmetic
    BitNot {
        operand: Operand,
    },

    // Logical
    Not {
        operand: Operand,
    },
    LogAnd {
        lhs: Operand,
        rhs: Operand,
    },
    LogOr {
        lhs: Operand,
        rhs: Operand,
    },

    // Comparisons
    Eq {
        lhs: Operand,
        rhs: Operand,
    }, // ==  (loose; coerces; may throw)
    StrictEq {
        lhs: Operand,
        rhs: Operand,
    }, // === (no coercion)
    Lt {
        lhs: Operand,
        rhs: Operand,
    },
    Le {
        lhs: Operand,
        rhs: Operand,
    },

    // Property access. `purity` records the tightest effect bound the
    // emitter (or a later analysis) can prove for this site. The frontend
    // defaults to `Opaque` for arbitrary-object accesses and `Throws` for
    // global-object accesses. Stores produce no value.
    LoadProp {
        obj: Operand,
        key: StrId,
        purity: Purity,
    },
    StoreProp {
        obj: Operand,
        key: StrId,
        val: Operand,
        purity: Purity,
    },
    LoadIndex {
        obj: Operand,
        index: Operand,
        purity: Purity,
    },
    StoreIndex {
        obj: Operand,
        index: Operand,
        val: Operand,
        purity: Purity,
    },

    // Allocation — pure (a fresh allocation is a fresh identity).
    AllocObject,
    AllocArray {
        length: Option<u32>,
    },
    AllocClosure {
        func: FuncId,
        captures: Vec<Operand>,
    },
    /// Regex literal `/pattern/flags`. Allocates a fresh RegExp
    /// object on each evaluation (literals are not constants per
    /// ES2015+). `source` is the verbatim raw source slice
    /// including delimiters, e.g. `/abc/gi`. PURE: allocation has
    /// fresh identity but no side effects.
    AllocRegExp {
        source: StrId,
    },

    // Calls
    Call {
        callee: Operand,
        args: Vec<Operand>,
    },
    /// Call with at least one spread argument. `args` mirrors the
    /// argument list; `spread_mask` is a bitmask where bit `i` set
    /// means `args[i]` is a spread (`...expr`). Semantics-preserving:
    /// the emitter reconstructs the spread syntax from the mask.
    ///
    /// TODO(analyzability): This node is opaque to passes. Any pass that
    /// wants to reason about argument arity or which args are positional
    /// must give up at CallSpread boundaries. To expand analyzability, the
    /// spread operand would need to be lowered to an iterator protocol
    /// invocation (Symbol.iterator + .next() loop) so each argument slot
    /// becomes a concrete SSA value. Until then, FB-PT% improvements from
    /// spread coverage don't translate to proportional analysis quality gains.
    CallSpread {
        callee: Operand,
        args: Vec<Operand>,
        spread_mask: Vec<bool>,
    },
    CallMethod {
        obj: Operand,
        method: StrId,
        args: Vec<Operand>,
    },
    /// Method call with spread arguments.
    ///
    /// TODO(analyzability): Same opacity caveat as CallSpread — argument
    /// arity is unknown to passes. See CallSpread for the expansion path.
    CallMethodSpread {
        obj: Operand,
        method: StrId,
        args: Vec<Operand>,
        spread_mask: Vec<bool>,
    },
    Construct {
        ctor: Operand,
        args: Vec<Operand>,
    },
    /// Array literal with at least one spread element. `elements` and
    /// `spread_mask` have the same length; bit `i` set means element `i`
    /// is a spread.
    ///
    /// TODO(analyzability): Opaque to passes — array contents and length are
    /// unknown at spread positions. To make array shape analyzable, spread
    /// elements would need to be lowered to iterator protocol calls producing
    /// concrete element slots. Until then, passes must conservatively assume
    /// unknown length and contents when ArrayWithSpread is present.
    ArrayWithSpread {
        elements: Vec<Operand>,
        spread_mask: Vec<bool>,
    },
    /// Object literal with at least one spread property. `keys` and
    /// `vals` have the same length; a `None` key means the entry is a
    /// spread property (`...val`).
    ///
    /// TODO(analyzability): Opaque to passes — object shape is unknown at
    /// spread positions. To make shape analyzable, each spread entry would
    /// need to be expanded via Object.assign or equivalent so each property
    /// becomes a concrete key/value pair in the IR.
    ObjectWithSpread {
        keys: Vec<Option<SpreadObjectKey>>,
        vals: Vec<Operand>,
    },

    // Coercions (explicit — no implicit JS coercions inside the IR).
    ToNumber {
        val: Operand,
    },
    ToString {
        val: Operand,
    },
    ToBool {
        val: Operand,
    },
    ToInt32 {
        val: Operand,
    },

    // Misc
    Typeof {
        val: Operand,
    },
    Instanceof {
        obj: Operand,
        ctor: Operand,
    },
    /// `key in obj` — runtime property-existence check. Equivalent
    /// to `HasProperty(obj, ToPropertyKey(key))` per ECMA-262.
    /// `obj` must be an object (TypeError otherwise); `key` may
    /// throw on Symbol/object coercion.
    In {
        key: Operand,
        obj: Operand,
    },

    // Escape hatch — opaque, blocks all reordering and analysis.
    OpaqueEval {
        source: Operand,
    },

    /// Reads the active exception value at the start of a catch entry
    /// block. MUST be the first instruction of a block reached as
    /// `catch_entry` from a `Terminator::Try`.
    ///
    /// Modeled as Opaque (depends on the implicit exception register;
    /// frontend lacks evidence of narrower behavior). A future
    /// `READS_EXCEPTION_STATE` lattice flag is the right precision tier
    /// when an optimization pass demands it; until then, Opaque is
    /// honest. See docs/design-try-phase-a.md §3.
    CatchValue,

    // -- For-in iteration protocol -----------------------------------------
    //
    // Three opaque iteration instructions implementing JS for-in
    // semantics: walk obj's enumerable string-keyed properties along
    // the prototype chain, lazily, in implementation-defined-but-
    // deterministic order. Reconstruction pattern-matches the standard
    // four-block loop (pre-header / header / body / exit) and re-emits
    // `for (k in obj) body`. See docs/design-forin.md.
    //
    // All three are Opaque purity (iteration touches getters / Proxy
    // traps / can throw / observes mutations during iteration). A
    // future `MAY_OBSERVE_GETTERS` lattice flag is the right precision
    // tier when an optimization pass demands it; until then, Opaque is
    // honest.

    /// Initialize a for-in iterator over `obj`. Returns an opaque
    /// iterator handle threaded through `ForInDone` / `ForInNext`.
    /// The handle's representation is not exposed; passes treat it
    /// as opaque.
    ForInInit { obj: Operand },

    /// Test whether the iterator has produced all keys. Returns Bool
    /// (`true` when iteration is exhausted). Idiomatically used as the
    /// loop's exit condition: `CondBranch !done, body, exit`.
    ForInDone { iter: Operand },

    /// Advance the iterator and return the next key as a string Value.
    /// MUST be called only when the matching `ForInDone` has returned
    /// `false` for the same iterator; calling on an exhausted iterator
    /// is undefined (in practice the JS engine panics at re-emit time).
    /// Reconstruction enforces this by pattern-matching the four-block
    /// shape — out-of-pattern construction would fail to round-trip.
    ForInNext { iter: Operand },
}

impl InstKind {
    pub fn effect(&self) -> Effect {
        use InstKind::*;
        match self {
            // Pure: typed arithmetic and logical/structural ops with no coercion.
            AddNumber { .. }
            | SubNumber { .. }
            | MulNumber { .. }
            | AddInt32 { .. }
            | SubInt32 { .. }
            | MulInt32 { .. }
            | ConcatStr { .. }
            | Not { .. }
            | LogAnd { .. }
            | LogOr { .. }
            | StrictEq { .. }
            | AllocObject
            | AllocArray { .. }
            | AllocClosure { .. }
            | AllocRegExp { .. }
            | ToBool { .. }
            | Typeof { .. } => Effect::PURE,

            // May throw: generic arithmetic and coercions invoke valueOf/toString.
            AddGeneric { .. }
            | SubGeneric { .. }
            | MulGeneric { .. }
            | DivGeneric { .. }
            | ModGeneric { .. }
            | PowGeneric { .. }
            | BitAnd { .. }
            | BitOr { .. }
            | BitXor { .. }
            | Shl { .. }
            | Shr { .. }
            | UShr { .. }
            | Eq { .. }
            | Lt { .. }
            | Le { .. }
            | BitNot { .. }
            | ToNumber { .. }
            | ToString { .. }
            | ToInt32 { .. }
            | Instanceof { .. }
            | In { .. } => Effect::MAY_THROW,

            // Heap reads: dispatched by the per-site Purity tier.
            LoadProp { purity, .. } | LoadIndex { purity, .. } => purity.load_effect(),

            // Stores: dispatched by the per-site Purity tier. Defaults to
            // Opaque (setter can do anything) unless an analysis proves
            // narrower behavior.
            StoreProp { purity, .. } | StoreIndex { purity, .. } => purity.store_effect(),

            // Calls: opaque by default. A future analysis can refine specific calls.
            Call { .. }
            | CallSpread { .. }
            | CallMethod { .. }
            | CallMethodSpread { .. }
            | Construct { .. }
            | OpaqueEval { .. } => Effect::OPAQUE,

            // CatchValue: reads the implicit exception register.
            // Opaque per design — see docs/design-try-phase-a.md §3.
            CatchValue => Effect::OPAQUE,

            // For-in iteration: opaque per design — getters / Proxy
            // traps / can throw / observe mutations. See docs/design-forin.md.
            ForInInit { .. } | ForInDone { .. } | ForInNext { .. } => Effect::OPAQUE,

            // Spread collections: opaque (iterator protocol can throw and mutate heap).
            ArrayWithSpread { .. } | ObjectWithSpread { .. } => Effect::OPAQUE,
        }
    }

    pub fn is_void(&self) -> bool {
        matches!(
            self,
            InstKind::StoreProp { .. } | InstKind::StoreIndex { .. }
        )
    }
}

// ============================================================================
// Terminators
// ============================================================================

#[derive(Clone, Debug)]
pub enum Terminator {
    Branch {
        target: BlockId,
        args: Vec<Operand>,
    },
    CondBranch {
        cond: Operand,
        then_b: BlockId,
        then_args: Vec<Operand>,
        else_b: BlockId,
        else_args: Vec<Operand>,
    },
    Switch {
        val: Operand,
        cases: Vec<SwitchCase>,
        /// Branch taken when no case key matches `val`. When the source
        /// has no `default:` clause, this points directly at `join` (no
        /// synthesized intermediate block). When the source has a
        /// `default:`, this points at the default-body block, which
        /// itself terminates in break/return/throw/continue — the same
        /// rule that applies to every case body.
        default: BlockId,
        default_args: Vec<Operand>,
        /// The post-switch landing block — where execution lands when
        /// any case breaks out of the switch. Tracked as a landmark
        /// because it is not always derivable from `default` (when
        /// source has a `default:` clause, `default != join`) and not
        /// always reachable (when every case path terminates in
        /// return/throw/continue, `join` is dead but still serves as
        /// the converter's current_block for code after the switch).
        /// Not a successor edge — the walker / successors() / edge_args
        /// helpers don't traverse it.
        join: BlockId,
    },
    Return {
        val: Option<Operand>,
    },
    Throw {
        val: Operand,
    },
    /// `try { try_body } catch (catch_param) { catch_body }` framing.
    /// Phase A — try+catch only, no finally. Same structural-landmark
    /// shape as Switch's `default` / `join`.
    ///
    /// Implicit-throw approximation: throws from inside the try-body to
    /// the catch handler are NOT modeled as per-block CFG edges.
    /// Instead this terminator declares the relationship structurally:
    /// "any throw between `try_entry` and `join` is caught at
    /// `catch_entry`." The CFG successor relation reports both
    /// `try_entry` and `catch_entry` as direct successors of the block
    /// terminating in `Terminator::Try`. See `docs/design-try-phase-a.md`
    /// for the implicit-throw approximation's correctness implications
    /// (catch cannot see SSA values defined inside try-body; catch
    /// reads of try-body-mutated symbols go through the
    /// try-address-taken set).
    Try {
        try_entry: BlockId,
        try_args: Vec<Operand>,
        /// `Some(_)` for try+catch (with or without finally); `None`
        /// for try+finally with no catch (Phase B).
        catch_entry: Option<BlockId>,
        /// `Some(_)` for try+finally (with or without catch — Phase B);
        /// `None` for Phase A's try+catch without finally.
        finally_entry: Option<BlockId>,
        /// Post-try landing block. With finally: try-body's tail and
        /// catch-body's tail (when present) branch to `finally_entry`;
        /// finally-body's tail branches to `join`. Without finally
        /// (Phase A shape): try-body's tail and catch-body's tail
        /// branch to `join` directly. Tracked as a landmark, like
        /// Switch's `join`. Not a successor edge — the walker /
        /// successors() helpers don't traverse it.
        join: BlockId,
    },
    Unreachable,
}

#[derive(Clone, Debug)]
pub struct SwitchCase {
    pub key: Constant,
    pub target: BlockId,
    pub args: Vec<Operand>,
}

impl Terminator {
    /// (target, edge-args) for every outgoing edge.
    pub fn successors(&self) -> Vec<(BlockId, &[Operand])> {
        match self {
            Terminator::Branch { target, args } => vec![(*target, args.as_slice())],
            Terminator::CondBranch {
                then_b,
                then_args,
                else_b,
                else_args,
                ..
            } => vec![
                (*then_b, then_args.as_slice()),
                (*else_b, else_args.as_slice()),
            ],
            Terminator::Switch {
                cases,
                default,
                default_args,
                ..
            } => {
                let mut s: Vec<(BlockId, &[Operand])> = cases
                    .iter()
                    .map(|c| (c.target, c.args.as_slice()))
                    .collect();
                s.push((*default, default_args.as_slice()));
                s
            }
            Terminator::Try {
                try_entry,
                try_args,
                catch_entry,
                finally_entry,
                ..
            } => {
                let mut s: Vec<(BlockId, &[Operand])> =
                    vec![(*try_entry, try_args.as_slice())];
                // catch_entry takes no edge args — its first instruction
                // is `InstKind::CatchValue`, which reads the implicit
                // exception value as an instruction result (not a
                // block-arg / phi). See docs/design-try-phase-a.md §1.
                if let Some(c) = catch_entry {
                    s.push((*c, [].as_slice()));
                }
                // finally_entry similarly takes no edge args — control
                // arrives via Branch from try-body's tail and (when
                // catch is present) catch-body's tail; both Branches
                // carry empty args.
                if let Some(f) = finally_entry {
                    s.push((*f, [].as_slice()));
                }
                s
            }
            Terminator::Return { .. } | Terminator::Throw { .. } | Terminator::Unreachable => {
                vec![]
            }
        }
    }
}

// ============================================================================
// Blocks, functions, modules
// ============================================================================

#[derive(Clone, Debug)]
pub struct BasicBlock {
    pub params: Vec<Value>,
    pub instructions: Vec<InstId>,
    pub terminator: Terminator,
    pub predecessors: Vec<BlockId>,
}

#[derive(Clone, Debug)]
pub enum ValueDefKind {
    Inst(InstId),
    BlockParam {
        block: BlockId,
        index: u32,
    },
    Arg(u32),
    /// A retired Value slot. The SSA converter's trivial-param removal
    /// retires block-param Values once their uses are replaced; we cannot
    /// reuse the Value index without renumbering, so the slot is marked
    /// `Removed` and skipped by validation/printing.
    Removed,
}

/// Per-Value definition record. `source_name`, when present, names the
/// source-level binding this value first held — set by the front-end for
/// `let`/`const` bindings and function parameters, and consumed by the
/// out-of-SSA renamer to preserve original names. None for synthesized
/// intermediates (block params, expression sub-results) and for retired
/// slots.
#[derive(Clone, Debug)]
pub struct ValueDef {
    pub kind: ValueDefKind,
    pub source_name: Option<StrId>,
}

impl ValueDef {
    pub fn new(kind: ValueDefKind) -> Self {
        Self {
            kind,
            source_name: None,
        }
    }
    pub fn inst(id: InstId) -> Self {
        Self::new(ValueDefKind::Inst(id))
    }
    pub fn block_param(block: BlockId, index: u32) -> Self {
        Self::new(ValueDefKind::BlockParam { block, index })
    }
    pub fn arg(idx: u32) -> Self {
        Self::new(ValueDefKind::Arg(idx))
    }
    pub fn removed() -> Self {
        Self::new(ValueDefKind::Removed)
    }
    pub fn is_removed(&self) -> bool {
        matches!(self.kind, ValueDefKind::Removed)
    }
}

pub struct Function {
    pub name: String,
    /// Number of captures this function takes from its enclosing scope.
    /// Always 0 for top-level functions. Inside a Lowered body, valid
    /// `Operand::Capture(i)` indices satisfy `i < capture_count`.
    pub capture_count: usize,
    pub body: FunctionBody,
    /// Value indices (within this function's Lowered body) that flow
    /// — directly or transitively — into a PassThrough closure body via
    /// some `AllocClosure` capture slot. The PassThrough body refers to
    /// these values lexically by their `source_name`, so the renamer
    /// must pin them: see `EmitConfig.immovable_names`. Always empty for
    /// PassThrough functions.
    pub captured_by_passthrough: Vec<u32>,
    /// Capture-slot indices on THIS function whose value, in the
    /// enclosing scope, must be treated as captured-by-passthrough.
    /// Populated when this Lowered function's body contains an
    /// `AllocClosure` whose capture operand was `Operand::Capture(idx)`
    /// and the target was PassThrough (or transitively reaches one).
    /// The fixed-point pass after lowering walks every AllocClosure
    /// targeting THIS function and propagates each flagged index back
    /// into the calling function's `captured_by_passthrough` (if the
    /// operand was a Value) or onward set (if it was a Capture).
    pub captures_pass_through_onward: Vec<usize>,
    /// Source-level names (interned StrId) that need an explicit
    /// `let name;` declaration at the top of this function's emitted
    /// body. Populated when the SSA lowering encounters a `Stmt::Let`
    /// or `Stmt::Const` whose name is referenced by a nested
    /// PassThrough body — the binding's value is also synthesized as
    /// a `StoreProp(GlobalThis, name, val)` so the assignment surfaces
    /// textually (lexical resolution then routes the write to this
    /// local let, not a global). Without the declaration, the
    /// synthesized assignment would silently become a global write.
    pub forced_let_decls: Vec<StrId>,
}

pub enum FunctionBody {
    Lowered(LoweredBody),
    PassThrough(PassThroughBody),
}

pub struct LoweredBody {
    pub params: Vec<Value>,
    pub blocks: Vec<BasicBlock>,
    pub instructions: Vec<Instruction>,
    pub values: Vec<ValueDef>,
    pub entry: BlockId,
}

/// Source-preserved function body. We hold an owned slice of the original
/// JS source covering the body (between `{` and `}`, exclusive of the
/// braces themselves), plus enough metadata to synthesize the function
/// header at emit time. Captured operands live on the `AllocClosure` in
/// the outer function — they are NOT stored here.
pub struct PassThroughBody {
    pub source: String,
    pub param_names: Vec<StrId>,
    pub function_name: Option<StrId>,
    pub is_async: bool,
    pub is_generator: bool,
    pub is_arrow: bool,
    /// True iff the original arrow had an expression body (no surrounding
    /// braces). Always false for non-arrow functions. Influences the
    /// re-parse wrapper at emit time.
    pub body_is_expression: bool,
}

impl Function {
    /// Borrow the lowered body. Panics if this is a PassThrough function.
    /// Most call sites in the lowering and pass pipeline only ever see
    /// Lowered bodies; sites that need to handle both should match on
    /// `&self.body` directly.
    pub fn lowered(&self) -> &LoweredBody {
        match &self.body {
            FunctionBody::Lowered(b) => b,
            FunctionBody::PassThrough(_) => {
                panic!("expected Lowered body for function `{}`", self.name)
            }
        }
    }
    pub fn lowered_mut(&mut self) -> &mut LoweredBody {
        match &mut self.body {
            FunctionBody::Lowered(b) => b,
            FunctionBody::PassThrough(_) => {
                panic!("expected Lowered body for function `{}`", self.name)
            }
        }
    }
    pub fn as_lowered(&self) -> Option<&LoweredBody> {
        match &self.body {
            FunctionBody::Lowered(b) => Some(b),
            FunctionBody::PassThrough(_) => None,
        }
    }
    pub fn is_pass_through(&self) -> bool {
        matches!(self.body, FunctionBody::PassThrough(_))
    }
}

pub struct Module {
    pub functions: Vec<Function>,
    pub strings: Rodeo,
    /// Module-level item order, preserved across the SSA round-trip.
    /// `lower_ssa` populates this from the tree-IR; `lower_tree` walks
    /// it to interleave functions with passthrough top-level
    /// statements in the original source order. Empty layout means
    /// "fall back to functions-only emission" (used by hand-built
    /// tests that don't go through the frontend).
    pub top_level_layout: Vec<TopLevelEntry>,
}

#[derive(Clone, Debug)]
pub enum TopLevelEntry {
    /// References a function by its `FuncId`. The `lower_tree`
    /// emitter looks up `module.functions[id.0]` and chooses
    /// `Item::Function` or `Item::PassThroughFunction` based on the
    /// function body kind.
    Function(FuncId),
    /// Verbatim source text for a non-function top-level statement
    /// (IIFE, expression statement, top-level `for` / `class`, etc.).
    /// Re-emitted as `Item::PassThroughStmt` without modification.
    PassThroughStmt(String),
}

impl Module {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            strings: Rodeo::new(),
            top_level_layout: Vec::new(),
        }
    }

    pub fn function(&self, id: FuncId) -> &Function {
        &self.functions[id.0 as usize]
    }
}

impl Default for Module {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Builder
// ============================================================================

pub struct FunctionBuilder<'m> {
    pub module: &'m mut Module,
    pub name: String,
    pub capture_count: usize,
    pub body: LoweredBody,
    current: Option<BlockId>,
    /// Side-table populated by the SSA converter as it emits
    /// AllocClosure into PassThrough targets. Drained into
    /// `Function.captured_by_passthrough` at `finish()`.
    pub captured_by_passthrough: Vec<u32>,
    /// Side-table populated for Operand::Capture(idx) flows into
    /// PassThrough captures. Drained at `finish()` into
    /// `Function.captures_pass_through_onward`.
    pub captures_pass_through_onward: Vec<usize>,
    /// Side-table populated when the converter sees a `Stmt::Let` /
    /// `Stmt::Const` whose name is referenced by a nested PassThrough
    /// body. Drained at `finish()` into `Function.forced_let_decls`.
    pub forced_let_decls: Vec<StrId>,
}

impl<'m> FunctionBuilder<'m> {
    pub fn new(module: &'m mut Module, name: impl Into<String>, num_args: u32) -> Self {
        Self::new_with_captures(module, name, num_args, 0)
    }

    pub fn new_with_captures(
        module: &'m mut Module,
        name: impl Into<String>,
        num_args: u32,
        capture_count: usize,
    ) -> Self {
        let mut body = LoweredBody {
            params: Vec::with_capacity(num_args as usize),
            blocks: Vec::new(),
            instructions: Vec::new(),
            values: Vec::new(),
            entry: BlockId(0),
        };
        for i in 0..num_args {
            let v = Value(body.values.len() as u32);
            body.values.push(ValueDef::arg(i));
            body.params.push(v);
        }
        Self {
            module,
            name: name.into(),
            capture_count,
            body,
            current: None,
            captured_by_passthrough: Vec::new(),
            captures_pass_through_onward: Vec::new(),
            forced_let_decls: Vec::new(),
        }
    }

    /// Mark a Value as flowing into a PassThrough body via some
    /// `AllocClosure` capture slot. Idempotent. The constraint that
    /// the Value must have a `source_name` is enforced at `finish()`
    /// rather than here, because the call sites that mark
    /// (alloc_closure_operand) tag the binding's source name in a
    /// later step within the same lowering.
    pub fn mark_value_captured_by_passthrough(&mut self, v: Value) {
        if !self.captured_by_passthrough.contains(&v.0) {
            self.captured_by_passthrough.push(v.0);
        }
    }

    /// Mark a capture slot on THIS function as flowing into a
    /// PassThrough body in some inner closure. Idempotent.
    pub fn mark_capture_pass_through_onward(&mut self, idx: usize) {
        if !self.captures_pass_through_onward.contains(&idx) {
            self.captures_pass_through_onward.push(idx);
        }
    }

    /// Reserve a top-of-function `let name;` declaration for a name
    /// referenced lexically by a nested PassThrough body. Idempotent.
    /// Caller passes the source-level name; we intern it here so the
    /// converter doesn't have to.
    pub fn mark_forced_let_decl(&mut self, name: &str) {
        let id = self.intern(name);
        if !self.forced_let_decls.contains(&id) {
            self.forced_let_decls.push(id);
        }
    }

    pub fn current_block(&self) -> Option<BlockId> {
        self.current
    }

    pub fn intern(&mut self, s: &str) -> StrId {
        self.module.strings.get_or_intern(s)
    }

    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.body.blocks.len() as u32);
        self.body.blocks.push(BasicBlock {
            params: Vec::new(),
            instructions: Vec::new(),
            terminator: Terminator::Unreachable,
            predecessors: Vec::new(),
        });
        if self.body.blocks.len() == 1 {
            self.body.entry = id;
        }
        id
    }

    pub fn switch_to_block(&mut self, b: BlockId) {
        self.current = Some(b);
    }

    pub fn add_block_param(&mut self, block: BlockId) -> Value {
        let index = self.body.blocks[block.0 as usize].params.len() as u32;
        let v = Value(self.body.values.len() as u32);
        self.body.values.push(ValueDef::block_param(block, index));
        self.body.blocks[block.0 as usize].params.push(v);
        v
    }

    pub fn add_inst(&mut self, kind: InstKind) -> Option<Value> {
        let block = self
            .current
            .expect("no current block; call switch_to_block first");
        let inst_id = InstId(self.body.instructions.len() as u32);
        let void = kind.is_void();
        self.body.instructions.push(Instruction { kind });
        self.body.blocks[block.0 as usize]
            .instructions
            .push(inst_id);
        if void {
            None
        } else {
            let v = Value(self.body.values.len() as u32);
            self.body.values.push(ValueDef::inst(inst_id));
            Some(v)
        }
    }

    /// Tag the most recently allocated Value with a source-level name.
    /// Caller passes the Value returned by `add_inst` (or any other
    /// builder method that produces a Value); the renamer will prefer
    /// this name when emitting the binding for that value.
    pub fn set_source_name(&mut self, v: Value, name: StrId) {
        self.body.values[v.0 as usize].source_name = Some(name);
    }

    pub fn set_terminator(&mut self, term: Terminator) {
        let block = self.current.expect("no current block");
        self.body.blocks[block.0 as usize].terminator = term;
    }

    /// Borrow the in-progress body. Equivalent to `Function::lowered`
    /// for callers that need to inspect blocks/values during build.
    pub fn body(&self) -> &LoweredBody {
        &self.body
    }
    pub fn body_mut(&mut self) -> &mut LoweredBody {
        &mut self.body
    }

    pub fn finish(mut self) -> FuncId {
        self.body.rebuild_predecessors();
        // Constraint: every captured-by-passthrough Value must carry a
        // source_name. PassThrough bodies refer to their captures by
        // lexical name, so a synthesized intermediate (no name) cannot
        // possibly be the right thing to capture into one. Bug in the
        // lowering, not the input — surface loudly.
        for &vi in &self.captured_by_passthrough {
            let def = &self.body.values[vi as usize];
            if def.source_name.is_none() {
                panic!(
                    "Value %{vi} is captured into a PassThrough body but has no source_name. \
                     A synthesized intermediate cannot be captured by a PassThrough body — \
                     the lowering must capture only named source-level bindings."
                );
            }
        }
        let fid = FuncId(self.module.functions.len() as u32);
        self.module.functions.push(Function {
            name: self.name,
            capture_count: self.capture_count,
            body: FunctionBody::Lowered(self.body),
            captured_by_passthrough: self.captured_by_passthrough,
            captures_pass_through_onward: self.captures_pass_through_onward,
            forced_let_decls: self.forced_let_decls,
        });
        fid
    }
}

// ============================================================================
// Validator
// ============================================================================

#[derive(Debug)]
pub struct ValidationError {
    pub func: FuncId,
    pub kind: ValidationErrorKind,
}

#[derive(Debug)]
pub enum ValidationErrorKind {
    EntryHasPredecessors {
        block: BlockId,
    },
    PredecessorMismatch {
        from: BlockId,
        to: BlockId,
        direction: &'static str,
    },
    EdgeArityMismatch {
        from: BlockId,
        to: BlockId,
        expected: usize,
        got: usize,
    },
    DanglingValue {
        value: Value,
    },
    DanglingBlock {
        block: BlockId,
    },
    DanglingInst {
        inst: InstId,
    },
    DanglingFunc {
        func: FuncId,
    },
    SwitchDuplicateKey {
        block: BlockId,
        key_index: usize,
    },
    BlockParamMisregistered {
        value: Value,
    },
    OperandReferencesRemovedValue {
        value: Value,
    },
    EmptyPassThroughSource,
    /// Capture index out of range for the enclosing function's
    /// declared `capture_count`.
    DanglingCapture {
        index: usize,
    },
}

impl Module {
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();
        let num_funcs = self.functions.len();
        for (i, f) in self.functions.iter().enumerate() {
            f.validate_into(FuncId(i as u32), num_funcs, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

impl Function {
    fn validate_into(&self, fid: FuncId, num_funcs: usize, errors: &mut Vec<ValidationError>) {
        let body = match &self.body {
            FunctionBody::Lowered(b) => b,
            FunctionBody::PassThrough(pt) => {
                // PassThrough validity: source is non-empty. The validator
                // does NOT parse or walk into the body slice — that is the
                // emitter's job, and any malformed source is caught there.
                if pt.source.is_empty() {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::EmptyPassThroughSource,
                    });
                }
                return;
            }
        };
        let block_count = body.blocks.len();
        let inst_count = body.instructions.len();
        let value_count = body.values.len();

        // (6) Entry has no predecessors.
        if (body.entry.0 as usize) < block_count
            && !body.blocks[body.entry.0 as usize].predecessors.is_empty()
        {
            errors.push(ValidationError {
                func: fid,
                kind: ValidationErrorKind::EntryHasPredecessors { block: body.entry },
            });
        }

        // (1, 2 forward, 3) Forward edges: predecessors and arity match.
        for (src_idx, src_blk) in body.blocks.iter().enumerate() {
            let src = BlockId(src_idx as u32);
            for (target, args) in src_blk.terminator.successors() {
                if (target.0 as usize) >= block_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingBlock { block: target },
                    });
                    continue;
                }
                let dst = &body.blocks[target.0 as usize];
                if args.len() != dst.params.len() {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::EdgeArityMismatch {
                            from: src,
                            to: target,
                            expected: dst.params.len(),
                            got: args.len(),
                        },
                    });
                }
                if !dst.predecessors.contains(&src) {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::PredecessorMismatch {
                            from: src,
                            to: target,
                            direction: "forward (target missing predecessor)",
                        },
                    });
                }
            }
        }

        // (2 reverse) Each listed predecessor's terminator targets us.
        for (dst_idx, dst_blk) in body.blocks.iter().enumerate() {
            let dst = BlockId(dst_idx as u32);
            for &pred in &dst_blk.predecessors {
                if (pred.0 as usize) >= block_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingBlock { block: pred },
                    });
                    continue;
                }
                let pred_blk = &body.blocks[pred.0 as usize];
                let actually = pred_blk
                    .terminator
                    .successors()
                    .iter()
                    .any(|(t, _)| *t == dst);
                if !actually {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::PredecessorMismatch {
                            from: pred,
                            to: dst,
                            direction: "reverse (terminator does not target)",
                        },
                    });
                }
            }
        }

        // (4, 7) Operand and InstId references in bounds.
        let values = &body.values;
        let capture_count = self.capture_count;
        let check_op = |op: &Operand, errors: &mut Vec<ValidationError>| match op {
            Operand::Value(v) => {
                if (v.0 as usize) >= value_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingValue { value: *v },
                    });
                } else if values[v.0 as usize].is_removed() {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::OperandReferencesRemovedValue { value: *v },
                    });
                }
            }
            Operand::Const(Constant::Func(f)) => {
                if (f.0 as usize) >= num_funcs {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingFunc { func: *f },
                    });
                }
            }
            Operand::Const(_) => {}
            Operand::Capture(idx) => {
                if *idx >= capture_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingCapture { index: *idx },
                    });
                }
            }
        };
        for inst in &body.instructions {
            walk_inst_operands(&inst.kind, |op| check_op(op, errors));
        }
        for blk in &body.blocks {
            walk_term_operands(&blk.terminator, |op| check_op(op, errors));
            for &iid in &blk.instructions {
                if (iid.0 as usize) >= inst_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingInst { inst: iid },
                    });
                }
            }
        }

        // (8) Switch case keys unique within a switch.
        for (b_idx, blk) in body.blocks.iter().enumerate() {
            if let Terminator::Switch { cases, .. } = &blk.terminator {
                let mut seen: Vec<&Constant> = Vec::new();
                for (i, c) in cases.iter().enumerate() {
                    if seen.iter().any(|s| const_eq(s, &c.key)) {
                        errors.push(ValidationError {
                            func: fid,
                            kind: ValidationErrorKind::SwitchDuplicateKey {
                                block: BlockId(b_idx as u32),
                                key_index: i,
                            },
                        });
                    }
                    seen.push(&c.key);
                }
            }
        }

        // ValueDef sanity.
        for (vi, def) in body.values.iter().enumerate() {
            match &def.kind {
                ValueDefKind::Inst(iid) => {
                    if (iid.0 as usize) >= inst_count {
                        errors.push(ValidationError {
                            func: fid,
                            kind: ValidationErrorKind::DanglingInst { inst: *iid },
                        });
                    }
                }
                ValueDefKind::BlockParam { block, index } => {
                    if (block.0 as usize) >= block_count {
                        errors.push(ValidationError {
                            func: fid,
                            kind: ValidationErrorKind::DanglingBlock { block: *block },
                        });
                    } else {
                        let blk = &body.blocks[block.0 as usize];
                        let v = Value(vi as u32);
                        if (*index as usize) >= blk.params.len() || blk.params[*index as usize] != v
                        {
                            errors.push(ValidationError {
                                func: fid,
                                kind: ValidationErrorKind::BlockParamMisregistered { value: v },
                            });
                        }
                    }
                }
                ValueDefKind::Arg(_) | ValueDefKind::Removed => {}
            }
        }
    }
}

fn const_eq(a: &Constant, b: &Constant) -> bool {
    use Constant::*;
    match (a, b) {
        // Bit-exact equality so that NaN keys are dedup'd against each other
        // and -0.0 / 0.0 stay distinct, matching strict-equality validator
        // intent.
        (Number(x), Number(y)) => x.to_bits() == y.to_bits(),
        (Int32(x), Int32(y)) => x == y,
        (Bool(x), Bool(y)) => x == y,
        (String(x), String(y)) => x == y,
        (Null, Null) => true,
        (Undefined, Undefined) => true,
        (Func(x), Func(y)) => x.0 == y.0,
        (GlobalThis, GlobalThis) => true,
        (This, This) => true,
        _ => false,
    }
}

pub fn walk_inst_operands(kind: &InstKind, mut f: impl FnMut(&Operand)) {
    use InstKind::*;
    match kind {
        AddGeneric { lhs, rhs }
        | SubGeneric { lhs, rhs }
        | MulGeneric { lhs, rhs }
        | DivGeneric { lhs, rhs }
        | ModGeneric { lhs, rhs }
        | PowGeneric { lhs, rhs }
        | BitAnd { lhs, rhs }
        | BitOr { lhs, rhs }
        | BitXor { lhs, rhs }
        | Shl { lhs, rhs }
        | Shr { lhs, rhs }
        | UShr { lhs, rhs }
        | AddNumber { lhs, rhs }
        | SubNumber { lhs, rhs }
        | MulNumber { lhs, rhs }
        | AddInt32 { lhs, rhs }
        | SubInt32 { lhs, rhs }
        | MulInt32 { lhs, rhs }
        | ConcatStr { lhs, rhs }
        | LogAnd { lhs, rhs }
        | LogOr { lhs, rhs }
        | Eq { lhs, rhs }
        | StrictEq { lhs, rhs }
        | Lt { lhs, rhs }
        | Le { lhs, rhs } => {
            f(lhs);
            f(rhs);
        }
        Not { operand } | BitNot { operand } => f(operand),
        LoadProp { obj, .. } => f(obj),
        StoreProp { obj, val, .. } => {
            f(obj);
            f(val);
        }
        LoadIndex { obj, index, .. } => {
            f(obj);
            f(index);
        }
        StoreIndex {
            obj, index, val, ..
        } => {
            f(obj);
            f(index);
            f(val);
        }
        AllocObject | AllocArray { .. } | AllocRegExp { .. } => {}
        AllocClosure { captures, .. } => {
            for c in captures {
                f(c);
            }
        }
        Call { callee, args } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallSpread { callee, args, .. } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallMethod { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        CallMethodSpread { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        Construct { ctor, args } => {
            f(ctor);
            for a in args {
                f(a);
            }
        }
        ArrayWithSpread { elements, .. } => {
            for e in elements {
                f(e);
            }
        }
        ObjectWithSpread { keys, vals } => {
            for k in keys {
                if let Some(SpreadObjectKey::Computed(op)) = k {
                    f(op);
                }
            }
            for v in vals {
                f(v);
            }
        }
        ToNumber { val }
        | ToString { val }
        | ToBool { val }
        | ToInt32 { val }
        | Typeof { val }
        | OpaqueEval { source: val } => f(val),
        Instanceof { obj, ctor } => {
            f(obj);
            f(ctor);
        }
        In { key, obj } => {
            f(key);
            f(obj);
        }
        CatchValue => {} // no operands
        ForInInit { obj } => f(obj),
        ForInDone { iter } | ForInNext { iter } => f(iter),
    }
}

pub fn walk_term_operands(t: &Terminator, mut f: impl FnMut(&Operand)) {
    match t {
        Terminator::Branch { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Terminator::CondBranch {
            cond,
            then_args,
            else_args,
            ..
        } => {
            f(cond);
            for a in then_args {
                f(a);
            }
            for a in else_args {
                f(a);
            }
        }
        Terminator::Switch {
            val,
            cases,
            default_args,
            ..
        } => {
            f(val);
            for c in cases {
                for a in &c.args {
                    f(a);
                }
            }
            for a in default_args {
                f(a);
            }
        }
        Terminator::Return { val: Some(v) } => f(v),
        Terminator::Return { val: None } => {}
        Terminator::Throw { val } => f(val),
        Terminator::Try { try_args, .. } => {
            for a in try_args {
                f(a);
            }
        }
        Terminator::Unreachable => {}
    }
}

// ============================================================================
// Pretty-printer
// ============================================================================

const COMMENT_COL: usize = 40;

pub fn pretty_module(m: &Module) -> String {
    let mut out = String::new();
    for i in 0..m.functions.len() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&pretty_function(m, FuncId(i as u32)));
    }
    out
}

pub fn pretty_function(m: &Module, id: FuncId) -> String {
    let func = &m.functions[id.0 as usize];
    let mut out = String::new();
    let body = match &func.body {
        FunctionBody::Lowered(b) => b,
        FunctionBody::PassThrough(pt) => {
            write!(out, "fn @{}(", func.name).unwrap();
            for (i, name) in pt.param_names.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(m.strings.resolve(name));
            }
            out.push_str(") passthrough");
            if func.capture_count > 0 {
                write!(out, " captures={}", func.capture_count).unwrap();
            }
            if pt.is_async {
                out.push_str(" async");
            }
            if pt.is_generator {
                out.push_str(" gen");
            }
            if pt.is_arrow {
                out.push_str(" arrow");
            }
            out.push_str(" {\n");
            for line in pt.source.lines() {
                out.push_str("  ");
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("}\n");
            return out;
        }
    };
    write!(out, "fn @{}(", func.name).unwrap();
    for (i, p) in body.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write!(out, "%{}", p.0).unwrap();
        if let Some(name) = body.values[p.0 as usize].source_name {
            write!(out, "[\"{}\"]", m.strings.resolve(&name)).unwrap();
        }
    }
    out.push_str(") {\n");
    for (i, blk) in body.blocks.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_block(&mut out, BlockId(i as u32), blk, body, m);
    }
    out.push_str("}\n");
    out
}

fn write_block(out: &mut String, id: BlockId, blk: &BasicBlock, func: &LoweredBody, m: &Module) {
    let mut header = String::new();
    write!(header, "  bb{}", id.0).unwrap();
    if !blk.params.is_empty() {
        header.push('(');
        for (i, p) in blk.params.iter().enumerate() {
            if i > 0 {
                header.push_str(", ");
            }
            write!(header, "%{}", p.0).unwrap();
        }
        header.push(')');
    }
    header.push(':');

    let mut preds = String::from("preds: ");
    if blk.predecessors.is_empty() {
        preds.push_str("<none>");
    } else {
        for (i, p) in blk.predecessors.iter().enumerate() {
            if i > 0 {
                preds.push_str(", ");
            }
            write!(preds, "bb{}", p.0).unwrap();
        }
    }
    push_with_comment(out, &header, Some(&preds));

    for &iid in &blk.instructions {
        write_inst(out, iid, func, m);
    }
    write_terminator(out, &blk.terminator, func, m);
}

fn write_inst(out: &mut String, iid: InstId, func: &LoweredBody, m: &Module) {
    let inst = &func.instructions[iid.0 as usize];
    let mut line = String::from("    ");
    if !inst.kind.is_void() {
        let v = find_value_for_inst(func, iid);
        write!(line, "%{}", v.0).unwrap();
        if let Some(name) = func.values[v.0 as usize].source_name {
            write!(line, "[\"{}\"]", m.strings.resolve(&name)).unwrap();
        }
        line.push_str(" = ");
    }
    write_inst_kind(&mut line, &inst.kind, m);
    let eff = inst.kind.effect();
    let comment = fmt_effect(eff);
    push_with_comment(out, &line, comment.as_deref());
}

/// Mutable counterpart to `walk_inst_operands`. Used by passes that
/// substitute operands in-place (e.g., constant folding, copy propagation).
pub fn walk_inst_operands_mut(kind: &mut InstKind, f: &mut impl FnMut(&mut Operand)) {
    use InstKind::*;
    match kind {
        AddGeneric { lhs, rhs }
        | SubGeneric { lhs, rhs }
        | MulGeneric { lhs, rhs }
        | DivGeneric { lhs, rhs }
        | ModGeneric { lhs, rhs }
        | PowGeneric { lhs, rhs }
        | BitAnd { lhs, rhs }
        | BitOr { lhs, rhs }
        | BitXor { lhs, rhs }
        | Shl { lhs, rhs }
        | Shr { lhs, rhs }
        | UShr { lhs, rhs }
        | AddNumber { lhs, rhs }
        | SubNumber { lhs, rhs }
        | MulNumber { lhs, rhs }
        | AddInt32 { lhs, rhs }
        | SubInt32 { lhs, rhs }
        | MulInt32 { lhs, rhs }
        | ConcatStr { lhs, rhs }
        | LogAnd { lhs, rhs }
        | LogOr { lhs, rhs }
        | Eq { lhs, rhs }
        | StrictEq { lhs, rhs }
        | Lt { lhs, rhs }
        | Le { lhs, rhs } => {
            f(lhs);
            f(rhs);
        }
        Not { operand } | BitNot { operand } => f(operand),
        LoadProp { obj, .. } => f(obj),
        StoreProp { obj, val, .. } => {
            f(obj);
            f(val);
        }
        LoadIndex { obj, index, .. } => {
            f(obj);
            f(index);
        }
        StoreIndex { obj, index, val, .. } => {
            f(obj);
            f(index);
            f(val);
        }
        AllocObject | AllocArray { .. } | AllocRegExp { .. } => {}
        AllocClosure { captures, .. } => {
            for c in captures {
                f(c);
            }
        }
        Call { callee, args } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallSpread { callee, args, .. } => {
            f(callee);
            for a in args {
                f(a);
            }
        }
        CallMethod { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        CallMethodSpread { obj, args, .. } => {
            f(obj);
            for a in args {
                f(a);
            }
        }
        ArrayWithSpread { elements, .. } => {
            for e in elements {
                f(e);
            }
        }
        ObjectWithSpread { keys, vals } => {
            for k in keys {
                if let Some(SpreadObjectKey::Computed(op)) = k {
                    f(op);
                }
            }
            for v in vals {
                f(v);
            }
        }
        Construct { ctor, args } => {
            f(ctor);
            for a in args {
                f(a);
            }
        }
        ToNumber { val }
        | ToString { val }
        | ToBool { val }
        | ToInt32 { val }
        | Typeof { val }
        | OpaqueEval { source: val } => f(val),
        Instanceof { obj, ctor } => {
            f(obj);
            f(ctor);
        }
        In { key, obj } => {
            f(key);
            f(obj);
        }
        CatchValue => {}
        ForInInit { obj } => f(obj),
        ForInDone { iter } | ForInNext { iter } => f(iter),
    }
}

/// Mutable counterpart to `walk_term_operands`.
pub fn walk_term_operands_mut(t: &mut Terminator, f: &mut impl FnMut(&mut Operand)) {
    match t {
        Terminator::Branch { args, .. } => {
            for a in args {
                f(a);
            }
        }
        Terminator::CondBranch {
            cond,
            then_args,
            else_args,
            ..
        } => {
            f(cond);
            for a in then_args {
                f(a);
            }
            for a in else_args {
                f(a);
            }
        }
        Terminator::Switch {
            val,
            cases,
            default_args,
            ..
        } => {
            f(val);
            for c in cases {
                for a in &mut c.args {
                    f(a);
                }
            }
            for a in default_args {
                f(a);
            }
        }
        Terminator::Return { val: Some(v) } => f(v),
        Terminator::Return { val: None } => {}
        Terminator::Throw { val } => f(val),
        Terminator::Try { try_args, .. } => {
            for a in try_args {
                f(a);
            }
        }
        Terminator::Unreachable => {}
    }
}

impl LoweredBody {
    /// Substitute every use of `old` (as `Operand::Value(old)`) with
    /// `new` across all instruction operands and terminator operands
    /// in this body. Used by SSA passes (constant folding, copy
    /// propagation, etc.) and by the converter's trivial-param
    /// elimination. Linear in the size of the body — sufficient for
    /// corpus-scale functions; if a pass profiles slow on this, build
    /// a use-list index in a follow-up ticket.
    ///
    /// Note: this updates only operands stored in the IR. Converter-
    /// internal state (e.g., `current_def`) and Function-level metadata
    /// (e.g., `captured_by_passthrough`) need separate handling at the
    /// caller.
    /// Rebuild every block's `predecessors` list from its terminator's
    /// successors. Passes that mutate terminators (branch folding,
    /// CFG simplification, etc.) MUST call this before validation runs
    /// or the predecessor invariant trips. Idempotent.
    pub fn rebuild_predecessors(&mut self) {
        let n = self.blocks.len();
        for blk in self.blocks.iter_mut() {
            blk.predecessors.clear();
        }
        for src_idx in 0..n {
            let src = BlockId(src_idx as u32);
            let succs: Vec<BlockId> = self.blocks[src_idx]
                .terminator
                .successors()
                .iter()
                .map(|(t, _)| *t)
                .collect();
            for t in succs {
                if (t.0 as usize) < n {
                    let preds = &mut self.blocks[t.0 as usize].predecessors;
                    if !preds.contains(&src) {
                        preds.push(src);
                    }
                }
            }
        }
    }

    pub fn replace_value_uses(&mut self, old: Value, new: Operand) {
        for inst in &mut self.instructions {
            walk_inst_operands_mut(&mut inst.kind, &mut |op| {
                if let Operand::Value(v) = op
                    && *v == old
                {
                    *op = new;
                }
            });
        }
        for blk in &mut self.blocks {
            walk_term_operands_mut(&mut blk.terminator, &mut |op| {
                if let Operand::Value(v) = op
                    && *v == old
                {
                    *op = new;
                }
            });
        }
    }
}

// ============================================================================
// Type lattice — used by RefineArithPass to specialise *Generic arithmetic
// ============================================================================

/// Coarse static type for an SSA `Value`. The lattice is flat over the JS
/// primitive types; `Top` means "could be anything" and `Bottom` means
/// "no information" (used as the join identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueType {
    Bottom,
    Int32,
    Number,
    String,
    Bool,
    Null,
    Undefined,
    Top,
}

impl ValueType {
    /// Lattice join: `Bottom ⊔ x = x`, `Top ⊔ x = Top`, equal types stay,
    /// any disagreement collapses to `Top`. Note Int32 and Number are
    /// kept distinct — refinement uses the finer distinction (Int32
    /// operands enable AddInt32, mixed Int32/Number does not).
    pub fn join(self, other: ValueType) -> ValueType {
        match (self, other) {
            (ValueType::Bottom, x) | (x, ValueType::Bottom) => x,
            (a, b) if a == b => a,
            _ => ValueType::Top,
        }
    }
}

fn const_type(c: &Constant) -> ValueType {
    match c {
        Constant::Number(_) => ValueType::Number,
        Constant::Int32(_) => ValueType::Int32,
        Constant::Bool(_) => ValueType::Bool,
        Constant::String(_) => ValueType::String,
        Constant::Null => ValueType::Null,
        Constant::Undefined => ValueType::Undefined,
        // GlobalThis/This/Func: opaque, no useful type.
        Constant::GlobalThis | Constant::This | Constant::Func(_) => ValueType::Top,
    }
}

/// Static type of an instruction's result, given a way to ask for the
/// type of any operand. Returns `Top` for instructions whose result
/// type we don't track (allocations, calls, loads, etc.).
fn inst_result_type(
    kind: &InstKind,
    mut op_type: impl FnMut(&Operand) -> ValueType,
) -> ValueType {
    use InstKind::*;
    match kind {
        // Always-Number arithmetic.
        AddNumber { .. }
        | SubNumber { .. }
        | MulNumber { .. }
        | DivGeneric { .. }
        | ModGeneric { .. }
        | PowGeneric { .. }
        | SubGeneric { .. }
        | MulGeneric { .. }
        | ToNumber { .. } => ValueType::Number,

        // Always-Int32: typed int arithmetic + bitwise (post-ToInt32).
        AddInt32 { .. }
        | SubInt32 { .. }
        | MulInt32 { .. }
        | BitAnd { .. }
        | BitOr { .. }
        | BitXor { .. }
        | Shl { .. }
        | Shr { .. }
        | BitNot { .. }
        | ToInt32 { .. } => ValueType::Int32,

        // UShr produces a Uint32 — out of i32 range when high bit set;
        // claiming Int32 would be unsound. Conservative Top.
        UShr { .. } => ValueType::Top,

        // String-producing.
        ConcatStr { .. } | ToString { .. } | Typeof { .. } | ForInNext { .. } => {
            ValueType::String
        }

        // Bool-producing.
        Not { .. }
        | Eq { .. }
        | StrictEq { .. }
        | Lt { .. }
        | Le { .. }
        | Instanceof { .. }
        | In { .. }
        | ToBool { .. }
        | ForInDone { .. } => ValueType::Bool,

        // AddGeneric: Number unless either side could be String, in which
        // case the result could be String. Be conservative — Top.
        AddGeneric { lhs, rhs } => {
            let lt = op_type(lhs);
            let rt = op_type(rhs);
            match (lt, rt) {
                (ValueType::Number, ValueType::Number)
                | (ValueType::Int32, ValueType::Int32)
                | (ValueType::Number, ValueType::Int32)
                | (ValueType::Int32, ValueType::Number) => ValueType::Number,
                (ValueType::String, ValueType::String) => ValueType::String,
                _ => ValueType::Top,
            }
        }

        // Logical short-circuit returns one of its operands; could be
        // either side's type. Take the join.
        LogAnd { lhs, rhs } | LogOr { lhs, rhs } => op_type(lhs).join(op_type(rhs)),

        // Everything else: opaque to this analysis.
        _ => ValueType::Top,
    }
}

/// Type of an SSA `Value` in the given body. Walks `ValueDef` once and
/// recurses through block-param edges and instruction operands. Cycles
/// (phi-of-phi loops) bail to `Top`.
pub fn value_type(body: &LoweredBody, v: Value) -> ValueType {
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    value_type_rec(body, v, &mut seen)
}

/// Type of an `Operand` — `Const` reads off the constant, `Value`
/// defers to `value_type`, and the opaque variants (Capture / Global /
/// This-binding) return `Top`.
pub fn operand_type(body: &LoweredBody, op: &Operand) -> ValueType {
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    operand_type_rec(body, op, &mut seen)
}

fn value_type_rec(
    body: &LoweredBody,
    v: Value,
    seen: &mut std::collections::HashSet<u32>,
) -> ValueType {
    if !seen.insert(v.0) {
        return ValueType::Top;
    }
    let def = &body.values[v.0 as usize];
    let result = match &def.kind {
        ValueDefKind::Removed | ValueDefKind::Arg(_) => ValueType::Top,
        ValueDefKind::Inst(iid) => {
            let kind = &body.instructions[iid.0 as usize].kind;
            inst_result_type(kind, |op| operand_type_rec(body, op, seen))
        }
        ValueDefKind::BlockParam { block, index } => {
            let blk = &body.blocks[block.0 as usize];
            let mut t = ValueType::Bottom;
            for &pred_id in &blk.predecessors {
                let pred = &body.blocks[pred_id.0 as usize];
                for (succ_block, args) in pred.terminator.successors() {
                    if succ_block == *block && (*index as usize) < args.len() {
                        t = t.join(operand_type_rec(body, &args[*index as usize], seen));
                    }
                }
            }
            // No predecessors contributed (entry block params, or the
            // catch-value edge): treat as opaque.
            if matches!(t, ValueType::Bottom) {
                ValueType::Top
            } else {
                t
            }
        }
    };
    seen.remove(&v.0);
    result
}

fn operand_type_rec(
    body: &LoweredBody,
    op: &Operand,
    seen: &mut std::collections::HashSet<u32>,
) -> ValueType {
    match op {
        Operand::Const(c) => const_type(c),
        Operand::Value(v) => value_type_rec(body, *v, seen),
        // Capture / non-Const opaque operands: unknown.
        _ => ValueType::Top,
    }
}

/// Try to constant-fold a typed-arithmetic / pure-comparison `InstKind`.
/// Returns `Some(c)` iff the instruction's result is determined by its
/// constant operands AND the fold is sound under JS semantics
/// (no coercion-throw risk, no Module-state required).
///
/// Handled InstKinds:
/// - `AddNumber` / `SubNumber` / `MulNumber` — IEEE-754 arithmetic on
///   Constant::Number operands. NaN / -0 / Infinity propagate per
///   IEEE-754; consumer is responsible for re-interpreting the result
///   under JS-equality semantics if relevant.
/// - `AddInt32` / `SubInt32` / `MulInt32` — i32 *wrapping* arithmetic
///   on Constant::Int32 operands. Matches the lowering's invariant
///   that AddInt32/SubInt32/MulInt32 are emitted only when both
///   operands fit i32; the result may wrap (matches `|0` semantics
///   in JS).
/// - `StrictEq` — between any pair of Const operands of the same
///   primitive type (Number/Int32/Bool/String/Null/Undefined).
///   Returns `Constant::Bool`. Number comparisons honor NaN ≠ NaN
///   and +0 === -0. Mixed-type comparisons are always `false`.
/// - `Lt` / `Le` — between Number or Int32 operands. Returns
///   Constant::Bool. NaN comparisons are always `false` per IEEE-754.
/// - `Not` — on Constant::Bool. Returns Constant::Bool.
///
/// Deliberately NOT handled (would require Module access or have
/// coercion-throw risk):
/// - `ConcatStr` (needs Module to intern result; future helper with
///   `try_const_fold_with_module(&Module, &InstKind)` signature).
/// - `Eq` (loose equality has coercion that may throw via valueOf).
/// - `AddGeneric` / `SubGeneric` / etc. (coercion may throw).
/// - Heap reads, calls, allocations.
pub fn try_const_fold(kind: &InstKind) -> Option<Constant> {
    use InstKind::*;
    fn n(op: &Operand) -> Option<f64> {
        match op {
            Operand::Const(Constant::Number(x)) => Some(*x),
            // Int32 ⊆ Number — accept either, since AddNumber et al.
            // operate on Number-typed values (Int32 is a subset).
            Operand::Const(Constant::Int32(x)) => Some(*x as f64),
            _ => None,
        }
    }
    fn i(op: &Operand) -> Option<i32> {
        match op {
            Operand::Const(Constant::Int32(x)) => Some(*x),
            _ => None,
        }
    }
    /// ECMA-262 §7.1.6 ToInt32 over a pure numeric constant. Bitwise
    /// ops apply ToInt32 to their operands implicitly, so accepting
    /// Number here matches their semantics. NaN/±Infinity → 0; finite
    /// values truncate then mod-2^32 then map to signed.
    fn to_i32(op: &Operand) -> Option<i32> {
        match op {
            Operand::Const(Constant::Int32(x)) => Some(*x),
            Operand::Const(Constant::Number(n)) => {
                if !n.is_finite() {
                    return Some(0);
                }
                let trunc = n.trunc();
                let modulo = trunc.rem_euclid(4_294_967_296.0);
                let u = modulo as u32;
                Some(u as i32)
            }
            _ => None,
        }
    }
    fn b(op: &Operand) -> Option<bool> {
        match op {
            Operand::Const(Constant::Bool(x)) => Some(*x),
            _ => None,
        }
    }
    match kind {
        AddNumber { lhs, rhs } => Some(Constant::Number(n(lhs)? + n(rhs)?)),
        SubNumber { lhs, rhs } => Some(Constant::Number(n(lhs)? - n(rhs)?)),
        MulNumber { lhs, rhs } => Some(Constant::Number(n(lhs)? * n(rhs)?)),
        AddInt32 { lhs, rhs } => Some(Constant::Int32(i(lhs)?.wrapping_add(i(rhs)?))),
        SubInt32 { lhs, rhs } => Some(Constant::Int32(i(lhs)?.wrapping_sub(i(rhs)?))),
        MulInt32 { lhs, rhs } => Some(Constant::Int32(i(lhs)?.wrapping_mul(i(rhs)?))),
        // Bitwise on Int32 constants — operands' ToInt32 already happened
        // (they're Constant::Int32), so these cannot throw. Shift counts
        // are masked by 31 per ECMA-262.
        BitAnd { lhs, rhs } => Some(Constant::Int32(to_i32(lhs)? & to_i32(rhs)?)),
        BitOr { lhs, rhs } => Some(Constant::Int32(to_i32(lhs)? | to_i32(rhs)?)),
        BitXor { lhs, rhs } => Some(Constant::Int32(to_i32(lhs)? ^ to_i32(rhs)?)),
        Shl { lhs, rhs } => {
            Some(Constant::Int32(to_i32(lhs)?.wrapping_shl((to_i32(rhs)? as u32) & 31)))
        }
        Shr { lhs, rhs } => {
            Some(Constant::Int32(to_i32(lhs)? >> ((to_i32(rhs)? as u32) & 31)))
        }
        BitNot { operand } => Some(Constant::Int32(!to_i32(operand)?)),
        Not { operand } => Some(Constant::Bool(!b(operand)?)),
        Lt { lhs, rhs } => {
            // Try Number-Number first, fall back to Int32-Int32. NaN
            // comparisons return false per IEEE-754 (which `<` already
            // does in Rust).
            if let (Some(l), Some(r)) = (n(lhs), n(rhs)) {
                return Some(Constant::Bool(l < r));
            }
            if let (Some(l), Some(r)) = (i(lhs), i(rhs)) {
                return Some(Constant::Bool(l < r));
            }
            None
        }
        Le { lhs, rhs } => {
            if let (Some(l), Some(r)) = (n(lhs), n(rhs)) {
                return Some(Constant::Bool(l <= r));
            }
            if let (Some(l), Some(r)) = (i(lhs), i(rhs)) {
                return Some(Constant::Bool(l <= r));
            }
            None
        }
        StrictEq { lhs, rhs } => {
            // Same-type comparison only. Mixed types are `false` per
            // ECMA-262 §7.2.16. NaN !== NaN; +0 === -0.
            match (lhs, rhs) {
                (Operand::Const(Constant::Number(a)), Operand::Const(Constant::Number(b))) => {
                    Some(Constant::Bool(a == b))
                }
                (Operand::Const(Constant::Int32(a)), Operand::Const(Constant::Int32(b))) => {
                    Some(Constant::Bool(a == b))
                }
                (Operand::Const(Constant::Bool(a)), Operand::Const(Constant::Bool(b))) => {
                    Some(Constant::Bool(a == b))
                }
                (Operand::Const(Constant::String(a)), Operand::Const(Constant::String(b))) => {
                    Some(Constant::Bool(a == b))
                }
                (Operand::Const(Constant::Null), Operand::Const(Constant::Null)) => {
                    Some(Constant::Bool(true))
                }
                (
                    Operand::Const(Constant::Undefined),
                    Operand::Const(Constant::Undefined),
                ) => Some(Constant::Bool(true)),
                // Cross-type comparisons of distinct primitive types: false.
                // We only return Some when BOTH operands are Const, otherwise
                // we don't know.
                (Operand::Const(a), Operand::Const(b)) => {
                    // Same-primitive-type cases handled above; reaching
                    // here means different primitive types (or one of the
                    // opaque sentinels — GlobalThis/This/Func — where we
                    // can't fold safely).
                    if matches!(
                        (a, b),
                        (Constant::GlobalThis, _)
                            | (_, Constant::GlobalThis)
                            | (Constant::This, _)
                            | (_, Constant::This)
                            | (Constant::Func(_), _)
                            | (_, Constant::Func(_))
                    ) {
                        return None;
                    }
                    Some(Constant::Bool(false))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// Module-aware constant folding. Falls through to `try_const_fold` for
/// everything that doesn't need string-interner access; the only extra
/// case it handles is `ConcatStr(Const::String, Const::String)`, which
/// needs `&mut Module` to intern the joined result.
pub fn try_const_fold_in_module(module: &mut Module, kind: &InstKind) -> Option<Constant> {
    if let InstKind::ConcatStr {
        lhs: Operand::Const(Constant::String(a)),
        rhs: Operand::Const(Constant::String(b)),
    } = kind
    {
        let la = module.strings.resolve(a).to_string();
        let lb = module.strings.resolve(b).to_string();
        let mut joined = String::with_capacity(la.len() + lb.len());
        joined.push_str(&la);
        joined.push_str(&lb);
        let id = module.strings.get_or_intern(&joined);
        return Some(Constant::String(id));
    }
    try_const_fold(kind)
}

/// Returns the `Value` that names the result of instruction `iid` in
/// the given lowered body. Panics if the instruction has no value-def
/// (i.e., a void instruction like `StoreProp`); callers must check
/// `InstKind::is_void()` before calling for instructions that may be
/// void.
pub fn find_value_for_inst(func: &LoweredBody, iid: InstId) -> Value {
    for (i, def) in func.values.iter().enumerate() {
        if let ValueDefKind::Inst(j) = &def.kind
            && *j == iid
        {
            return Value(i as u32);
        }
    }
    panic!("instruction {iid:?} has no value-def");
}

fn write_inst_kind(out: &mut String, kind: &InstKind, m: &Module) {
    use InstKind::*;
    match kind {
        AddGeneric { lhs, rhs } => write_binop(out, "add_generic", lhs, rhs, m),
        SubGeneric { lhs, rhs } => write_binop(out, "sub_generic", lhs, rhs, m),
        MulGeneric { lhs, rhs } => write_binop(out, "mul_generic", lhs, rhs, m),
        DivGeneric { lhs, rhs } => write_binop(out, "div_generic", lhs, rhs, m),
        ModGeneric { lhs, rhs } => write_binop(out, "mod_generic", lhs, rhs, m),
        PowGeneric { lhs, rhs } => write_binop(out, "pow_generic", lhs, rhs, m),
        BitAnd { lhs, rhs } => write_binop(out, "bit_and", lhs, rhs, m),
        BitOr { lhs, rhs } => write_binop(out, "bit_or", lhs, rhs, m),
        BitXor { lhs, rhs } => write_binop(out, "bit_xor", lhs, rhs, m),
        Shl { lhs, rhs } => write_binop(out, "shl", lhs, rhs, m),
        Shr { lhs, rhs } => write_binop(out, "shr", lhs, rhs, m),
        UShr { lhs, rhs } => write_binop(out, "ushr", lhs, rhs, m),
        AddNumber { lhs, rhs } => write_binop(out, "add_number", lhs, rhs, m),
        SubNumber { lhs, rhs } => write_binop(out, "sub_number", lhs, rhs, m),
        MulNumber { lhs, rhs } => write_binop(out, "mul_number", lhs, rhs, m),
        AddInt32 { lhs, rhs } => write_binop(out, "add_int32", lhs, rhs, m),
        SubInt32 { lhs, rhs } => write_binop(out, "sub_int32", lhs, rhs, m),
        MulInt32 { lhs, rhs } => write_binop(out, "mul_int32", lhs, rhs, m),
        ConcatStr { lhs, rhs } => write_binop(out, "concat_str", lhs, rhs, m),
        Not { operand } => {
            out.push_str("not ");
            write_operand(out, operand, m);
        }
        BitNot { operand } => unop(out, "bit_not", operand, m),
        LogAnd { lhs, rhs } => write_binop(out, "log_and", lhs, rhs, m),
        LogOr { lhs, rhs } => write_binop(out, "log_or", lhs, rhs, m),
        Eq { lhs, rhs } => write_binop(out, "eq", lhs, rhs, m),
        StrictEq { lhs, rhs } => write_binop(out, "strict_eq", lhs, rhs, m),
        Lt { lhs, rhs } => write_binop(out, "lt", lhs, rhs, m),
        Le { lhs, rhs } => write_binop(out, "le", lhs, rhs, m),
        LoadProp { obj, key, purity } => {
            write!(out, "load_prop{} ", purity.suffix()).unwrap();
            write_operand(out, obj, m);
            out.push_str(", ");
            write_string_literal(out, m.strings.resolve(key));
        }
        StoreProp {
            obj,
            key,
            val,
            purity,
        } => {
            write!(out, "store_prop{} ", purity.suffix()).unwrap();
            write_operand(out, obj, m);
            out.push_str(", ");
            write_string_literal(out, m.strings.resolve(key));
            out.push_str(", ");
            write_operand(out, val, m);
        }
        LoadIndex { obj, index, purity } => {
            write!(out, "load_index{} ", purity.suffix()).unwrap();
            write_operand(out, obj, m);
            out.push_str(", ");
            write_operand(out, index, m);
        }
        StoreIndex {
            obj,
            index,
            val,
            purity,
        } => {
            write!(out, "store_index{} ", purity.suffix()).unwrap();
            write_operand(out, obj, m);
            out.push_str(", ");
            write_operand(out, index, m);
            out.push_str(", ");
            write_operand(out, val, m);
        }
        AllocObject => out.push_str("alloc_object"),
        AllocArray { length } => match length {
            Some(n) => write!(out, "alloc_array len={}", n).unwrap(),
            None => out.push_str("alloc_array len=?"),
        },
        AllocRegExp { source } => {
            write!(out, "alloc_regexp ").unwrap();
            write_string_literal(out, m.strings.resolve(source));
        }
        AllocClosure { func, captures } => {
            write!(out, "alloc_closure @{}, [", func_label(m, *func)).unwrap();
            for (i, c) in captures.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                write_operand(out, c, m);
            }
            out.push(']');
        }
        Call { callee, args } => {
            out.push_str("call ");
            write_callee(out, callee, m);
            write_arg_list(out, args, m);
        }
        CallSpread { callee, args, spread_mask } => {
            out.push_str("call_spread ");
            write_callee(out, callee, m);
            out.push('(');
            for (i, (a, &is_spread)) in args.iter().zip(spread_mask.iter()).enumerate() {
                if i > 0 { out.push_str(", "); }
                if is_spread { out.push_str("..."); }
                write_operand(out, a, m);
            }
            out.push(')');
        }
        CallMethod { obj, method, args } => {
            out.push_str("call_method ");
            write_operand(out, obj, m);
            out.push_str(", ");
            write_string_literal(out, m.strings.resolve(method));
            out.push_str(", ");
            write_arg_list(out, args, m);
        }
        CallMethodSpread { obj, method, args, spread_mask } => {
            out.push_str("call_method_spread ");
            write_operand(out, obj, m);
            out.push_str(", ");
            write_string_literal(out, m.strings.resolve(method));
            out.push_str(", (");
            for (i, (a, &is_spread)) in args.iter().zip(spread_mask.iter()).enumerate() {
                if i > 0 { out.push_str(", "); }
                if is_spread { out.push_str("..."); }
                write_operand(out, a, m);
            }
            out.push(')');
        }
        Construct { ctor, args } => {
            out.push_str("construct ");
            write_callee(out, ctor, m);
            write_arg_list(out, args, m);
        }
        ArrayWithSpread { elements, spread_mask } => {
            out.push_str("array_with_spread [");
            for (i, (e, &is_spread)) in elements.iter().zip(spread_mask.iter()).enumerate() {
                if i > 0 { out.push_str(", "); }
                if is_spread { out.push_str("..."); }
                write_operand(out, e, m);
            }
            out.push(']');
        }
        ObjectWithSpread { keys, vals } => {
            out.push_str("object_with_spread {");
            for (i, (k, v)) in keys.iter().zip(vals.iter()).enumerate() {
                if i > 0 { out.push_str(", "); }
                match k {
                    None => { out.push_str("..."); write_operand(out, v, m); }
                    Some(SpreadObjectKey::Static(s)) => {
                        write_string_literal(out, m.strings.resolve(s));
                        out.push_str(": ");
                        write_operand(out, v, m);
                    }
                    Some(SpreadObjectKey::Computed(key_op)) => {
                        out.push('[');
                        write_operand(out, key_op, m);
                        out.push_str("]: ");
                        write_operand(out, v, m);
                    }
                }
            }
            out.push('}');
        }
        ToNumber { val } => unop(out, "to_number", val, m),
        ToString { val } => unop(out, "to_string", val, m),
        ToBool { val } => unop(out, "to_bool", val, m),
        ToInt32 { val } => unop(out, "to_int32", val, m),
        Typeof { val } => unop(out, "typeof", val, m),
        Instanceof { obj, ctor } => write_binop(out, "instanceof", obj, ctor, m),
        In { key, obj } => write_binop(out, "in", key, obj, m),
        OpaqueEval { source } => unop(out, "opaque_eval", source, m),
        CatchValue => out.push_str("catch_value"),
        ForInInit { obj } => unop(out, "for_in_init", obj, m),
        ForInDone { iter } => unop(out, "for_in_done", iter, m),
        ForInNext { iter } => unop(out, "for_in_next", iter, m),
    }
}

/// Render a function reference as `name` if known, else `f<id>`. Caller
/// supplies the leading `@`.
fn func_label(m: &Module, id: FuncId) -> String {
    if (id.0 as usize) < m.functions.len() {
        m.functions[id.0 as usize].name.clone()
    } else {
        format!("f{}", id.0)
    }
}

/// Direct-call rendering shortcut: a callee that is `Const(Func(id))` is
/// rendered as `@name` rather than the generic constant form so call sites
/// to known functions read naturally.
fn write_callee(out: &mut String, op: &Operand, m: &Module) {
    if let Operand::Const(Constant::Func(id)) = op {
        write!(out, "@{}", func_label(m, *id)).unwrap();
    } else {
        write_operand(out, op, m);
    }
}

fn write_binop(out: &mut String, name: &str, lhs: &Operand, rhs: &Operand, m: &Module) {
    out.push_str(name);
    out.push(' ');
    write_operand(out, lhs, m);
    out.push_str(", ");
    write_operand(out, rhs, m);
}

fn unop(out: &mut String, name: &str, val: &Operand, m: &Module) {
    out.push_str(name);
    out.push(' ');
    write_operand(out, val, m);
}

fn write_arg_list(out: &mut String, args: &[Operand], m: &Module) {
    out.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_operand(out, a, m);
    }
    out.push(')');
}

fn write_operand(out: &mut String, op: &Operand, m: &Module) {
    match op {
        Operand::Value(v) => write!(out, "%{}", v.0).unwrap(),
        Operand::Const(c) => write_constant(out, c, m),
        Operand::Capture(idx) => write!(out, "^{}", idx).unwrap(),
    }
}

fn write_constant(out: &mut String, c: &Constant, m: &Module) {
    match c {
        Constant::Number(n) => {
            if n.is_nan() {
                out.push_str("NaN");
            } else if n.is_infinite() {
                out.push_str(if *n > 0.0 { "Infinity" } else { "-Infinity" });
            } else if n.fract() == 0.0 && n.abs() < 1e15 {
                write!(out, "{}", *n as i64).unwrap();
            } else {
                write!(out, "{}", n).unwrap();
            }
        }
        Constant::Int32(n) => write!(out, "{}i32", n).unwrap(),
        Constant::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Constant::String(s) => write_string_literal(out, m.strings.resolve(s)),
        Constant::Null => out.push_str("null"),
        Constant::Undefined => out.push_str("undefined"),
        Constant::Func(id) => write!(out, "@{}", func_label(m, *id)).unwrap(),
        Constant::GlobalThis => out.push_str("globalThis"),
        Constant::This => out.push_str("this"),
    }
}

fn write_string_literal(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_terminator(out: &mut String, t: &Terminator, _func: &LoweredBody, m: &Module) {
    let mut line = String::from("    ");
    match t {
        Terminator::Branch { target, args } => {
            write!(line, "br ").unwrap();
            write_block_target(&mut line, *target, args, m);
        }
        Terminator::CondBranch {
            cond,
            then_b,
            then_args,
            else_b,
            else_args,
        } => {
            line.push_str("cbr ");
            write_operand(&mut line, cond, m);
            line.push_str(", ");
            write_block_target(&mut line, *then_b, then_args, m);
            line.push_str(", ");
            write_block_target(&mut line, *else_b, else_args, m);
        }
        Terminator::Switch {
            val,
            cases,
            default,
            default_args,
            join,
        } => {
            line.push_str("switch ");
            write_operand(&mut line, val, m);
            line.push_str(", [");
            push_with_comment(out, &line, None);
            for c in cases {
                let mut case_line = String::from("        ");
                write_constant(&mut case_line, &c.key, m);
                case_line.push_str(": ");
                write_block_target(&mut case_line, c.target, &c.args, m);
                case_line.push(',');
                push_with_comment(out, &case_line, None);
            }
            let mut tail = String::from("    ] default ");
            write_block_target(&mut tail, *default, default_args, m);
            write!(&mut tail, " join bb{}", join.0).unwrap();
            push_with_comment(out, &tail, None);
            return;
        }
        Terminator::Return { val: Some(v) } => {
            line.push_str("ret ");
            write_operand(&mut line, v, m);
        }
        Terminator::Return { val: None } => line.push_str("ret"),
        Terminator::Throw { val } => {
            line.push_str("throw ");
            write_operand(&mut line, val, m);
        }
        Terminator::Try {
            try_entry,
            try_args,
            catch_entry,
            finally_entry,
            join,
        } => {
            line.push_str("try ");
            write_block_target(&mut line, *try_entry, try_args, m);
            if let Some(c) = catch_entry {
                write!(&mut line, " catch bb{}", c.0).unwrap();
            }
            if let Some(f) = finally_entry {
                write!(&mut line, " finally bb{}", f.0).unwrap();
            }
            write!(&mut line, " join bb{}", join.0).unwrap();
        }
        Terminator::Unreachable => line.push_str("unreachable"),
    }
    push_with_comment(out, &line, None);
}

fn write_block_target(out: &mut String, target: BlockId, args: &[Operand], m: &Module) {
    write!(out, "bb{}", target.0).unwrap();
    if !args.is_empty() {
        out.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            write_operand(out, a, m);
        }
        out.push(')');
    }
}

fn fmt_effect(e: Effect) -> Option<String> {
    if e == Effect::PURE {
        return None;
    }
    if e == Effect::OPAQUE {
        return Some("opaque".to_string());
    }
    let mut parts: Vec<&str> = Vec::new();
    if e.contains(Effect::READS_HEAP) {
        parts.push("reads_heap");
    }
    if e.contains(Effect::WRITES_HEAP) {
        parts.push("writes_heap");
    }
    if e.contains(Effect::MAY_THROW) {
        parts.push("may_throw");
    }
    if e.contains(Effect::MAY_DIVERGE) {
        parts.push("may_diverge");
    }
    Some(parts.join(", "))
}

fn push_with_comment(out: &mut String, line: &str, comment: Option<&str>) {
    out.push_str(line);
    if let Some(c) = comment {
        let len = line.len();
        if len < COMMENT_COL {
            for _ in len..COMMENT_COL {
                out.push(' ');
            }
        } else {
            out.push(' ');
        }
        out.push_str("; ");
        out.push_str(c);
    }
    out.push('\n');
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sum_to_builds_validates_and_dumps() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "sum_to", 1);
        let arg0 = b.body.params[0];

        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let i = b.add_block_param(bb1);
        let acc = b.add_block_param(bb1);
        let bb2 = b.new_block();
        let bb3 = b.new_block();

        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![
                Operand::Const(Constant::Int32(0)),
                Operand::Const(Constant::Int32(0)),
            ],
        });

        b.switch_to_block(bb1);
        let cmp = b
            .add_inst(InstKind::Lt {
                lhs: Operand::Value(i),
                rhs: Operand::Value(arg0),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cmp),
            then_b: bb2,
            then_args: vec![],
            else_b: bb3,
            else_args: vec![],
        });

        b.switch_to_block(bb2);
        let i_next = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(i),
                rhs: Operand::Const(Constant::Int32(1)),
            })
            .unwrap();
        let acc_next = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Value(acc),
                rhs: Operand::Value(i),
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![Operand::Value(i_next), Operand::Value(acc_next)],
        });

        b.switch_to_block(bb3);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(acc)),
        });

        let _fid = b.finish();
        module.validate().expect("ssa module should validate");

        let dump = pretty_function(&module, FuncId(0));
        insta::assert_snapshot!(dump, @r#"
        fn @sum_to(%0) {
          bb0:                                  ; preds: <none>
            br bb1(0i32, 0i32)

          bb1(%1, %2):                          ; preds: bb0, bb2
            %3 = lt %1, %0                      ; may_throw
            cbr %3, bb2, bb3

          bb2:                                  ; preds: bb1
            %4 = add_generic %1, 1i32           ; may_throw
            %5 = add_generic %2, %1             ; may_throw
            br bb1(%4, %5)

          bb3:                                  ; preds: bb1
            ret %2
        }
        "#);
    }

    #[test]
    fn effect_dce_classification() {
        assert!(Effect::PURE.removable_when_unused());
        assert!(Effect::READS_HEAP.removable_when_unused());
        assert!(!Effect::MAY_THROW.removable_when_unused());
        assert!(!Effect::WRITES_HEAP.removable_when_unused());
        assert!(!Effect::MAY_DIVERGE.removable_when_unused());
        assert!(!Effect::OPAQUE.removable_when_unused());
    }

    #[test]
    fn validator_catches_arity_mismatch() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        let bb1 = b.new_block();
        let _p = b.add_block_param(bb1);
        b.switch_to_block(bb0);
        b.set_terminator(Terminator::Branch {
            target: bb1,
            args: vec![],
        });
        b.switch_to_block(bb1);
        b.set_terminator(Terminator::Return { val: None });
        b.finish();

        let errs = module.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| matches!(e.kind, ValidationErrorKind::EdgeArityMismatch { .. }))
        );
    }
}
