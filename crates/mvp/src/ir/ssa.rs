//! SSA-form intermediate representation.
//!
//! Block params (no phi nodes), inlined-constant operands, separate
//! Value/Instruction ID spaces joined by `ValueDef`, bitflags-based effect
//! lattice. The textual format produced by `pretty_function` is the binding
//! spec for snapshot tests; deviations are bugs.

// The binary doesn't yet consume this module; conversion and emit will land
// in a later milestone. Tests exercise the full surface, so we silence
// dead-code noise instead of papering over with shadow uses.
#![allow(dead_code)]

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
}

#[derive(Clone, Copy, Debug)]
pub enum Operand {
    Value(Value),
    Const(Constant),
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

    // Calls
    Call {
        callee: Operand,
        args: Vec<Operand>,
    },
    CallMethod {
        obj: Operand,
        method: StrId,
        args: Vec<Operand>,
    },
    Construct {
        ctor: Operand,
        args: Vec<Operand>,
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

    // Escape hatch — opaque, blocks all reordering and analysis.
    OpaqueEval {
        source: Operand,
    },
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
            | ToBool { .. }
            | Typeof { .. } => Effect::PURE,

            // May throw: generic arithmetic and coercions invoke valueOf/toString.
            AddGeneric { .. }
            | SubGeneric { .. }
            | MulGeneric { .. }
            | DivGeneric { .. }
            | ModGeneric { .. }
            | Eq { .. }
            | Lt { .. }
            | Le { .. }
            | ToNumber { .. }
            | ToString { .. }
            | ToInt32 { .. }
            | Instanceof { .. } => Effect::MAY_THROW,

            // Heap reads: dispatched by the per-site Purity tier.
            LoadProp { purity, .. } | LoadIndex { purity, .. } => purity.load_effect(),

            // Stores: dispatched by the per-site Purity tier. Defaults to
            // Opaque (setter can do anything) unless an analysis proves
            // narrower behavior.
            StoreProp { purity, .. } | StoreIndex { purity, .. } => purity.store_effect(),

            // Calls: opaque by default. A future analysis can refine specific calls.
            Call { .. } | CallMethod { .. } | Construct { .. } | OpaqueEval { .. } => {
                Effect::OPAQUE
            }
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
        default: BlockId,
        default_args: Vec<Operand>,
    },
    Return {
        val: Option<Operand>,
    },
    Throw {
        val: Operand,
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
    pub params: Vec<Value>,
    pub blocks: Vec<BasicBlock>,
    pub instructions: Vec<Instruction>,
    pub values: Vec<ValueDef>,
    pub entry: BlockId,
}

pub struct Module {
    pub functions: Vec<Function>,
    pub strings: Rodeo,
}

impl Module {
    pub fn new() -> Self {
        Self {
            functions: Vec::new(),
            strings: Rodeo::new(),
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
    pub func: Function,
    current: Option<BlockId>,
}

impl<'m> FunctionBuilder<'m> {
    pub fn new(module: &'m mut Module, name: impl Into<String>, num_args: u32) -> Self {
        let mut func = Function {
            name: name.into(),
            params: Vec::with_capacity(num_args as usize),
            blocks: Vec::new(),
            instructions: Vec::new(),
            values: Vec::new(),
            entry: BlockId(0),
        };
        for i in 0..num_args {
            let v = Value(func.values.len() as u32);
            func.values.push(ValueDef::arg(i));
            func.params.push(v);
        }
        Self {
            module,
            func,
            current: None,
        }
    }

    pub fn current_block(&self) -> Option<BlockId> {
        self.current
    }

    pub fn intern(&mut self, s: &str) -> StrId {
        self.module.strings.get_or_intern(s)
    }

    pub fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.func.blocks.len() as u32);
        self.func.blocks.push(BasicBlock {
            params: Vec::new(),
            instructions: Vec::new(),
            terminator: Terminator::Unreachable,
            predecessors: Vec::new(),
        });
        if self.func.blocks.len() == 1 {
            self.func.entry = id;
        }
        id
    }

    pub fn switch_to_block(&mut self, b: BlockId) {
        self.current = Some(b);
    }

    pub fn add_block_param(&mut self, block: BlockId) -> Value {
        let index = self.func.blocks[block.0 as usize].params.len() as u32;
        let v = Value(self.func.values.len() as u32);
        self.func.values.push(ValueDef::block_param(block, index));
        self.func.blocks[block.0 as usize].params.push(v);
        v
    }

    pub fn add_inst(&mut self, kind: InstKind) -> Option<Value> {
        let block = self
            .current
            .expect("no current block; call switch_to_block first");
        let inst_id = InstId(self.func.instructions.len() as u32);
        let void = kind.is_void();
        self.func.instructions.push(Instruction { kind });
        self.func.blocks[block.0 as usize]
            .instructions
            .push(inst_id);
        if void {
            None
        } else {
            let v = Value(self.func.values.len() as u32);
            self.func.values.push(ValueDef::inst(inst_id));
            Some(v)
        }
    }

    /// Tag the most recently allocated Value with a source-level name.
    /// Caller passes the Value returned by `add_inst` (or any other
    /// builder method that produces a Value); the renamer will prefer
    /// this name when emitting the binding for that value.
    pub fn set_source_name(&mut self, v: Value, name: StrId) {
        self.func.values[v.0 as usize].source_name = Some(name);
    }

    pub fn set_terminator(&mut self, term: Terminator) {
        let block = self.current.expect("no current block");
        self.func.blocks[block.0 as usize].terminator = term;
    }

    pub fn finish(mut self) -> FuncId {
        for blk in self.func.blocks.iter_mut() {
            blk.predecessors.clear();
        }
        let n = self.func.blocks.len();
        for src_idx in 0..n {
            let src = BlockId(src_idx as u32);
            let succs: Vec<BlockId> = self.func.blocks[src_idx]
                .terminator
                .successors()
                .iter()
                .map(|(t, _)| *t)
                .collect();
            for t in succs {
                if (t.0 as usize) < n {
                    let preds = &mut self.func.blocks[t.0 as usize].predecessors;
                    if !preds.contains(&src) {
                        preds.push(src);
                    }
                }
            }
        }
        let fid = FuncId(self.module.functions.len() as u32);
        self.module.functions.push(self.func);
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
        let block_count = self.blocks.len();
        let inst_count = self.instructions.len();
        let value_count = self.values.len();

        // (6) Entry has no predecessors.
        if (self.entry.0 as usize) < block_count
            && !self.blocks[self.entry.0 as usize].predecessors.is_empty()
        {
            errors.push(ValidationError {
                func: fid,
                kind: ValidationErrorKind::EntryHasPredecessors { block: self.entry },
            });
        }

        // (1, 2 forward, 3) Forward edges: predecessors and arity match.
        for (src_idx, src_blk) in self.blocks.iter().enumerate() {
            let src = BlockId(src_idx as u32);
            for (target, args) in src_blk.terminator.successors() {
                if (target.0 as usize) >= block_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingBlock { block: target },
                    });
                    continue;
                }
                let dst = &self.blocks[target.0 as usize];
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
        for (dst_idx, dst_blk) in self.blocks.iter().enumerate() {
            let dst = BlockId(dst_idx as u32);
            for &pred in &dst_blk.predecessors {
                if (pred.0 as usize) >= block_count {
                    errors.push(ValidationError {
                        func: fid,
                        kind: ValidationErrorKind::DanglingBlock { block: pred },
                    });
                    continue;
                }
                let pred_blk = &self.blocks[pred.0 as usize];
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
        let values = &self.values;
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
        };
        for inst in &self.instructions {
            walk_inst_operands(&inst.kind, |op| check_op(op, errors));
        }
        for blk in &self.blocks {
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
        for (b_idx, blk) in self.blocks.iter().enumerate() {
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
        for (vi, def) in self.values.iter().enumerate() {
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
                        let blk = &self.blocks[block.0 as usize];
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
        _ => false,
    }
}

fn walk_inst_operands(kind: &InstKind, mut f: impl FnMut(&Operand)) {
    use InstKind::*;
    match kind {
        AddGeneric { lhs, rhs }
        | SubGeneric { lhs, rhs }
        | MulGeneric { lhs, rhs }
        | DivGeneric { lhs, rhs }
        | ModGeneric { lhs, rhs }
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
        Not { operand } => f(operand),
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
        AllocObject | AllocArray { .. } => {}
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
        CallMethod { obj, args, .. } => {
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
    }
}

fn walk_term_operands(t: &Terminator, mut f: impl FnMut(&Operand)) {
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
    write!(out, "fn @{}(", func.name).unwrap();
    for (i, p) in func.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write!(out, "%{}", p.0).unwrap();
        if let Some(name) = func.values[p.0 as usize].source_name {
            write!(out, "[\"{}\"]", m.strings.resolve(&name)).unwrap();
        }
    }
    out.push_str(") {\n");
    for (i, blk) in func.blocks.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        write_block(&mut out, BlockId(i as u32), blk, func, m);
    }
    out.push_str("}\n");
    out
}

fn write_block(out: &mut String, id: BlockId, blk: &BasicBlock, func: &Function, m: &Module) {
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

fn write_inst(out: &mut String, iid: InstId, func: &Function, m: &Module) {
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

fn find_value_for_inst(func: &Function, iid: InstId) -> Value {
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
        CallMethod { obj, method, args } => {
            out.push_str("call_method ");
            write_operand(out, obj, m);
            out.push_str(", ");
            write_string_literal(out, m.strings.resolve(method));
            out.push_str(", ");
            write_arg_list(out, args, m);
        }
        Construct { ctor, args } => {
            out.push_str("construct ");
            write_callee(out, ctor, m);
            write_arg_list(out, args, m);
        }
        ToNumber { val } => unop(out, "to_number", val, m),
        ToString { val } => unop(out, "to_string", val, m),
        ToBool { val } => unop(out, "to_bool", val, m),
        ToInt32 { val } => unop(out, "to_int32", val, m),
        Typeof { val } => unop(out, "typeof", val, m),
        Instanceof { obj, ctor } => write_binop(out, "instanceof", obj, ctor, m),
        OpaqueEval { source } => unop(out, "opaque_eval", source, m),
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

fn write_terminator(out: &mut String, t: &Terminator, _func: &Function, m: &Module) {
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
        let arg0 = b.func.params[0];

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
