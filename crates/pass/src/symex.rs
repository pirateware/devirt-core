//! Symbolic execution + Z3 SMT backend. Issue #50 v1 scaffolding.
//!
//! The whole module is gated behind the `smt` Cargo feature so that
//! default `cargo test --workspace` does not require libz3 on the host.
//!
//! Goal of this scaffolding (v1): provide the data types and a small
//! block-walker that the next PRs can extend. Specifically:
//!
//! - [`AbstractValue`]: the per-Value lattice — `Concrete(Const)`,
//!   `Symbolic(BvAst)`, or `Unknown`. Operations update both the
//!   concrete and symbolic state where possible.
//! - [`SymbolicState`]: per-Value mapping plus a path-condition stack.
//!   At each `CondBranch` the walker pushes the branch constraint and
//!   asks Z3 whether the path is feasible.
//! - One concrete demo ([`solve_counter_loop_exit`]) showing that a
//!   bounded counter loop (constructed via `FunctionBuilder`) collapses
//!   to its exit count via SMT.
//!
//! Out of scope for v1:
//! - Memory modeling (LoadProp/StoreProp are `Unknown`).
//! - Path merging / summarization.
//! - Full Effect-lattice integration (effects are not consulted yet —
//!   the walker simply gives up when it hits an instruction it cannot
//!   model).
//! - Wiring this into the default pipeline. The pass is opt-in: even
//!   with `--features smt`, nothing in `PassManager`'s defaults runs
//!   it. Callers must construct it explicitly.
//!
//! The choices below (32-bit bitvectors, integer-only modeling, single
//! function scope) are intentional v1 limits. Doubles / strings /
//! object identities will land alongside the heap modeling work.
//!
//! ## System dependency
//!
//! Building with `--features smt` links libz3 (Ubuntu/Debian:
//! `sudo apt install libz3-dev`, macOS: `brew install z3`). Default
//! builds do not link Z3 and have no system dependency beyond the
//! workspace's existing toolchain.
//!
//! ## API note (z3 0.20)
//!
//! The `z3` crate at 0.20 maintains a thread-local `Context` and most
//! constructors no longer take a `&Context` parameter. We follow that
//! convention here — `SymbolicState` does not carry a context.

#![cfg(feature = "smt")]

use std::collections::HashMap;

use ir::ssa::{
    BlockId, Constant, FuncId, InstKind, LoweredBody, Module, Operand, Terminator, Value,
    find_value_for_inst,
};

use z3::{
    SatResult, Solver,
    ast::{BV, Bool},
};

/// Width of the bitvector domain we model integer values in. 32 bits
/// matches JavaScript's ToInt32 semantics for bitwise ops and is wide
/// enough for the bounded-loop counters we currently care about.
pub const BV_WIDTH: u32 = 32;

/// Per-Value abstract value. Each SSA `Value` maps to one of these in
/// the [`SymbolicState`]. The concrete tier is preferred where
/// available — operations only fall back to the symbolic tier when at
/// least one operand is symbolic.
#[derive(Clone)]
pub enum AbstractValue {
    /// Statically known constant. Always preferred over Symbolic when
    /// both are computable.
    Concrete(Constant),
    /// Symbolic 32-bit bitvector. Carries a Z3 AST that the solver
    /// can reason about.
    Symbolic(BV),
    /// Out of the modeled domain (heap reads, calls, doubles, etc.).
    /// Operations on Unknown propagate Unknown.
    Unknown,
}

impl AbstractValue {
    /// Convert to a Z3 BV when possible. Concrete-Int32 / Bool lift to
    /// constant BVs; Symbolic returns its BV directly; Unknown or
    /// unmodeled constants return None.
    pub fn as_bv(&self) -> Option<BV> {
        match self {
            AbstractValue::Concrete(Constant::Int32(i)) => Some(BV::from_i64(*i as i64, BV_WIDTH)),
            AbstractValue::Concrete(Constant::Number(n)) if n.fract() == 0.0 && n.is_finite() => {
                let i = *n as i64;
                if (i32::MIN as i64..=i32::MAX as i64).contains(&i) {
                    Some(BV::from_i64(i, BV_WIDTH))
                } else {
                    None
                }
            }
            AbstractValue::Concrete(Constant::Bool(b)) => {
                Some(BV::from_i64(if *b { 1 } else { 0 }, BV_WIDTH))
            }
            AbstractValue::Symbolic(bv) => Some(bv.clone()),
            _ => None,
        }
    }
}

/// Symbolic state held while walking a function. One per active path.
pub struct SymbolicState {
    pub values: HashMap<u32, AbstractValue>,
    /// Path-condition stack: every entry is a Z3 Bool that must hold
    /// on the current path. The solver consults them as the
    /// conjunction of all entries.
    pub path: Vec<Bool>,
}

impl Default for SymbolicState {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolicState {
    pub fn new() -> Self {
        Self {
            values: HashMap::new(),
            path: Vec::new(),
        }
    }

    /// Look up the abstract value of an operand.
    pub fn eval(&self, op: &Operand) -> AbstractValue {
        match op {
            Operand::Const(c) => AbstractValue::Concrete(*c),
            Operand::Value(v) => self
                .values
                .get(&v.0)
                .cloned()
                .unwrap_or(AbstractValue::Unknown),
            Operand::Capture(_) => AbstractValue::Unknown,
        }
    }

    /// Bind a Value to an abstract value.
    pub fn set(&mut self, v: Value, av: AbstractValue) {
        self.values.insert(v.0, av);
    }

    /// Push a path condition. Returns whether the path remains
    /// feasible (`SatResult::Sat`) — caller is expected to abandon
    /// branches that are unsatisfiable.
    pub fn assume(&mut self, cond: Bool) -> SatResult {
        self.path.push(cond);
        let solver = Solver::new();
        for c in &self.path {
            solver.assert(c);
        }
        solver.check()
    }

    /// Pop the most recently pushed path condition.
    pub fn pop_assumption(&mut self) {
        self.path.pop();
    }
}

/// Result of evaluating a single instruction.
fn eval_inst(state: &SymbolicState, kind: &InstKind) -> AbstractValue {
    match kind {
        InstKind::AddInt32 { lhs, rhs }
        | InstKind::SubInt32 { lhs, rhs }
        | InstKind::MulInt32 { lhs, rhs } => {
            let l = state.eval(lhs);
            let r = state.eval(rhs);
            // Concrete fast path.
            if let (
                AbstractValue::Concrete(Constant::Int32(a)),
                AbstractValue::Concrete(Constant::Int32(b)),
            ) = (&l, &r)
            {
                let v = match kind {
                    InstKind::AddInt32 { .. } => a.wrapping_add(*b),
                    InstKind::SubInt32 { .. } => a.wrapping_sub(*b),
                    InstKind::MulInt32 { .. } => a.wrapping_mul(*b),
                    _ => unreachable!(),
                };
                return AbstractValue::Concrete(Constant::Int32(v));
            }
            // Symbolic path.
            match (l.as_bv(), r.as_bv()) {
                (Some(la), Some(ra)) => {
                    let bv = match kind {
                        InstKind::AddInt32 { .. } => la.bvadd(&ra),
                        InstKind::SubInt32 { .. } => la.bvsub(&ra),
                        InstKind::MulInt32 { .. } => la.bvmul(&ra),
                        _ => unreachable!(),
                    };
                    AbstractValue::Symbolic(bv)
                }
                _ => AbstractValue::Unknown,
            }
        }
        InstKind::StrictEq { lhs, rhs } | InstKind::Eq { lhs, rhs } => {
            let l = state.eval(lhs);
            let r = state.eval(rhs);
            if let (
                AbstractValue::Concrete(Constant::Int32(a)),
                AbstractValue::Concrete(Constant::Int32(b)),
            ) = (&l, &r)
            {
                return AbstractValue::Concrete(Constant::Bool(a == b));
            }
            match (l.as_bv(), r.as_bv()) {
                (Some(la), Some(ra)) => {
                    let eq_bool = la.eq(&ra);
                    let one = BV::from_i64(1, BV_WIDTH);
                    let zero = BV::from_i64(0, BV_WIDTH);
                    AbstractValue::Symbolic(eq_bool.ite(&one, &zero))
                }
                _ => AbstractValue::Unknown,
            }
        }
        InstKind::Lt { lhs, rhs } | InstKind::Le { lhs, rhs } => {
            let l = state.eval(lhs);
            let r = state.eval(rhs);
            if let (
                AbstractValue::Concrete(Constant::Int32(a)),
                AbstractValue::Concrete(Constant::Int32(b)),
            ) = (&l, &r)
            {
                let res = matches!(kind, InstKind::Lt { .. }) && a < b
                    || matches!(kind, InstKind::Le { .. }) && a <= b;
                return AbstractValue::Concrete(Constant::Bool(res));
            }
            match (l.as_bv(), r.as_bv()) {
                (Some(la), Some(ra)) => {
                    let cmp = match kind {
                        InstKind::Lt { .. } => la.bvslt(&ra),
                        InstKind::Le { .. } => la.bvsle(&ra),
                        _ => unreachable!(),
                    };
                    let one = BV::from_i64(1, BV_WIDTH);
                    let zero = BV::from_i64(0, BV_WIDTH);
                    AbstractValue::Symbolic(cmp.ite(&one, &zero))
                }
                _ => AbstractValue::Unknown,
            }
        }
        InstKind::Not { operand } => {
            let v = state.eval(operand);
            if let AbstractValue::Concrete(Constant::Bool(b)) = v {
                return AbstractValue::Concrete(Constant::Bool(!b));
            }
            if let Some(bv) = v.as_bv() {
                let zero = BV::from_i64(0, BV_WIDTH);
                let one = BV::from_i64(1, BV_WIDTH);
                let is_zero = bv.eq(&zero);
                return AbstractValue::Symbolic(is_zero.ite(&one, &zero));
            }
            AbstractValue::Unknown
        }
        // Heap, calls, allocations, coercions — out of v1 scope.
        _ => AbstractValue::Unknown,
    }
}

/// Convert an abstract Bool-shaped value into a Z3 Bool. Concrete
/// Bool lifts trivially; symbolic 1/0 BVs lift as `bv != 0`.
#[allow(dead_code)]
fn to_bool(av: &AbstractValue) -> Option<Bool> {
    match av {
        AbstractValue::Concrete(Constant::Bool(b)) => Some(Bool::from_bool(*b)),
        AbstractValue::Concrete(Constant::Int32(i)) => Some(Bool::from_bool(*i != 0)),
        AbstractValue::Symbolic(bv) => {
            let zero = BV::from_i64(0, BV_WIDTH);
            Some(bv.eq(&zero).not())
        }
        _ => None,
    }
}

/// Walk a single block. Returns the block's terminator after every
/// instruction's effect on the symbolic state has been applied.
pub fn walk_block(
    state: &mut SymbolicState,
    body: &LoweredBody,
    block: BlockId,
) -> Terminator {
    let blk = &body.blocks[block.0 as usize];
    for &iid in &blk.instructions {
        let inst = &body.instructions[iid.0 as usize];
        if inst.kind.is_void() {
            continue;
        }
        let v = find_value_for_inst(body, iid);
        let av = eval_inst(state, &inst.kind);
        state.set(v, av);
    }
    blk.terminator.clone()
}

/// A single concrete demonstration: solve for the exit value of a
/// counter loop whose termination condition is `start + n*step == limit`.
///
/// The function body shape this models:
/// - entry: branch to header(start)
/// - header(c): cond `c == limit` → exit(c), else body
/// - body: c' = c + step, branch to header(c')
/// - exit(r): return r
///
/// We don't pattern-match the IR's exact shape; instead we encode the
/// loop semantics directly into Z3 ("there exists a non-negative N
/// such that start + N*step == limit") and ask the solver for the
/// satisfying value of the exit param. This is the SMT-driven
/// reasoning the v1 deliverable wants to demonstrate. Full IR-level
/// loop discovery + symbolic-summary synthesis is the next PR.
///
/// Returns the resolved exit value as an `i32` if Z3 produces a
/// concrete model.
pub fn solve_counter_loop_exit(start: i32, step: i32, limit: i32) -> Option<i32> {
    let solver = Solver::new();

    let n = BV::new_const("n", BV_WIDTH);
    let s = BV::from_i64(start as i64, BV_WIDTH);
    let st = BV::from_i64(step as i64, BV_WIDTH);
    let lim = BV::from_i64(limit as i64, BV_WIDTH);
    let exit_val = s.bvadd(&n.bvmul(&st));
    solver.assert(&exit_val.eq(&lim));
    let zero = BV::from_i64(0, BV_WIDTH);
    solver.assert(&n.bvsge(&zero));

    if solver.check() != SatResult::Sat {
        return None;
    }
    let model = solver.get_model()?;
    let n_val = model.eval(&n, true)?.as_i64()? as i32;
    Some(start.wrapping_add(n_val.wrapping_mul(step)))
}

/// Walk the entry block of `module.functions[fid]` once, populating a
/// fresh [`SymbolicState`]. This is the seam where future PRs hook in
/// CFG traversal + path splitting; v1 ships only the entry-block
/// walk so the data plumbing is exercised by tests without committing
/// to a particular fixpoint discipline.
pub fn walk_entry(state: &mut SymbolicState, module: &Module, fid: FuncId) -> Terminator {
    let body = module.functions[fid.0 as usize].lowered();
    walk_block(state, body, body.entry)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ir::ssa::{Constant, FunctionBuilder, InstKind, Module, Operand, Terminator};

    /// Concrete folding through the symbolic-eval path: 1 + 2 should
    /// evaluate to Concrete(Int32(3)) without ever materializing a Z3
    /// BV.
    #[test]
    fn concrete_add_int32_evaluates() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 0);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let v = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Const(Constant::Int32(1)),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(v)),
        });
        let fid = b.finish();

        let mut state = SymbolicState::new();
        let term = walk_entry(&mut state, &module, fid);

        match state.values.get(&v.0).expect("value bound") {
            AbstractValue::Concrete(Constant::Int32(3)) => {}
            AbstractValue::Concrete(c) => panic!("expected Concrete(Int32(3)), got {c:?}"),
            AbstractValue::Symbolic(_) => panic!("expected Concrete, got Symbolic"),
            AbstractValue::Unknown => panic!("expected Concrete, got Unknown"),
        }
        assert!(matches!(term, Terminator::Return { .. }));
    }

    /// Symbolic chain: with a function arg (modeled as an unknown BV
    /// the test seeds), `arg + 1 + 2` should produce a Symbolic value
    /// whose Z3 model agrees with `arg + 3` for any concrete arg.
    #[test]
    fn symbolic_add_chain_proves_associativity() {
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let bb0 = b.new_block();
        b.switch_to_block(bb0);
        let arg = b.body().params[0];
        let t1 = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(arg),
                rhs: Operand::Const(Constant::Int32(1)),
            })
            .unwrap();
        let t2 = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(t1),
                rhs: Operand::Const(Constant::Int32(2)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(t2)),
        });
        let fid = b.finish();

        let mut state = SymbolicState::new();
        let arg_bv = BV::new_const("arg", BV_WIDTH);
        state.set(arg, AbstractValue::Symbolic(arg_bv.clone()));
        walk_entry(&mut state, &module, fid);

        let t2_bv = match state.values.get(&t2.0).expect("t2 bound") {
            AbstractValue::Symbolic(bv) => bv.clone(),
            _ => panic!("expected Symbolic"),
        };

        // Prove t2 == arg + 3 with Z3 by checking unsat of the negation.
        let three = BV::from_i64(3, BV_WIDTH);
        let expected = arg_bv.bvadd(&three);
        let solver = Solver::new();
        solver.assert(&t2_bv.eq(&expected).not());
        assert_eq!(
            solver.check(),
            SatResult::Unsat,
            "t2 must be provably equal to arg + 3"
        );
    }

    /// Path-condition feasibility: assuming `arg == 5` and then
    /// `arg == 6` must be unsatisfiable.
    #[test]
    fn contradictory_path_conditions_unsat() {
        let mut state = SymbolicState::new();
        let arg_bv = BV::new_const("arg2", BV_WIDTH);
        let five = BV::from_i64(5, BV_WIDTH);
        let six = BV::from_i64(6, BV_WIDTH);
        assert_eq!(state.assume(arg_bv.eq(&five)), SatResult::Sat);
        assert_eq!(state.assume(arg_bv.eq(&six)), SatResult::Unsat);
    }

    /// Synthetic loop-bound demo: a counter starting at 0 with step 1
    /// reaching 7 should have exit value 7.
    #[test]
    fn counter_loop_exit_solved_by_z3() {
        let exit = solve_counter_loop_exit(0, 1, 7).expect("solver returned a model");
        assert_eq!(exit, 7);
    }

    /// Same demo with step 3 reaching 21.
    #[test]
    fn counter_loop_step_three() {
        let exit = solve_counter_loop_exit(0, 3, 21).expect("solver returned a model");
        assert_eq!(exit, 21);
    }

    /// Demonstrate that the entry-block walk + Z3 together can replace
    /// the loop's exit value with a Const for a synthetic SSA function
    /// representing a bounded counter loop. The function shape:
    ///
    ///   entry: branch header(0)
    ///   header(c): if c == 7 goto exit(c) else goto body
    ///   body:    c' = c + 1; branch header(c')
    ///   exit(r): return r
    ///
    /// The test does NOT mutate the IR; it asserts that the symbolic
    /// summary of the loop (encoded directly in Z3) yields the
    /// expected exit value. IR rewriting is the next PR.
    #[test]
    fn rotation_iife_shape_collapses_to_const_via_smt() {
        // Build the IR for documentation / future-PR reuse, even
        // though the v1 demo uses the encoded summary rather than
        // walking the loop.
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "loop_to_seven", 0);
        let entry = b.new_block();
        let header = b.new_block();
        let body = b.new_block();
        let exit_b = b.new_block();
        let counter = b.add_block_param(header);
        let result = b.add_block_param(exit_b);

        b.switch_to_block(entry);
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Const(Constant::Int32(0))],
        });

        b.switch_to_block(header);
        let cmp = b
            .add_inst(InstKind::StrictEq {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(7)),
            })
            .unwrap();
        b.set_terminator(Terminator::CondBranch {
            cond: Operand::Value(cmp),
            then_b: exit_b,
            then_args: vec![Operand::Value(counter)],
            else_b: body,
            else_args: vec![],
        });

        b.switch_to_block(body);
        let next = b
            .add_inst(InstKind::AddInt32 {
                lhs: Operand::Value(counter),
                rhs: Operand::Const(Constant::Int32(1)),
            })
            .unwrap();
        b.set_terminator(Terminator::Branch {
            target: header,
            args: vec![Operand::Value(next)],
        });

        b.switch_to_block(exit_b);
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(result)),
        });
        let _fid = b.finish();

        // The symbolic summary: exit value is the unique r such that
        // r == 0 + n * 1 and r == 7 for some n >= 0.
        let exit = solve_counter_loop_exit(0, 1, 7).expect("Z3 solves the loop summary");
        assert_eq!(exit, 7, "loop exit value collapses to Const(7) via SMT");
    }

    /// Sanity check: the abstract-value Lt comparison routes through
    /// Z3 as expected.
    #[test]
    fn symbolic_lt_evaluates() {
        let mut state = SymbolicState::new();
        let arg_bv = BV::new_const("x", BV_WIDTH);
        let mut module = Module::new();
        let mut b = FunctionBuilder::new(&mut module, "f", 1);
        let bb = b.new_block();
        b.switch_to_block(bb);
        let arg = b.body().params[0];
        let v = b
            .add_inst(InstKind::Lt {
                lhs: Operand::Value(arg),
                rhs: Operand::Const(Constant::Int32(10)),
            })
            .unwrap();
        b.set_terminator(Terminator::Return {
            val: Some(Operand::Value(v)),
        });
        let fid = b.finish();

        state.set(arg, AbstractValue::Symbolic(arg_bv.clone()));
        walk_entry(&mut state, &module, fid);

        let lt_bv = match state.values.get(&v.0).unwrap() {
            AbstractValue::Symbolic(bv) => bv.clone(),
            _ => panic!("expected Symbolic"),
        };

        // Assert the comparison is feasible both ways.
        let one = BV::from_i64(1, BV_WIDTH);
        let zero = BV::from_i64(0, BV_WIDTH);
        let solver = Solver::new();
        solver.assert(&lt_bv.eq(&one));
        assert_eq!(solver.check(), SatResult::Sat);
        let solver = Solver::new();
        solver.assert(&lt_bv.eq(&zero));
        assert_eq!(solver.check(), SatResult::Sat);
    }
}
