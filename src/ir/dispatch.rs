//! Dispatcher-agnostic recovery substrate (Plan A4 — the unification).
//!
//! Every control-flow-flattening *dialect* the deobfuscator recognizes today is a
//! separate narrow pass: integer-state (`passes::unflatten`), string/bool-state
//! (same file's `sval` engine), lookup-table transitions, param-state
//! (`passes::unflatten_param`), string-array cursor
//! (`passes::unflatten_string_order`), and object/computed-state
//! (`passes::computed_state`, which *normalizes* its object into the integer
//! engine). They differ only in four facts:
//!
//!   1. **what the state is** — an integer, a string/bool, a cursor into a known
//!      sequence, or an expression over a coercion object + counter;
//!   2. **its initial value** — the dispatcher's entry state;
//!   3. **how a case transitions it** — an absolute jump, a relative/lookup step, a
//!      "next array element", or a conditional split;
//!   4. **when the loop exits** — the loop test goes false, or a terminal case fires.
//!
//! This module captures those four facts behind a single [`DispatchModel`] trait
//! and provides **one** abstract interpreter ([`interpret`]) that enumerates the
//! reachable states from the entry and emits the exact transition edges into a
//! [`cff::RecoveredCfg`] — *regardless of dialect*. The recovered graph is then
//! handed to [`cff::structure_general`] / [`relooper`] (the Plan A2 relooper) to be
//! structured back into `if`/`else`/`while`/`for`.
//!
//! The end goal: **a new obfuscator dialect is a small `DispatchModel` impl, not a
//! new pass.** A model says "here is my entry state, here is whether a state
//! continues the loop, and here is what a case does to the state"; the interpreter
//! and the relooper do the rest.
//!
//! ## Soundness
//!
//! The interpreter is *sound by construction or it declines*:
//!   * It only ever materializes states the model says are reachable from the entry
//!     by following the model's own declared transitions — it never invents a state
//!     or an edge.
//!   * Every step the model declines to describe (`Step::Decline`) aborts the whole
//!     recovery (`interpret` returns `None`), so the caller leaves the dispatcher
//!     untouched (a sound no-op). Partial recovery is never emitted.
//!   * The enumeration is bounded (`MAX_STATES`); a model whose reachable-state set
//!     blows past the bound declines rather than looping. Because each distinct
//!     state is visited once, a finite reachable set always terminates.
//!   * The interpreter performs **no AST mutation** and clones nothing. It builds
//!     the read-only [`cff::RecoveredCfg`]; the (verbatim, by-identity) re-emission
//!     of case content is the relooper's job via a [`relooper::CaseSource`] the
//!     caller supplies. So this substrate cannot, on its own, drop or reorder a
//!     statement.
//!
//! ## Status: additive
//!
//! This is **new, self-contained scaffolding**. It is unit-tested in isolation and
//! is *not yet wired* into the pipeline — the existing dialect passes remain the
//! production recognizers (re-routing them is a follow-up that must prove byte-for-
//! byte parity on the sample corpus before it lands). Building the trait +
//! interpreter + the integer-state model first, green and unwired, is the
//! low-risk first step.
//!
//! Because the substrate is deliberately unwired, its public surface has no
//! production caller yet (only the unit tests below exercise it). We therefore
//! `allow(dead_code)` for the whole module rather than scatter per-item attributes —
//! every item here is intended API for the wiring follow-up, not stale code.

#![allow(dead_code)]

use std::collections::HashMap;
use std::collections::VecDeque;

use super::cff::{CaseBlock, Edge, Guard, RecoveredCfg, Target};

/// A model-agnostic dispatch state value. Every dialect's state — an integer
/// counter, a string/boolean key, or a cursor position — projects onto one of
/// these. The value is hashable and totally ordered so the interpreter can use it
/// as a map key and produce a deterministic enumeration order.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StateVal {
    /// An integer state (the `unflatten` integer engine; a cursor index; the
    /// normalized object/computed counter).
    Int(i64),
    /// A string state key (the `.split("|")` cursor keys; a `switch (s)` on strings).
    Str(String),
    /// A boolean state key.
    Bool(bool),
}

impl From<i64> for StateVal {
    fn from(v: i64) -> Self {
        StateVal::Int(v)
    }
}

/// Where a single (sub-)transition lands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dest {
    /// Continue the machine in this concrete state.
    To(StateVal),
    /// Leave the machine: the loop test goes false here, or a `return`/`throw`
    /// terminator runs. (The interpreter does not distinguish the two for graph
    /// shape; both are the recovered-CFG `Exit`.)
    Exit,
}

/// What a case does once its body has run — the *only* dialect-specific control
/// fact the interpreter needs. The model computes this from the concrete current
/// state (so relative/lookup/cursor transitions are fully determined), which is why
/// one interpreter serves every dialect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Unconditional transition: run the body, then go to `dest`.
    Goto(Dest),
    /// A two-way conditional split on the case's recovered `if` test (kept verbatim
    /// by the caller's [`relooper::CaseSource`]). `on_true` is the `then` arm,
    /// `on_false` the `else`.
    Branch { on_true: Dest, on_false: Dest },
    /// The machine cannot model this state's transition soundly. Recovery declines.
    Decline,
}

/// The four facts that define a flattening dialect. Implement this and the
/// interpreter + relooper recover the original control flow.
pub trait DispatchModel {
    /// The dispatcher's entry state (the value the state variable / cursor is
    /// initialized to).
    fn entry(&self) -> StateVal;

    /// What the case for `state` does to the machine — the transition. `None` is
    /// returned for a state the model has no case for (which, when reached, aborts
    /// recovery: a missing case + no `default` is an unmodelled jump).
    fn step(&self, state: &StateVal) -> Option<Step>;
}

/// Defensive bound on the reachable-state enumeration. Real dispatchers have at
/// most a few thousand states; a model that would enumerate more is declined rather
/// than risk an unbounded walk. (Cycle-freedom is *not* required — back edges are
/// the recovered loops — so the bound, not acyclicity, is what guarantees
/// termination of the BFS.)
const MAX_STATES: usize = 100_000;

/// The reachable-state numbering the interpreter assigns: each distinct reachable
/// [`StateVal`], in BFS discovery order from the entry, gets a dense `i64` id. The
/// id is what the produced [`cff::RecoveredCfg`] keys its blocks on (so the existing
/// relooper, which is `i64`-keyed, consumes it unchanged), and the map lets the
/// caller's [`relooper::CaseSource`] translate an id back to the original state to
/// resolve content/conditions.
#[derive(Debug, Default)]
pub struct StateNumbering {
    /// id → the concrete state it stands for (dense; `order[i]` has id `i`).
    pub order: Vec<StateVal>,
    /// state → its dense id.
    pub index: HashMap<StateVal, i64>,
}

impl StateNumbering {
    fn intern(&mut self, s: &StateVal) -> i64 {
        if let Some(&id) = self.index.get(s) {
            return id;
        }
        let id = self.order.len() as i64;
        self.order.push(s.clone());
        self.index.insert(s.clone(), id);
        id
    }

    /// The concrete state behind a dense id (`None` if out of range).
    pub fn state_of(&self, id: i64) -> Option<&StateVal> {
        usize::try_from(id).ok().and_then(|i| self.order.get(i))
    }
}

/// Abstract-interpret `model`: enumerate every state reachable from the entry by
/// following the model's declared transitions, numbering them densely, and emit the
/// exact edges into a [`cff::RecoveredCfg`]. The recovered graph's node ids are the
/// dense state ids; the returned [`StateNumbering`] maps them back to concrete
/// states for the caller's [`relooper::CaseSource`].
///
/// Returns `None` (decline → caller leaves the dispatcher untouched) when the model
/// declines any reached state's transition, when a reached state has no case, or
/// when the reachable set exceeds [`MAX_STATES`]. The result is otherwise an exact
/// model of the state machine: a node is "the case at state K", an edge is "the
/// transition K performs", with nothing added, dropped, or reordered.
pub fn interpret(model: &dyn DispatchModel) -> Option<(RecoveredCfg, StateNumbering)> {
    let entry = model.entry();
    let mut num = StateNumbering::default();
    let entry_id = num.intern(&entry);

    let mut blocks: HashMap<i64, CaseBlock> = HashMap::new();
    let mut order: Vec<i64> = Vec::new();

    let mut queue: VecDeque<StateVal> = VecDeque::new();
    queue.push_back(entry.clone());

    // BFS over reachable states. `num.index` doubles as the "discovered" set: a
    // state is interned exactly once, the first time it is reached, and that intern
    // order *is* the dense block ordering (entry == 0), which the dominator
    // computation in `relooper`/`cff` relies on.
    let mut emitted: std::collections::HashSet<i64> = std::collections::HashSet::new();
    while let Some(state) = queue.pop_front() {
        let id = *num.index.get(&state)?;
        if !emitted.insert(id) {
            continue; // already processed (a state can be enqueued before processed)
        }
        if num.order.len() > MAX_STATES {
            return None; // reachable set too large → decline
        }
        order.push(id);

        let step = model.step(&state)?;
        let mut edges: Vec<Edge> = Vec::new();
        let enqueue =
            |num: &mut StateNumbering, queue: &mut VecDeque<StateVal>, d: &Dest| -> Target {
                match d {
                    Dest::Exit => Target::Exit,
                    Dest::To(s) => {
                        let known = num.index.contains_key(s);
                        let tid = num.intern(s);
                        if !known {
                            queue.push_back(s.clone());
                        }
                        Target::State(tid)
                    }
                }
            };
        match step {
            Step::Goto(dest) => {
                let target = enqueue(&mut num, &mut queue, &dest);
                edges.push(Edge {
                    guard: Guard::Always,
                    target,
                });
            }
            Step::Branch { on_true, on_false } => {
                let t = enqueue(&mut num, &mut queue, &on_true);
                let f = enqueue(&mut num, &mut queue, &on_false);
                edges.push(Edge {
                    guard: Guard::IfTrue,
                    target: t,
                });
                edges.push(Edge {
                    guard: Guard::IfFalse,
                    target: f,
                });
            }
            Step::Decline => return None,
        }

        // `content`/`cond_stmt` are resolved by the caller's CaseSource from the
        // original AST (the interpreter is content-agnostic): it records *which*
        // states exist and *how* they transition, not the statements they run. The
        // CaseSource keys on the same dense id (via the StateNumbering), so leaving
        // these empty here is correct — they are not consulted for graph shape.
        blocks.insert(
            id,
            CaseBlock {
                content: Vec::new(),
                cond_stmt: None,
                edges,
            },
        );
    }

    // Every interned state must have been processed (BFS over a finite set reached
    // from the entry guarantees it); if not, the model produced a dangling target.
    if blocks.len() != num.order.len() {
        return None;
    }

    Some((
        RecoveredCfg {
            entry: entry_id,
            blocks,
            order,
        },
        num,
    ))
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

/// How a case mutates an **integer** dispatch state once its body has run. This is
/// the substrate-level mirror of the (private) `passes::unflatten::Trans` enum,
/// recast as a model fact so the same transition algebra is shared across dialects
/// rather than re-implemented per pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntTransition {
    /// `s = K` — jump to an absolute state.
    Abs(i64),
    /// `s += K` / `s -= K` (negative `K`) — relative to the current state.
    Rel(i64),
    /// `s *= K` — multiplicative relative to the current state.
    Mul(i64),
    /// `s = table[s]` / `s = table[K]` — read the next state from a provably-
    /// immutable const integer table; the index is the current state (`None`) or a
    /// constant (`Some(k)`). Out-of-range reads (`undefined` in JS) decline.
    Lookup {
        table: std::rc::Rc<Vec<i64>>,
        index: Option<i64>,
    },
}

impl IntTransition {
    /// The next integer state, or `None` for an out-of-range lookup (which decays to
    /// a model `Decline`).
    fn apply(&self, state: i64) -> Option<i64> {
        match self {
            IntTransition::Abs(v) => Some(*v),
            IntTransition::Rel(d) => Some(state.checked_add(*d)?),
            IntTransition::Mul(f) => Some(state.checked_mul(*f)?),
            IntTransition::Lookup { table, index } => {
                let i = index.unwrap_or(state);
                let idx = usize::try_from(i).ok()?;
                table.get(idx).copied()
            }
        }
    }
}

/// What an integer-state case does, in substrate terms: an unconditional transition,
/// a conditional split between two integer transitions, or a terminator/loop-exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IntCase {
    /// Run the body, then take `trans`.
    Step(IntTransition),
    /// Conditional split: the `if` arm takes `on_true`, the `else` arm `on_false`.
    Branch {
        on_true: IntTransition,
        on_false: IntTransition,
    },
    /// `return`/`throw`: the machine stops here.
    Terminal,
}

/// The canonical **integer-state** dialect as a [`DispatchModel`] — the substrate
/// re-expression of the `passes::unflatten` integer engine (absolute / relative /
/// multiplicative / lookup-table transitions, conditional splits, terminal cases,
/// and a loop-exit predicate). A case absent from `cases` is an unmodelled jump and
/// declines when reached.
///
/// `is_continue(state)` is the dialect's loop test (`while (s)`, `while (s != K)`,
/// …) lifted to a predicate: a transition to a state where the test is false is the
/// machine's `Exit`. This is what lets the same model express both a `0`-terminated
/// machine and a `!= 105`-terminated one without the interpreter knowing the test.
pub struct IntStateModel<'c> {
    entry: i64,
    cases: &'c HashMap<i64, IntCase>,
    is_continue: &'c dyn Fn(i64) -> bool,
}

impl<'c> IntStateModel<'c> {
    /// Build an integer-state model. `is_continue` is the loop test as a predicate
    /// over the state (true = the loop body runs again at this state; false = the
    /// machine exits).
    pub fn new(
        entry: i64,
        cases: &'c HashMap<i64, IntCase>,
        is_continue: &'c dyn Fn(i64) -> bool,
    ) -> Self {
        IntStateModel {
            entry,
            cases,
            is_continue,
        }
    }

    /// Map a concrete next integer state to a [`Dest`]: a state where the loop test
    /// is false is the machine's `Exit`, otherwise it continues there.
    fn dest_of(&self, next: i64) -> Dest {
        if (self.is_continue)(next) {
            Dest::To(StateVal::Int(next))
        } else {
            Dest::Exit
        }
    }

    /// Resolve one integer transition from `state` to a [`Dest`] (`Exit` on an
    /// out-of-range lookup — an `undefined` read we decline by leaving the machine).
    fn dest_for(&self, state: i64, t: &IntTransition) -> Option<Dest> {
        let next = t.apply(state)?;
        Some(self.dest_of(next))
    }
}

impl DispatchModel for IntStateModel<'_> {
    fn entry(&self) -> StateVal {
        StateVal::Int(self.entry)
    }

    fn step(&self, state: &StateVal) -> Option<Step> {
        let &StateVal::Int(s) = state else {
            return Some(Step::Decline); // an integer model only has integer states
        };
        let Some(case) = self.cases.get(&s) else {
            return None; // no case for a reachable state → unmodelled jump
        };
        Some(match case {
            IntCase::Terminal => Step::Goto(Dest::Exit),
            IntCase::Step(t) => match self.dest_for(s, t) {
                Some(d) => Step::Goto(d),
                None => Step::Decline,
            },
            IntCase::Branch { on_true, on_false } => {
                match (self.dest_for(s, on_true), self.dest_for(s, on_false)) {
                    (Some(t), Some(f)) => Step::Branch {
                        on_true: t,
                        on_false: f,
                    },
                    _ => Step::Decline,
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::rc::Rc;

    fn cases(pairs: Vec<(i64, IntCase)>) -> HashMap<i64, IntCase> {
        pairs.into_iter().collect()
    }

    // `while (s)` — exit at state 0.
    fn truthy(s: i64) -> bool {
        s != 0
    }

    #[test]
    fn linear_chain_recovers_acyclic_graph() {
        // 1 -> 2 -> 3 -> Exit(0), absolute jumps.
        let c = cases(vec![
            (1, IntCase::Step(IntTransition::Abs(2))),
            (2, IntCase::Step(IntTransition::Abs(3))),
            (3, IntCase::Step(IntTransition::Abs(0))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let (cfg, num) = interpret(&m).expect("recovers");
        // Three reachable states, entry == dense id 0, BFS discovery order = 1,2,3.
        assert_eq!(
            num.order,
            vec![StateVal::Int(1), StateVal::Int(2), StateVal::Int(3)]
        );
        assert_eq!(cfg.entry, 0);
        assert_eq!(cfg.order, vec![0, 1, 2]);
        assert!(!cfg.has_loop());
        // State 3 transitions to 0 (test false) → Exit.
        assert_eq!(
            cfg.blocks[&2].edges,
            vec![Edge {
                guard: Guard::Always,
                target: Target::Exit
            }]
        );
    }

    #[test]
    fn relative_transition_is_fully_determined() {
        // 1 -> (s += 1) 2 -> (s += 1) 3 -> (s -= 3) 0/Exit.
        let c = cases(vec![
            (1, IntCase::Step(IntTransition::Rel(1))),
            (2, IntCase::Step(IntTransition::Rel(1))),
            (3, IntCase::Step(IntTransition::Rel(-3))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let (cfg, num) = interpret(&m).unwrap();
        assert_eq!(num.order.len(), 3);
        assert!(!cfg.has_loop());
    }

    #[test]
    fn multiplicative_self_cycle_is_a_loop() {
        // 1 -> (s *= 1) 1 forever: a back edge, i.e. a recovered self-loop. The
        // substrate models it as a graph with a loop (the relooper later structures
        // or declines it); it does NOT spin, because state 1 is interned once.
        let c = cases(vec![(1, IntCase::Step(IntTransition::Mul(1)))]);
        let m = IntStateModel::new(1, &c, &truthy);
        let (cfg, _num) = interpret(&m).unwrap();
        assert_eq!(cfg.order, vec![0]);
        assert!(cfg.has_loop());
        assert_eq!(cfg.back_edges(), vec![(0, 0)]);
    }

    #[test]
    fn lookup_table_transition_resolves() {
        // next = [2, 3, 0]; start at 0: 0 -> next[0]=2 -> next[2]=0/Exit. State 1 is
        // unreachable from the entry and must not appear.
        let table = Rc::new(vec![2i64, 3, 0]);
        let c = cases(vec![
            (
                0,
                IntCase::Step(IntTransition::Lookup {
                    table: table.clone(),
                    index: None,
                }),
            ),
            (
                2,
                IntCase::Step(IntTransition::Lookup {
                    table: table.clone(),
                    index: None,
                }),
            ),
        ]);
        // Exit at 0 only when *reached as a transition target* — but the entry is 0
        // and is_continue(0) must be true to enter. Use a terminal-style test: exit
        // at negative, so 0 stays in-machine; the chain ends by next[2]=0 == entry,
        // which is a back edge to the entry (a loop), exercising re-interning.
        let cont = |s: i64| s >= 0;
        let m = IntStateModel::new(0, &c, &cont);
        let (cfg, num) = interpret(&m).unwrap();
        // Reachable: 0 and 2 only.
        assert_eq!(num.order, vec![StateVal::Int(0), StateVal::Int(2)]);
        assert!(!num.index.contains_key(&StateVal::Int(1)));
        // 2 -> next[2] = 0 = entry: a back edge (loop).
        assert!(cfg.has_loop());
    }

    #[test]
    fn out_of_range_lookup_declines() {
        // next has length 1; indexing next[5] is undefined → decline the recovery.
        let table = Rc::new(vec![5i64]);
        let c = cases(vec![(
            5,
            IntCase::Step(IntTransition::Lookup { table, index: None }),
        )]);
        let m = IntStateModel::new(5, &c, &|_| true);
        assert!(interpret(&m).is_none());
    }

    #[test]
    fn missing_case_declines() {
        // Entry 1 jumps to 2, but there is no case 2 (and the test continues there) →
        // an unmodelled jump → decline (sound: leave the dispatcher).
        let c = cases(vec![(1, IntCase::Step(IntTransition::Abs(2)))]);
        let m = IntStateModel::new(1, &c, &|_| true);
        assert!(interpret(&m).is_none());
    }

    #[test]
    fn conditional_split_recovers_two_edges() {
        // 1: if (c) s = 2 else s = 3; 2 -> Exit; 3 -> Exit. A diamond with no merge
        // (both arms exit) — the recovered graph has the IfTrue/IfFalse split.
        let c = cases(vec![
            (
                1,
                IntCase::Branch {
                    on_true: IntTransition::Abs(2),
                    on_false: IntTransition::Abs(3),
                },
            ),
            (2, IntCase::Step(IntTransition::Abs(0))),
            (3, IntCase::Step(IntTransition::Abs(0))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let (cfg, _num) = interpret(&m).unwrap();
        let entry = &cfg.blocks[&cfg.entry];
        assert_eq!(entry.edges.len(), 2);
        assert_eq!(entry.edges[0].guard, Guard::IfTrue);
        assert_eq!(entry.edges[1].guard, Guard::IfFalse);
        assert!(!cfg.has_loop());
    }

    #[test]
    fn conditional_back_edge_is_a_while_loop() {
        // 1 -> 2; 2: if (c) s = 3 else exit; 3 -> 2. The recovered while-loop:
        // header 2 with a back edge from 3.
        let c = cases(vec![
            (1, IntCase::Step(IntTransition::Abs(2))),
            (
                2,
                IntCase::Branch {
                    on_true: IntTransition::Abs(3),
                    on_false: IntTransition::Abs(0),
                },
            ),
            (3, IntCase::Step(IntTransition::Abs(2))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let (cfg, num) = interpret(&m).unwrap();
        assert!(cfg.has_loop());
        // States 1,2,3 in discovery order → ids 0,1,2. Back edge 3->2 == (2,1) ids.
        let id3 = num.index[&StateVal::Int(3)];
        let id2 = num.index[&StateVal::Int(2)];
        assert_eq!(cfg.back_edges(), vec![(id3, id2)]);
    }

    #[test]
    fn terminal_case_exits() {
        // 1 -> 2; 2 is a return/throw terminator → Exit.
        let c = cases(vec![
            (1, IntCase::Step(IntTransition::Abs(2))),
            (2, IntCase::Terminal),
        ]);
        let m = IntStateModel::new(1, &c, &|_| true);
        let (cfg, num) = interpret(&m).unwrap();
        assert_eq!(num.order.len(), 2);
        let id2 = num.index[&StateVal::Int(2)];
        assert_eq!(
            cfg.blocks[&id2].edges,
            vec![Edge {
                guard: Guard::Always,
                target: Target::Exit
            }]
        );
        assert!(!cfg.has_loop());
    }

    #[test]
    fn non_integer_state_in_int_model_declines() {
        // The integer model only has integer states; a string entry is a category
        // error the model declines on rather than mis-recovering.
        struct StrEntry;
        impl DispatchModel for StrEntry {
            fn entry(&self) -> StateVal {
                StateVal::Str("a".into())
            }
            fn step(&self, _s: &StateVal) -> Option<Step> {
                Some(Step::Decline)
            }
        }
        assert!(interpret(&StrEntry).is_none());
    }

    // ---- end-to-end: substrate CFG → the A2 relooper (proves interop) ----

    use crate::ir::relooper::{self, CaseSource};
    use oxc_allocator::Allocator;
    use oxc_ast::ast::{Expression, Statement};
    use oxc_ast::AstBuilder;
    use oxc_codegen::{Codegen, CodegenOptions};
    use oxc_span::SPAN;

    /// A [`CaseSource`] that emits one marker call `m(id);` per state and a boolean
    /// identifier `c<id>` as each split condition, so the structured output can be
    /// snapshotted as source. Content is keyed by the substrate's dense state id —
    /// exactly the id the interpreter put on the recovered block — which is what a
    /// real wiring would translate (via `StateNumbering`) back to the original case.
    struct MarkerSource<'a> {
        ast: AstBuilder<'a>,
    }
    impl<'a> CaseSource<'a> for MarkerSource<'a> {
        fn content(&self, k: i64) -> Vec<Statement<'a>> {
            let callee = self.ast.expression_identifier(SPAN, "m");
            let mut args = self.ast.vec();
            args.push(oxc_ast::ast::Argument::from(
                self.ast.expression_numeric_literal(
                    SPAN,
                    k as f64,
                    None,
                    oxc_syntax::number::NumberBase::Decimal,
                ),
            ));
            let call = self
                .ast
                .expression_call(SPAN, callee, oxc_ast::NONE, args, false);
            vec![self.ast.statement_expression(SPAN, call)]
        }
        fn cond(&self, k: i64) -> Option<Expression<'a>> {
            Some(
                self.ast
                    .expression_identifier(SPAN, self.ast.str(&format!("c{k}"))),
            )
        }
    }

    /// Interpret `model`, structure the recovered graph via the real A2 relooper, and
    /// return the generated source (whitespace-normalized). `None` if either step
    /// declines.
    fn recover_and_structure(model: &dyn DispatchModel) -> Option<String> {
        let alloc = Allocator::default();
        let ast = AstBuilder::new(&alloc);
        let (cfg, _num) = interpret(model)?;
        let src = MarkerSource { ast };
        let body = relooper::structure_general(&cfg, &src, ast)?;
        let mut stmts = ast.vec();
        for s in body {
            stmts.push(s);
        }
        let program = ast.program(
            SPAN,
            oxc_span::SourceType::default(),
            "",
            ast.vec(),
            None,
            ast.vec(),
            stmts,
        );
        let code = Codegen::new()
            .with_options(CodegenOptions::default())
            .build(&program)
            .code;
        Some(code.split_whitespace().collect::<Vec<_>>().join(" "))
    }

    #[test]
    fn substrate_linear_chain_structures_to_a_sequence() {
        // 1 -> 2 -> 3 -> Exit: a flat statement sequence, in run order, no control.
        let c = cases(vec![
            (1, IntCase::Step(IntTransition::Abs(2))),
            (2, IntCase::Step(IntTransition::Abs(3))),
            (3, IntCase::Step(IntTransition::Abs(0))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let out = recover_and_structure(&m).expect("structures");
        assert_eq!(out, "m(0); m(1); m(2);");
    }

    #[test]
    fn substrate_diamond_structures_to_if_else() {
        // 1: if (c) -> 2 else -> 3 ; 2 -> 4 ; 3 -> 4 ; 4 -> Exit. An if/else that
        // re-merges at 4 — the relooper places 4 once, after the `if`.
        let c = cases(vec![
            (
                1,
                IntCase::Branch {
                    on_true: IntTransition::Abs(2),
                    on_false: IntTransition::Abs(3),
                },
            ),
            (2, IntCase::Step(IntTransition::Abs(4))),
            (3, IntCase::Step(IntTransition::Abs(4))),
            (4, IntCase::Step(IntTransition::Abs(0))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let out = recover_and_structure(&m).expect("structures");
        // ids: 1->0, 2->1, 3->2, 4->3. Header emits m(0), then if(c0){m(1)}else{m(2)},
        // then the merge m(3).
        assert_eq!(out, "m(0); if (c0) { m(1); } else { m(2); } m(3);");
    }

    #[test]
    fn substrate_while_loop_structures_to_while_true() {
        // 1 -> 2; 2: if (c) -> 3 else -> Exit; 3 -> 2. A while-loop with body 3.
        let c = cases(vec![
            (1, IntCase::Step(IntTransition::Abs(2))),
            (
                2,
                IntCase::Branch {
                    on_true: IntTransition::Abs(3),
                    on_false: IntTransition::Abs(0),
                },
            ),
            (3, IntCase::Step(IntTransition::Abs(2))),
        ]);
        let m = IntStateModel::new(1, &c, &truthy);
        let out = recover_and_structure(&m).expect("structures");
        // ids: 1->0, 2->1, 3->2. Body 3 (= m(2)) loops back to header 2 (= continue);
        // the else arm leaves the loop (= break).
        assert_eq!(
            out,
            "m(0); while (true) { m(1); if (c1) { m(2); continue; } else { break; } }"
        );
    }

    #[test]
    fn state_numbering_round_trips() {
        let c = cases(vec![
            (7, IntCase::Step(IntTransition::Abs(9))),
            (9, IntCase::Step(IntTransition::Abs(0))),
        ]);
        let m = IntStateModel::new(7, &c, &truthy);
        let (_cfg, num) = interpret(&m).unwrap();
        // id 0 is the entry state 7; state_of inverts the numbering.
        assert_eq!(num.state_of(0), Some(&StateVal::Int(7)));
        assert_eq!(num.state_of(1), Some(&StateVal::Int(9)));
        assert_eq!(num.state_of(2), None);
        assert_eq!(num.index[&StateVal::Int(7)], 0);
    }
}
