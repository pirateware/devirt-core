//! SSA-IR → tree-IR conversion (out-of-SSA).
//!
//! Two conceptual phases performed simultaneously during the structure-trace
//! walk:
//!
//! 1. **Eliminate block params.** Block params have no syntactic equivalent in
//!    JS, so each is replaced by a fresh tree-IR variable (`_t0`, `_t1`, …).
//!    For every `Branch { target, args }` whose target carries params, we
//!    emit `name(target.params[i]) = expr(args[i])` immediately before
//!    transferring control to `target` in the tree-IR. Conceptually this is
//!    the "always-insert-a-copy-block" simplification of standard out-of-SSA
//!    — it avoids the swap problem because each predecessor's copies live in
//!    a syntactically separate location.
//!
//! 2. **Reconstruct structured control flow.** We walk the CFG and recognize
//!    the specific shapes that the upstream SSA converter produces:
//!     - linear chains (single-successor `Branch`),
//!     - if/else diamonds (cbr to two blocks meeting at a common
//!       post-dominator),
//!     - while loops (cbr where one branch reaches back to the cbr's block,
//!       the other exits).
//!
//! Anything else panics with `unsupported CFG shape`. This is deliberate: the
//! converter only emits these shapes today; future passes that produce
//! arbitrary CFGs will need a real relooper, which is a later milestone.
//!
//! What survives the round-trip:
//!   - **Behavior** — the only invariant that matters; verified by the
//!     roundtrip tests at the bottom of this file.
//!   - **Original variable names** — NOT preserved. SSA conversion already
//!     erased them; this pass allocates fresh `_a0`/`_t0` names.
//!   - **`const` vs `let`** — NOT preserved; everything emerges as `let`.
//!   - **Block-shape exactness** — mostly preserved by the structure-trace
//!     heuristics; small differences (extra temporaries, redundant blocks)
//!     are fine.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};

use crate::ir::ssa::{
    self, BlockId, Constant, FuncId, Function, InstId, InstKind, Operand, Terminator, Value,
    ValueDefKind,
};
use crate::ir::tree::{self, BinOp, Expr, IrFunction, Item, Stmt, UnOp};

// ============================================================================
// Public entry points
// ============================================================================

pub fn lower_module(sm: &ssa::Module) -> tree::Module {
    lower_module_with_config(sm, &EmitConfig::default())
}

pub fn lower_module_with_config(sm: &ssa::Module, cfg: &EmitConfig) -> tree::Module {
    let mut items = Vec::with_capacity(sm.functions.len());
    for (i, func) in sm.functions.iter().enumerate() {
        items.push(Item::Function(lower_function(
            sm,
            FuncId(i as u32),
            func,
            cfg,
        )));
    }
    tree::Module { items }
}

/// Configuration for the out-of-SSA renamer.
///
/// `immovable_names` pins specific Values to specific names — typically
/// because some downstream emit (a PassThrough closure body, a debug
/// pin) lexically references the name and cannot tolerate a
/// suffix-renamed substitute. The pinned name is reserved: other Values
/// that prefer it via `source_name` get suffix-disambiguated, and the
/// suffix-collision path skips the pinned name when generating
/// candidates. Pinning two distinct Values to the same name is a caller
/// bug and panics.
///
/// Implementation note: the ticket spec described this as
/// `HashSet<String>`, but a per-Value mapping is required to express
/// "this specific Value owns this name" — without it, the conflict
/// (test 8) and route-around (test 7) scenarios are indistinguishable
/// from the renamer's perspective.
#[derive(Default, Debug, Clone)]
pub struct EmitConfig {
    pub immovable_names: HashMap<u32, String>,
}

fn lower_function(sm: &ssa::Module, _id: FuncId, func: &Function, cfg: &EmitConfig) -> IrFunction {
    let mut ctx = LowerCtx::new(sm, func, cfg);
    let param_names: Vec<String> = func
        .params
        .iter()
        .map(|v| ctx.name_for(*v).to_string())
        .collect();

    let mut body = Vec::new();
    // Hoist all temporary declarations to the top of the function. Hoisting
    // is the simplest way to ensure that names assigned inside if/while
    // bodies are visible in subsequent code: a `let` declared inside a JS
    // block scope is invisible after the block. Cosmetic ugliness; later
    // passes can sink lets back to first-use.
    for name in ctx.hoisted_names_in_decl_order() {
        body.push(Stmt::Let { name, init: None });
    }

    ctx.walk(func.entry, None, &mut body);

    IrFunction {
        name: func.name.clone(),
        params: param_names,
        body,
    }
}

// ============================================================================
// LowerCtx — per-function state for the walk
// ============================================================================

struct LowerCtx<'a> {
    sm: &'a ssa::Module,
    func: &'a Function,

    /// Per-Value name. Indexed by `Value.0`. `Removed` slots are skipped.
    value_names: HashMap<u32, String>,
    /// Names to hoist at function-body top, in allocation order.
    hoisted: Vec<String>,
    /// Cached forward-reachability per block (for join detection).
    reachable_cache: HashMap<BlockId, HashSet<BlockId>>,
    /// Cached "is loop header" decisions.
    loop_info_cache: HashMap<BlockId, Option<LoopInfo>>,
    /// Cached domination relation.
    dom_cache: HashMap<(BlockId, BlockId), bool>,
}

#[derive(Clone, Copy, Debug)]
struct LoopInfo {
    body_b: BlockId,
    exit_b: BlockId,
}

impl<'a> LowerCtx<'a> {
    fn new(sm: &'a ssa::Module, func: &'a Function, cfg: &EmitConfig) -> Self {
        let (value_names, hoisted) = assign_names(sm, func, cfg);
        Self {
            sm,
            func,
            value_names,
            hoisted,
            reachable_cache: HashMap::new(),
            loop_info_cache: HashMap::new(),
            dom_cache: HashMap::new(),
        }
    }

    fn hoisted_names_in_decl_order(&self) -> Vec<String> {
        self.hoisted.clone()
    }

    fn name_for(&self, v: Value) -> &str {
        self.value_names
            .get(&v.0)
            .unwrap_or_else(|| panic!("no name allocated for value %{}", v.0))
    }

    fn operand_to_expr(&self, op: &Operand) -> Expr {
        match op {
            Operand::Value(v) => Expr::Ident(self.name_for(*v).to_string()),
            Operand::Const(c) => self.constant_to_expr(c),
        }
    }

    fn constant_to_expr(&self, c: &Constant) -> Expr {
        match c {
            Constant::Number(n) => Expr::Number(*n),
            Constant::Int32(n) => Expr::Number(*n as f64),
            Constant::Bool(b) => Expr::Bool(*b),
            Constant::String(s) => Expr::String(self.sm.strings.resolve(s).to_string()),
            Constant::Null => Expr::Null,
            Constant::Undefined => Expr::Undefined,
            // Direct references to top-level functions become identifier
            // reads of the function name. Function declarations are
            // top-level, so the binding is in scope at every call site.
            Constant::Func(id) => Expr::Ident(self.sm.functions[id.0 as usize].name.clone()),
            // `GlobalThis` used as a value (not as the obj of a LoadProp /
            // StoreProp — those short-circuit at the inst level) emits as
            // the literal `globalThis` identifier. Note this is
            // intentionally a lossy normalization: `window`, `self`, and
            // `global` collapse to the same Const(GlobalThis) and therefore
            // all re-emit as `globalThis`.
            Constant::GlobalThis => Expr::Ident("globalThis".to_string()),
        }
    }
}

// ============================================================================
// Structure-trace walk
// ============================================================================

impl LowerCtx<'_> {
    /// Walk the CFG starting at `start`, appending statements to `out`. Stops
    /// when reaching `stop` (after emitting any required edge copies into
    /// `stop`) or when the path terminates internally.
    fn walk(&mut self, start: BlockId, stop: Option<BlockId>, out: &mut Vec<Stmt>) {
        let mut cur = start;
        loop {
            if Some(cur) == stop {
                return;
            }

            // Loop header check happens BEFORE emitting instructions: the
            // header's instructions are part of the loop's per-iteration
            // condition evaluation, not pre-loop work.
            if let Some(info) = self.loop_info(cur) {
                self.emit_loop(cur, info, out);
                cur = info.exit_b;
                continue;
            }

            self.emit_block_insts(cur, out);

            let term = self.func.blocks[cur.0 as usize].terminator.clone();
            match term {
                Terminator::Return { val } => {
                    let e = val.as_ref().map(|v| self.operand_to_expr(v));
                    out.push(Stmt::Return(e));
                    return;
                }
                Terminator::Throw { val } => {
                    // Tree-IR has no throw yet. Lower to a placeholder call
                    // so the IR is well-formed; future tree-IR additions will
                    // upgrade this to a real Stmt::Throw.
                    let arg = self.operand_to_expr(&val);
                    out.push(Stmt::ExprStmt(Expr::Call {
                        callee: Box::new(Expr::Ident("__throw__".to_string())),
                        args: vec![arg],
                    }));
                    return;
                }
                Terminator::Unreachable => {
                    out.push(Stmt::ExprStmt(Expr::Call {
                        callee: Box::new(Expr::Ident("__unreachable__".to_string())),
                        args: vec![],
                    }));
                    return;
                }
                Terminator::Branch { target, args } => {
                    self.emit_edge_copies(target, &args, out);
                    if Some(target) == stop {
                        return;
                    }
                    cur = target;
                }
                Terminator::CondBranch {
                    cond,
                    then_b,
                    then_args,
                    else_b,
                    else_args,
                } => {
                    let cond_expr = self.operand_to_expr(&cond);
                    let inner_stop = self.find_diamond_join(then_b, else_b, stop);
                    let pass_through_stop = inner_stop.or(stop);

                    let mut then_stmts = Vec::new();
                    self.emit_edge_copies(then_b, &then_args, &mut then_stmts);
                    self.walk(then_b, pass_through_stop, &mut then_stmts);

                    let mut else_stmts = Vec::new();
                    self.emit_edge_copies(else_b, &else_args, &mut else_stmts);
                    self.walk(else_b, pass_through_stop, &mut else_stmts);

                    out.push(Stmt::If {
                        cond: cond_expr,
                        then_body: then_stmts,
                        else_body: Some(else_stmts),
                    });

                    match inner_stop {
                        Some(j) => {
                            cur = j;
                        }
                        None => return,
                    }
                }
                Terminator::Switch { .. } => {
                    panic!(
                        "Switch terminator at bb{} — out-of-SSA does not yet lower switches; \
                         this is a TODO once tree-IR grows a Switch statement.",
                        cur.0
                    );
                }
            }
        }
    }

    /// Emit a basic block's straight-line instructions (no terminator). Each
    /// instruction's result Value is bound to its hoisted name via plain
    /// assignment — the `let` was emitted at function-body top.
    fn emit_block_insts(&self, cur: BlockId, out: &mut Vec<Stmt>) {
        let blk = &self.func.blocks[cur.0 as usize];
        for &iid in &blk.instructions {
            self.emit_inst(iid, out);
        }
    }

    fn emit_inst(&self, iid: InstId, out: &mut Vec<Stmt>) {
        let kind = &self.func.instructions[iid.0 as usize].kind;
        if kind.is_void() {
            // Stores produce no value. Emit as a side-effecting statement.
            match kind {
                InstKind::StoreProp { obj, key, val, .. } => {
                    let prop = self.sm.strings.resolve(key).to_string();
                    // Stores rooted at `Const(GlobalThis)` re-emit as bare
                    // `name = value`, undoing the structural indirection
                    // introduced by the unbound-write lowering.
                    let target = match obj {
                        Operand::Const(Constant::GlobalThis) => Expr::Ident(prop),
                        _ => Expr::Member {
                            obj: Box::new(self.operand_to_expr(obj)),
                            prop,
                        },
                    };
                    out.push(Stmt::Assign {
                        target,
                        value: self.operand_to_expr(val),
                    });
                }
                InstKind::StoreIndex {
                    obj, index, val, ..
                } => {
                    out.push(Stmt::Assign {
                        target: Expr::Index {
                            obj: Box::new(self.operand_to_expr(obj)),
                            index: Box::new(self.operand_to_expr(index)),
                        },
                        value: self.operand_to_expr(val),
                    });
                }
                _ => unreachable!("is_void() classified non-void instruction"),
            }
            return;
        }

        let name = {
            let v = find_value_for_inst(self.func, iid);
            self.name_for(v).to_string()
        };
        let value_expr = self.inst_kind_to_expr(kind);
        out.push(Stmt::Assign {
            target: Expr::Ident(name),
            value: value_expr,
        });
    }

    /// Lower an InstKind into a tree-IR expression. Only the kinds the
    /// current SSA converter actually emits are handled fully; the rest
    /// either fall through to a reasonable approximation or panic with a
    /// TODO message.
    fn inst_kind_to_expr(&self, kind: &InstKind) -> Expr {
        use InstKind::*;
        match kind {
            // Generic arithmetic — frontend's primary emit shape.
            AddGeneric { lhs, rhs } | AddNumber { lhs, rhs } | AddInt32 { lhs, rhs } => {
                self.binop(BinOp::Add, lhs, rhs)
            }
            SubGeneric { lhs, rhs } | SubNumber { lhs, rhs } | SubInt32 { lhs, rhs } => {
                self.binop(BinOp::Sub, lhs, rhs)
            }
            MulGeneric { lhs, rhs } | MulNumber { lhs, rhs } | MulInt32 { lhs, rhs } => {
                self.binop(BinOp::Mul, lhs, rhs)
            }
            DivGeneric { lhs, rhs } => self.binop(BinOp::Div, lhs, rhs),
            ModGeneric { lhs, rhs } => self.binop(BinOp::Mod, lhs, rhs),
            ConcatStr { lhs, rhs } => self.binop(BinOp::Add, lhs, rhs),
            Not { operand } => Expr::Unary {
                op: UnOp::Not,
                operand: Box::new(self.operand_to_expr(operand)),
            },
            // Value-producing logical ops shouldn't normally appear (the
            // converter routes && / || through a CFG diamond). If one does
            // sneak in, the both-sides-eval semantics differ slightly from
            // JS short-circuit, but for inputs without side effects it's
            // equivalent.
            LogAnd { lhs, rhs } => self.binop(BinOp::LogAnd, lhs, rhs),
            LogOr { lhs, rhs } => self.binop(BinOp::LogOr, lhs, rhs),
            Eq { lhs, rhs } => self.binop(BinOp::Eq, lhs, rhs),
            StrictEq { lhs, rhs } => self.binop(BinOp::StrictEq, lhs, rhs),
            Lt { lhs, rhs } => self.binop(BinOp::Lt, lhs, rhs),
            Le { lhs, rhs } => self.binop(BinOp::Le, lhs, rhs),
            LoadProp { obj, key, .. } => {
                let prop = self.sm.strings.resolve(key).to_string();
                // Reads rooted at `Const(GlobalThis)` re-emit as a bare
                // identifier (`console`, not `globalThis.console`),
                // undoing the structural indirection introduced by the
                // unbound-read lowering.
                match obj {
                    Operand::Const(Constant::GlobalThis) => Expr::Ident(prop),
                    _ => Expr::Member {
                        obj: Box::new(self.operand_to_expr(obj)),
                        prop,
                    },
                }
            }
            LoadIndex { obj, index, .. } => Expr::Index {
                obj: Box::new(self.operand_to_expr(obj)),
                index: Box::new(self.operand_to_expr(index)),
            },
            Call { callee, args } => Expr::Call {
                callee: Box::new(self.operand_to_expr(callee)),
                args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
            },
            CallMethod { obj, method, args } => {
                let callee = Expr::Member {
                    obj: Box::new(self.operand_to_expr(obj)),
                    prop: self.sm.strings.resolve(method).to_string(),
                };
                Expr::Call {
                    callee: Box::new(callee),
                    args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
                }
            }
            ToNumber { val } => Expr::Unary {
                op: UnOp::Pos,
                operand: Box::new(self.operand_to_expr(val)),
            },
            Typeof { val } => Expr::Unary {
                op: UnOp::Typeof,
                operand: Box::new(self.operand_to_expr(val)),
            },
            // The rest aren't emitted by the current converter. Keep loud
            // panics so a future addition surfaces here rather than silently
            // generating wrong code.
            StoreProp { .. } | StoreIndex { .. } => {
                unreachable!("void inst handled in emit_inst")
            }
            AllocObject => Expr::Call {
                callee: Box::new(Expr::Ident("Object".to_string())),
                args: vec![],
            },
            AllocArray { .. } => Expr::Call {
                callee: Box::new(Expr::Ident("Array".to_string())),
                args: vec![],
            },
            AllocClosure { .. } => {
                panic!("AllocClosure not lowered: closures aren't yet emitted by the SSA converter")
            }
            Construct { ctor, args } => Expr::Call {
                callee: Box::new(self.operand_to_expr(ctor)),
                args: args.iter().map(|a| self.operand_to_expr(a)).collect(),
            },
            ToString { val } => Expr::Call {
                callee: Box::new(Expr::Ident("String".to_string())),
                args: vec![self.operand_to_expr(val)],
            },
            ToBool { val } => Expr::Call {
                callee: Box::new(Expr::Ident("Boolean".to_string())),
                args: vec![self.operand_to_expr(val)],
            },
            ToInt32 { val } => Expr::Binary {
                op: BinOp::BitOr,
                lhs: Box::new(self.operand_to_expr(val)),
                rhs: Box::new(Expr::Number(0.0)),
            },
            Instanceof { obj, ctor } => Expr::Call {
                callee: Box::new(Expr::Ident("__instanceof__".to_string())),
                args: vec![self.operand_to_expr(obj), self.operand_to_expr(ctor)],
            },
            OpaqueEval { source } => Expr::Call {
                callee: Box::new(Expr::Ident("eval".to_string())),
                args: vec![self.operand_to_expr(source)],
            },
        }
    }

    fn binop(&self, op: BinOp, lhs: &Operand, rhs: &Operand) -> Expr {
        Expr::Binary {
            op,
            lhs: Box::new(self.operand_to_expr(lhs)),
            rhs: Box::new(self.operand_to_expr(rhs)),
        }
    }

    /// Emit `name(target.params[i]) = expr(args[i])` assignments for a
    /// `Branch { target, args }` edge. The names are hoisted at function
    /// top, so `Stmt::Assign` is always sufficient — no `let` ever needed
    /// here.
    fn emit_edge_copies(&self, target: BlockId, args: &[Operand], out: &mut Vec<Stmt>) {
        let target_blk = &self.func.blocks[target.0 as usize];
        debug_assert_eq!(
            target_blk.params.len(),
            args.len(),
            "edge arity mismatch into bb{}",
            target.0
        );
        for (param, arg) in target_blk.params.iter().zip(args.iter()) {
            let lhs_name = self.name_for(*param).to_string();
            let rhs_expr = self.operand_to_expr(arg);
            // Suppress trivial self-copies (`x = x`) — they're noise after
            // trivial-param removal would have caught them, but the SSA
            // form may legitimately keep a param whose name now matches one
            // of its own incomings.
            if let Expr::Ident(rhs_name) = &rhs_expr
                && rhs_name == &lhs_name
            {
                continue;
            }
            out.push(Stmt::Assign {
                target: Expr::Ident(lhs_name),
                value: rhs_expr,
            });
        }
    }

    // -----------------------------------------------------------------
    // Loop emission
    // -----------------------------------------------------------------

    /// Emit a `while` loop for `header` (already known to be a loop header).
    /// The body walks until reaching `header` again (the back-edge), at
    /// which point the back-edge copies are emitted at the body's tail.
    fn emit_loop(&mut self, header: BlockId, info: LoopInfo, out: &mut Vec<Stmt>) {
        let header_blk = &self.func.blocks[header.0 as usize];
        let cond_op = match &header_blk.terminator {
            Terminator::CondBranch { cond, .. } => *cond,
            _ => unreachable!("loop_info already validated CondBranch terminator"),
        };

        // Try to inline the cond. If the header has exactly one instruction
        // whose result IS the cbr's cond, we can inline it into the
        // `while (...)` expression — the typical lower_while shape produces
        // this. Otherwise we fall back to evaluating the header's
        // instructions both before the loop AND at the body's tail (and
        // reading the cond Value name in the while header).
        let (cond_expr, header_insts_handled_inline) = self.try_inline_cond(header, &cond_op);

        let mut body_stmts = Vec::new();

        // Walk through the cond-edge's copies (cbr edges have no args in
        // our converter, but we go through emit_edge_copies for symmetry
        // and future-proofing).
        let cbr_then_args: Vec<Operand>;
        let cbr_else_args: Vec<Operand>;
        match &header_blk.terminator {
            Terminator::CondBranch {
                then_args,
                else_args,
                ..
            } => {
                cbr_then_args = then_args.clone();
                cbr_else_args = else_args.clone();
            }
            _ => unreachable!(),
        };

        // Determine which cbr branch is the body and which is the exit (the
        // ordering depends on how lower_while's cbr ended up).
        let (body_args, exit_args) = match &header_blk.terminator {
            Terminator::CondBranch { then_b, else_b, .. } => {
                if *then_b == info.body_b {
                    (cbr_then_args.clone(), cbr_else_args.clone())
                } else {
                    debug_assert_eq!(*else_b, info.body_b);
                    (cbr_else_args.clone(), cbr_then_args.clone())
                }
            }
            _ => unreachable!(),
        };

        // Inside the loop body: emit copies for the body-edge from header,
        // then walk body until reaching the header (back-edge stop).
        self.emit_edge_copies(info.body_b, &body_args, &mut body_stmts);
        self.walk(info.body_b, Some(header), &mut body_stmts);

        // If we couldn't inline the cond, the header's instructions need to
        // re-evaluate at the end of the body so the next iteration's check
        // sees up-to-date values. (And BEFORE the loop so the first check
        // sees initial values — emitted just below.)
        if !header_insts_handled_inline {
            self.emit_block_insts(header, &mut body_stmts);
        }

        // For the same reason, emit header's instructions BEFORE the loop too.
        if !header_insts_handled_inline {
            self.emit_block_insts(header, out);
        }

        out.push(Stmt::While {
            cond: cond_expr,
            body: body_stmts,
        });

        // Emit copies for the exit-edge into exit_b. (lower_while produces
        // empty exit_args, so this is a no-op in the typical case.)
        self.emit_edge_copies(info.exit_b, &exit_args, out);
    }

    /// If the header's cond is the result of a single instruction in the
    /// header, return the inlined expression and `true`; else return the
    /// fallback (use the cond Value's name) and `false`.
    fn try_inline_cond(&self, header: BlockId, cond_op: &Operand) -> (Expr, bool) {
        let blk = &self.func.blocks[header.0 as usize];
        if let Operand::Value(cv) = cond_op
            && let ValueDefKind::Inst(cv_inst) = self.func.values[cv.0 as usize].kind
            && blk.instructions.len() == 1
            && blk.instructions[0] == cv_inst
        {
            let kind = &self.func.instructions[cv_inst.0 as usize].kind;
            return (self.inst_kind_to_expr(kind), true);
        }
        // Fallback. If header has zero instructions, this still works — we
        // just read the cond name (or constant). If it has more than one
        // instruction, the caller emits all of them before the loop and at
        // body tail.
        (self.operand_to_expr(cond_op), blk.instructions.is_empty())
    }

    // -----------------------------------------------------------------
    // CFG analyses (cached)
    // -----------------------------------------------------------------

    fn loop_info(&mut self, b: BlockId) -> Option<LoopInfo> {
        if let Some(c) = self.loop_info_cache.get(&b) {
            return *c;
        }
        let result = self.compute_loop_info(b);
        self.loop_info_cache.insert(b, result);
        result
    }

    fn compute_loop_info(&mut self, b: BlockId) -> Option<LoopInfo> {
        let block = &self.func.blocks[b.0 as usize];
        let (then_b, else_b) = match &block.terminator {
            Terminator::CondBranch { then_b, else_b, .. } => (*then_b, *else_b),
            _ => return None,
        };

        // Back-edge test: is there a predecessor dominated by `b`? Only then
        // is `b` a loop header in the standard sense. Without this guard, a
        // cbr inside a loop body whose successors transitively reach the
        // OUTER header would be misidentified as a header.
        let preds = block.predecessors.clone();
        let has_back_edge = preds.iter().any(|&p| self.dominates(b, p));
        if !has_back_edge {
            return None;
        }

        let then_reaches_b = self.reaches(then_b, b);
        let else_reaches_b = self.reaches(else_b, b);
        match (then_reaches_b, else_reaches_b) {
            (true, false) => Some(LoopInfo {
                body_b: then_b,
                exit_b: else_b,
            }),
            (false, true) => Some(LoopInfo {
                body_b: else_b,
                exit_b: then_b,
            }),
            (true, true) => panic!(
                "loop header bb{} has both cbr branches reaching the header — \
                 unsupported irreducible CFG; needs a real relooper",
                b.0
            ),
            (false, false) => panic!(
                "block bb{} has a back-edge predecessor but neither cbr branch \
                 reaches the header — CFG analysis disagrees with itself",
                b.0
            ),
        }
    }

    /// `dominator` dominates `dominated` iff every path from entry to
    /// `dominated` passes through `dominator`. Equivalent: removing
    /// `dominator` leaves `dominated` unreachable from entry.
    fn dominates(&mut self, dominator: BlockId, dominated: BlockId) -> bool {
        if dominator == dominated {
            return true;
        }
        if dominator == self.func.entry {
            return true;
        }
        let key = (dominator, dominated);
        if let Some(v) = self.dom_cache.get(&key) {
            return *v;
        }
        let mut visited: HashSet<BlockId> = HashSet::new();
        let mut stack = vec![self.func.entry];
        let mut reaches_target = false;
        while let Some(b) = stack.pop() {
            if b == dominator {
                continue;
            }
            if !visited.insert(b) {
                continue;
            }
            if b == dominated {
                reaches_target = true;
                break;
            }
            for (succ, _) in self.func.blocks[b.0 as usize].terminator.successors() {
                stack.push(succ);
            }
        }
        let dominates = !reaches_target;
        self.dom_cache.insert(key, dominates);
        dominates
    }

    fn reaches(&mut self, start: BlockId, target: BlockId) -> bool {
        // Convenience: reachability through forward edges.
        let r = self.forward_reachable(start);
        r.contains(&target)
    }

    fn forward_reachable(&mut self, start: BlockId) -> HashSet<BlockId> {
        if let Some(s) = self.reachable_cache.get(&start) {
            return s.clone();
        }
        let mut visited = HashSet::new();
        let mut stack = vec![start];
        while let Some(b) = stack.pop() {
            if !visited.insert(b) {
                continue;
            }
            for (succ, _) in self.func.blocks[b.0 as usize].terminator.successors() {
                stack.push(succ);
            }
        }
        self.reachable_cache.insert(start, visited.clone());
        visited
    }

    /// Given an if/else cbr with branches `b1` and `b2`, find the diamond's
    /// post-dominator (the join). Returns None if the branches have no
    /// common forward-reachable block, or if the only common block is the
    /// parent's `stop` (in which case the parent will continue from there
    /// after the if statement returns control).
    fn find_diamond_join(
        &mut self,
        b1: BlockId,
        b2: BlockId,
        parent_stop: Option<BlockId>,
    ) -> Option<BlockId> {
        let r1 = self.forward_reachable(b1);
        let r2 = self.forward_reachable(b2);
        let common: Vec<BlockId> = r1.intersection(&r2).copied().collect();
        if common.is_empty() {
            return None;
        }
        // Filter out the parent's stop — if both branches reach it, that's
        // the parent's responsibility, not an if-internal join.
        let candidates: Vec<BlockId> = common
            .into_iter()
            .filter(|b| Some(*b) != parent_stop)
            .collect();
        if candidates.is_empty() {
            return None;
        }
        // Pick the candidate that is a post-dominator: a block whose
        // forward-reachable set is a SUBSET of every other candidate's
        // forward-reachable set (i.e., everyone passes through it before
        // diverging again). For the shapes our converter produces this is
        // unambiguous and equal to the explicit join_b in lower_if.
        //
        // Implementation: pick the candidate dominated-from-b1 by no other
        // candidate. Equivalent: the one closest to b1 among those that all
        // other candidates reach. Naive O(n^2) loop is fine — n is the
        // diamond's interior size, which is small in practice.
        let mut best = candidates[0];
        let mut best_dist = usize::MAX;
        for &c in &candidates {
            let d = self.distance(b1, c).min(self.distance(b2, c));
            if d < best_dist {
                best_dist = d;
                best = c;
            }
        }
        Some(best)
    }

    /// Forward distance (BFS hop count) from `start` to `target`. Returns
    /// `usize::MAX` if unreachable.
    fn distance(&self, start: BlockId, target: BlockId) -> usize {
        if start == target {
            return 0;
        }
        let mut visited: HashSet<BlockId> = HashSet::new();
        let mut frontier = vec![start];
        let mut depth = 0usize;
        while !frontier.is_empty() {
            depth += 1;
            let mut next: Vec<BlockId> = Vec::new();
            for b in frontier {
                if !visited.insert(b) {
                    continue;
                }
                for (succ, _) in self.func.blocks[b.0 as usize].terminator.successors() {
                    if succ == target {
                        return depth;
                    }
                    next.push(succ);
                }
            }
            frontier = next;
        }
        usize::MAX
    }
}

// ============================================================================
// Free helpers
// ============================================================================

/// Two-pass renamer. Returns a `(value_index → final_name)` map and the
/// list of `_tN`-style hoisted-temporary names whose `let` declarations
/// must be emitted at the function body's top.
///
/// Determinism: the COLLECT pass walks Values in index order (which is
/// the order `FunctionBuilder` allocates them — left-to-right by
/// instruction emission and parameter declaration). The ASSIGN pass
/// honors the COLLECT order so two equivalent SSA modules produce
/// byte-identical names.
///
/// Scope: collisions are detected at function granularity, not per JS
/// block. This is a deliberate simplification documented in the ticket
/// — two source-level `i`s in unrelated for-loops emit as `i` and
/// `i_1` rather than both as `i` (which would be JS-legal under block
/// scoping). Output is slightly uglier in pathological cases, but the
/// renamer is dramatically simpler. Revisit if quality complaints
/// surface.
fn assign_names(
    sm: &ssa::Module,
    func: &Function,
    cfg: &EmitConfig,
) -> (HashMap<u32, String>, Vec<String>) {
    let mut final_names: HashMap<u32, String> = HashMap::new();
    let mut committed: HashMap<String, u32> = HashMap::new();
    let mut hoisted: Vec<String> = Vec::new();
    let mut t_counter: u32 = 0;

    // Pre-claim every pinned name to the pinned Value. Caller
    // staging-conflict (two distinct Values pinned to the same name)
    // panics here.
    let pinned_names: std::collections::HashSet<&str> =
        cfg.immovable_names.values().map(|s| s.as_str()).collect();
    for (&vi, name) in &cfg.immovable_names {
        if let Some(&prior) = committed.get(name)
            && prior != vi
        {
            panic!(
                "EmitConfig.immovable_names pins both %{prior} and %{vi} to `{name}`. \
                 Caller bug — two distinct Values cannot share an immovable name."
            );
        }
        committed.insert(name.clone(), vi);
        final_names.insert(vi, name.clone());
    }

    for (i, def) in func.values.iter().enumerate() {
        let vi = i as u32;
        if final_names.contains_key(&vi) {
            // Already assigned via pinning; if it's an Inst or
            // BlockParam its name still needs to surface in `hoisted`
            // so the function-top let-block declares it.
            if matches!(
                def.kind,
                ValueDefKind::Inst(_) | ValueDefKind::BlockParam { .. }
            ) {
                hoisted.push(final_names[&vi].clone());
            }
            continue;
        }
        match &def.kind {
            ValueDefKind::Removed => continue,
            ValueDefKind::Arg(_) => {
                let preferred = def.source_name.map(|s| sm.strings.resolve(&s).to_string());
                let chosen = pick_name(
                    preferred,
                    &mut committed,
                    vi,
                    &pinned_names,
                    &mut t_counter,
                    false,
                );
                final_names.insert(vi, chosen);
            }
            ValueDefKind::Inst(_) | ValueDefKind::BlockParam { .. } => {
                let preferred = def.source_name.map(|s| sm.strings.resolve(&s).to_string());
                let chosen = pick_name(
                    preferred,
                    &mut committed,
                    vi,
                    &pinned_names,
                    &mut t_counter,
                    true,
                );
                hoisted.push(chosen.clone());
                final_names.insert(vi, chosen);
            }
        }
    }

    (final_names, hoisted)
}

/// Resolve the final name for a single Value. `for_let` controls only
/// whether the synthesized fallback uses the `_tN` (true) or `_aN`
/// (false) prefix; both prefixes share the committed-name space because
/// emitted JS scopes them together.
///
/// `pinned` is the set of names already locked by EmitConfig. The
/// suffix-collision path skips any candidate that lands on a pinned
/// name; bare-name preference for a pinned name fails through to the
/// suffix path (the pin's owner already claimed the bare slot).
fn pick_name(
    preferred: Option<String>,
    committed: &mut HashMap<String, u32>,
    vi: u32,
    pinned: &std::collections::HashSet<&str>,
    t_counter: &mut u32,
    for_let: bool,
) -> String {
    if let Some(want) = preferred {
        if !committed.contains_key(&want) && !pinned.contains(want.as_str()) {
            committed.insert(want.clone(), vi);
            return want;
        }
        let mut k: u32 = 1;
        loop {
            let candidate = format!("{want}_{k}");
            if !committed.contains_key(&candidate) && !pinned.contains(candidate.as_str()) {
                committed.insert(candidate.clone(), vi);
                return candidate;
            }
            k += 1;
        }
    }

    loop {
        let prefix = if for_let { "_t" } else { "_a" };
        let candidate = format!("{prefix}{}", *t_counter);
        *t_counter += 1;
        if !committed.contains_key(&candidate) && !pinned.contains(candidate.as_str()) {
            committed.insert(candidate.clone(), vi);
            return candidate;
        }
    }
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emit::emit;
    use crate::ir::lower_ssa;
    use crate::lower::parse_and_lower;

    /// Full pipeline: source → tree-IR → SSA-IR → tree-IR → emit. Returns
    /// the final emitted JS string. Asserts that the resulting JS re-parses
    /// (a minimum well-formedness check).
    fn full_roundtrip(src: &str) -> String {
        let tm = parse_and_lower(src).expect("frontend lowering must succeed");
        let sm = lower_ssa::lower_module(&tm);
        sm.validate()
            .unwrap_or_else(|errs| panic!("SSA module failed validation: {errs:#?}"));
        let tm2 = lower_module(&sm);
        let out = emit(&tm2);
        // Re-parse the emitted JS. This is a structural sanity check; it
        // doesn't prove behavior preservation but catches gross syntax bugs.
        parse_and_lower(&out).unwrap_or_else(|e| {
            panic!("emitted JS failed to re-parse: {e}\n--- emitted ---\n{out}\n--- end ---")
        });
        out
    }

    #[test]
    fn fibonacci_roundtrip() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let out = full_roundtrip(src);
        // Spot-check: function name preserved (top-level decl), recursive
        // call appears as identifier reference.
        assert!(out.contains("function fibonacci"));
        assert!(
            out.contains("fibonacci("),
            "expected at least one call to fibonacci, got:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn sum_to_loop_roundtrip() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("function sum_to"));
        assert!(out.contains("while"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn if_else_mutation_roundtrip() {
        let src = r#"
            function f(c) {
                let x = 1;
                if (c) { x = 2; } else { x = 3; }
                return x;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("function f"));
        assert!(out.contains("if"));
        insta::assert_snapshot!(out);
    }

    #[test]
    fn lexical_shadowing_roundtrip() {
        let src = r#"
            function f() {
                let x = 1;
                if (true) { let x = 2; }
                return x;
            }
        "#;
        let out = full_roundtrip(src);
        // The outer x = 1 is unchanged; SSA's trivial-param removal
        // collapses the join into `ret 1`. Out-of-SSA must preserve that —
        // the function should return 1.
        assert!(
            out.contains("return 1"),
            "expected literal 'return 1' in output:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn early_return_in_branch_roundtrip() {
        let src = r#"
            function f(c) {
                if (c) return 1;
                return 2;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("return 1"));
        assert!(out.contains("return 2"));
        insta::assert_snapshot!(out);
    }

    /// Idempotence (weaker than behavior preservation, doesn't need node):
    /// running the pipeline twice should produce the same output as once.
    /// SSA conversion is deterministic given the tree-IR, and out-of-SSA
    /// is deterministic given the SSA. So once the pipeline reaches a
    /// fixed point (no more renames happen), it stays there.
    #[test]
    fn pipeline_idempotent_on_fibonacci() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let once = full_roundtrip(src);
        let twice = full_roundtrip(&once);
        assert_eq!(once, twice, "pipeline should be idempotent after one pass");
    }

    #[test]
    fn pipeline_idempotent_on_sum_to() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let once = full_roundtrip(src);
        let twice = full_roundtrip(&once);
        assert_eq!(once, twice, "pipeline should be idempotent after one pass");
    }

    /// SSA validator must accept the pre-out-of-SSA module for every test
    /// input. This is implicit in `full_roundtrip`'s `sm.validate()` call,
    /// but a dedicated test makes the contract explicit.
    #[test]
    fn ssa_module_validates_for_all_test_inputs() {
        let inputs = [
            "function f(n) { if (n <= 1) return n; return f(n-1) + f(n-2); }",
            "function f(n) { let i=0; while (i<n) { i=i+1; } return i; }",
            "function f(c) { let x=1; if (c) { x=2; } else { x=3; } return x; }",
        ];
        for src in inputs {
            let tm = parse_and_lower(src).unwrap();
            let sm = lower_ssa::lower_module(&tm);
            sm.validate()
                .unwrap_or_else(|errs| panic!("SSA validation failed for {src}: {errs:#?}"));
        }
    }

    // ====================================================================
    // Globals model — round-trips. Each test confirms the unbound-ident
    // lowering survives the full pipeline and re-emits as JS that re-parses.
    // ====================================================================

    /// The current out-of-SSA pass emits each SSA value as its own
    /// `_tN = ...;` statement; expression chaining like `console.log("hi")`
    /// only re-fuses once an inliner runs (a future pass). We therefore
    /// assert on the parts — `console` and `.log` and the string arg —
    /// that the IR reliably preserves. The snapshot pins the full shape.
    #[test]
    fn console_log_roundtrip() {
        let src = r#"function f() { console.log("hi"); }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("console"), "missing `console`:\n{out}");
        assert!(out.contains(".log"), "missing `.log`:\n{out}");
        assert!(out.contains("\"hi\""), "missing `\"hi\"`:\n{out}");
        assert!(
            !out.contains("globalThis"),
            "GlobalThis indirection should be invisible in emit:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn math_floor_roundtrip() {
        let src = r#"function f(x) { return Math.floor(x); }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("Math"), "missing `Math`:\n{out}");
        assert!(out.contains(".floor"), "missing `.floor`:\n{out}");
        insta::assert_snapshot!(out);
    }

    #[test]
    fn unbound_write_roundtrip() {
        let src = r#"function f() { x = 1; }"#;
        let out = full_roundtrip(src);
        // The store re-emits as a bare assignment (`x = 1;`), not
        // `globalThis.x = 1;`.
        assert!(out.contains("x = 1"), "expected `x = 1` in output:\n{out}");
        assert!(
            !out.contains("globalThis"),
            "GlobalThis indirection should be invisible in emit:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    /// `globalThis.foo` collapses to bare `foo` after the round-trip — the
    /// SSA already merges them (verified in `lower_ssa::tests::
    /// globalthis_dot_foo_normalizes_to_bare_foo`); this test confirms the
    /// out-of-SSA pass picks the bare-identifier surface form. The literal
    /// `globalThis` substring must NOT survive — that's the whole point of
    /// the lossy normalization.
    #[test]
    fn globalthis_dot_foo_emits_as_bare_foo() {
        let src = r#"function f() { return globalThis.foo; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("foo"), "expected `foo` in output:\n{out}");
        assert!(
            !out.contains("globalThis"),
            "explicit `globalThis` indirection should be erased:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    #[test]
    fn lexical_console_shadowing_roundtrip() {
        let src = r#"function f() { let console = 1; return console; }"#;
        let out = full_roundtrip(src);
        // The local binding shadows the global, so the return collapses to
        // `return 1` after trivial-param removal — no `console` ident,
        // no global lookup.
        assert!(
            out.contains("return 1"),
            "shadowed `console` should fold to `return 1`:\n{out}"
        );
        insta::assert_snapshot!(out);
    }

    // ====================================================================
    // Behavior-preservation tests (feature-gated on `node-tests`).
    //
    // These shell out to `node` to evaluate both the original source and
    // the round-tripped source on a battery of inputs, asserting equal
    // outputs. They're off by default so CI runs without a Node runtime;
    // enable with `cargo test --features node-tests`.
    // ====================================================================

    #[cfg(feature = "node-tests")]
    fn node_eval(src: &str, harness: &str) -> String {
        use std::io::Write;
        use std::process::{Command, Stdio};
        let program = format!("{src}\n{harness}\n");
        let mut child = Command::new("node")
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn node — install Node or disable the node-tests feature");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(program.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        if !out.status.success() {
            panic!(
                "node failed:\nstderr: {}\nprogram:\n{}",
                String::from_utf8_lossy(&out.stderr),
                program
            );
        }
        String::from_utf8(out.stdout).unwrap()
    }

    #[cfg(feature = "node-tests")]
    fn assert_behavior_equivalent(src: &str, harness: &str) {
        let original = node_eval(src, harness);
        let roundtripped_src = full_roundtrip(src);
        let roundtripped = node_eval(&roundtripped_src, harness);
        assert_eq!(
            original, roundtripped,
            "behavior diverged after round-trip\n\
             ===== original src =====\n{src}\n\
             ===== roundtripped src =====\n{roundtripped_src}\n\
             ===== original output =====\n{original}\
             ===== roundtripped output =====\n{roundtripped}"
        );
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn fibonacci_behavior_preserved() {
        let src = r#"
            function fibonacci(n) {
                if (n <= 1) return n;
                return fibonacci(n - 1) + fibonacci(n - 2);
            }
        "#;
        let harness = "for (let i = 0; i <= 12; i++) console.log(fibonacci(i));";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn sum_to_behavior_preserved() {
        let src = r#"
            function sum_to(n) {
                let i = 0;
                let s = 0;
                while (i < n) {
                    s = s + i;
                    i = i + 1;
                }
                return s;
            }
        "#;
        let harness = "for (let i = 0; i <= 20; i++) console.log(sum_to(i));";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn if_else_behavior_preserved() {
        let src = r#"
            function f(c) {
                let x = 1;
                if (c) { x = 2; } else { x = 3; }
                return x;
            }
        "#;
        let harness =
            "console.log(f(true)); console.log(f(false)); console.log(f(0)); console.log(f(1));";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn lexical_shadowing_behavior_preserved() {
        let src = r#"
            function f() {
                let x = 1;
                if (true) { let x = 2; }
                return x;
            }
        "#;
        let harness = "console.log(f());";
        assert_behavior_equivalent(src, harness);
    }

    #[cfg(feature = "node-tests")]
    #[test]
    fn early_return_behavior_preserved() {
        let src = r#"
            function f(c) {
                if (c) return 1;
                return 2;
            }
        "#;
        let harness = "console.log(f(true)); console.log(f(false)); console.log(f(0)); console.log(f(\"x\"));";
        assert_behavior_equivalent(src, harness);
    }

    // =====================================================================
    // T1: name-preservation acceptance tests
    // =====================================================================

    /// Two distinct named lets in the same scope retain their original
    /// names. Non-trivial inits used so the SSA actually creates a Value
    /// for each binding (constant-init lets fold away in lowering — the
    /// name has nothing to attach to).
    #[test]
    fn distinct_named_lets_preserve_names() {
        let src = r#"function f(a) { let x = a + 1; let y = a + 2; return x + y; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("let x"), "expected `let x`:\n{out}");
        assert!(out.contains("let y"), "expected `let y`:\n{out}");
    }

    /// Function parameters emit by their original source names.
    #[test]
    fn parameter_names_preserved() {
        let src = r#"function f(a, b) { return a + b; }"#;
        let out = full_roundtrip(src);
        assert!(
            out.contains("function f(a, b)"),
            "expected `function f(a, b)`:\n{out}"
        );
        assert!(!out.contains("_a0"));
        assert!(!out.contains("_a1"));
    }

    /// Reassignment becomes a fresh `let` with a numeric suffix. This is
    /// the documented T1 gap: the SSA shape produces two distinct values
    /// both annotated with the source name, and the renamer suffix-
    /// disambiguates the second. A future ticket will recover the
    /// original mutation pattern.
    ///
    /// Non-trivial RHS so the SSA produces an Inst for each assignment
    /// (constants fold to `Operand::Const` and have nothing to tag).
    #[test]
    fn reassignment_becomes_fresh_let() {
        let src = r#"function f(a) { let x = a + 1; x = a + 2; return x; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("let x"), "expected `let x`:\n{out}");
        assert!(
            out.contains("let x_1"),
            "expected `let x_1` for the reassignment:\n{out}"
        );
        assert!(
            out.contains("return x_1"),
            "expected `return x_1` (the latest value):\n{out}"
        );
    }

    /// Lexical shadowing: outer x and inner x get x and x_1 under the
    /// conservative function-flat scope. JS would allow both to be `x`
    /// under block scoping, but the renamer treats the entire function
    /// body as one namespace for collision detection. See `assign_names`
    /// for the rationale.
    ///
    /// Non-trivial inits, plus a use of the inner x in the same branch,
    /// so SSA actually materializes both bindings as Values.
    #[test]
    fn lexical_shadow_inner_gets_suffix() {
        let src = r#"
            function f(a, c) {
                let x = a + 1;
                let r = 0;
                if (c) {
                    let x = a + 2;
                    r = x;
                }
                return x + r;
            }
        "#;
        let out = full_roundtrip(src);
        assert!(out.contains("let x"), "outer x missing:\n{out}");
        // The inner let gets x_1 — even though JS scoping would allow
        // both to be `x` in distinct blocks. Conservative simplification.
        assert!(
            out.contains("let x_1"),
            "inner shadowing x should emit as x_1:\n{out}"
        );
    }

    /// Synthetic intermediates remain `_tN`. We do not invent names for
    /// expression sub-results.
    #[test]
    fn complex_expr_intermediates_emit_as_t() {
        let src = r#"function f(a, b, c, d) { let r = (a + b) * (c - d); return r; }"#;
        let out = full_roundtrip(src);
        assert!(out.contains("let r"), "expected `let r`:\n{out}");
        // The (a+b) and (c-d) sub-expressions have no source names.
        assert!(
            out.contains("_t0") || out.contains("_t1"),
            "expected synthesized intermediates `_tN`:\n{out}"
        );
    }

    // =====================================================================
    // EmitConfig — pinned-name behavior. Direct unit tests against the
    // renamer rather than via the full pipeline.
    // =====================================================================

    /// A pinned Value gets exactly its required name. A second Value
    /// preferring the same source name is suffix-disambiguated, even
    /// though it would otherwise win the bare slot by walk order.
    #[test]
    fn emit_config_pin_routes_other_value_around() {
        use crate::ir::ssa::{
            Constant, FunctionBuilder, InstKind, Module as SsaModule, Operand, Terminator,
        };
        let mut module = SsaModule::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v_a = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(1)),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        let v_b = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(3)),
                rhs: Operand::Const(Constant::Int32(4)),
            })
            .unwrap();
        let y_id = b.intern("y");
        b.set_source_name(v_a, y_id);
        b.set_source_name(v_b, y_id);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(v_b)),
        });
        b.finish();

        // Pin v_b to "y". The earlier-walked v_a — which prefers "y"
        // via source_name — must be suffix-disambiguated to "y_1".
        let mut cfg = EmitConfig::default();
        cfg.immovable_names.insert(v_b.0, "y".to_string());
        let tm_out = lower_module_with_config(&module, &cfg);
        let out = crate::emit::emit(&tm_out);
        assert!(
            out.contains("let y_1"),
            "v_a should be routed to y_1 because v_b is pinned:\n{out}"
        );
        assert!(
            out.contains("let y"),
            "v_b should still get exactly `y`:\n{out}"
        );
    }

    /// Pinning two distinct Values to the same name is a caller bug.
    #[test]
    #[should_panic(expected = "two distinct Values cannot share an immovable name")]
    fn emit_config_pin_conflict_panics() {
        use crate::ir::ssa::{
            Constant, FunctionBuilder, InstKind, Module as SsaModule, Operand, Terminator,
        };
        let mut module = SsaModule::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v_a = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(1)),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        let v_b = b
            .add_inst(InstKind::AddGeneric {
                lhs: Operand::Const(Constant::Int32(3)),
                rhs: Operand::Const(Constant::Int32(4)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(v_b)),
        });
        b.finish();

        let mut cfg = EmitConfig::default();
        cfg.immovable_names.insert(v_a.0, "y".to_string());
        cfg.immovable_names.insert(v_b.0, "y".to_string());
        let _ = lower_module_with_config(&module, &cfg);
    }
}
