# Plan P1 — Whole-program sound constant resolution

**Goal:** resolve the *symbolic integer constants* that gate control-flow
recovery — flattened-switch case labels (`case var591:`), param-state entry
arguments (`f(var944, …)`), and table indices — to their literal values, **soundly
under a happens-before proof**, and expose the result as a reusable resolver the
unflattener, decoder-lift, and the folds all consult. This is the unblocker that
makes the cascade compound: decode → **resolve constants** → unflatten → fold →
DCE → rename.

**Why first:** the unflattener (merged) and `unflatten_param` only fire on
*numeric* labels/entries. On samples 1/4/11 those are still symbolic at the point
the passes run, so recovery stalls. Phase 2 (CFG recovery) is dead without this.

---

## 1. Success criteria (measurable)

- `global-const-inline` (or its successor) resolves the case-label tables of
  samples 1, 4, 11 — `[gci] inlined / candidates` ratio approaches 1 on those
  files (today it is far below 1; see its `GCI_DEBUG` line).
- After P1, `unflatten` + `unflatten_param` fire on the *local-state* and
  *param-state* dispatchers of samples 1/4/11 whose only remaining blocker was
  symbolic labels/entries → visible `switch` count drop.
- **Zero** equivalence-harness regressions; new adversarial tests (a should-resolve
  case paired with a must-NOT-resolve sibling) all green.
- sample_7's Base91 table lifts (the `var = pureCall()` value-binding completion).

## 2. What already exists (build on, don't reinvent)

| Capability | Where | Limitation for our goal |
|---|---|---|
| Intraprocedural SSA reaching-def const-prop | `src/ir/` — `SsaOpt` (`mod.rs`), `ssa::resolve` (`ssa.rs`), `Lowerer` (`cfg.rs`), `Dominators` (`dom.rs`) | One scope only; can't carry a constant from an *init* function to a *dispatcher* function. |
| Cross-function write-once inlining + happens-before proof | `src/passes/global_const_inline.rs` (4-condition core) | Conditions are narrow: init function called **exactly once** via an *unconditional bare* `F();` that is a direct child of a region; write at conditional-depth 0. Misses IIFE/`.call` inits, multiple inits, idempotency-guarded inits. |
| Escape / immutability analysis | `src/ir/analysis.rs` — `collect_candidates`, `mutable_aggregate_tables` | Already the soundness gate for "value can't be mutated through an alias." Reuse verbatim. |
| Sandbox value evaluation | `src/sandbox.rs`, `decoder_lift.rs` | Scalar-only `EvalValue`; no array results; no `var = pureCall()` preamble bindings. |

**Key realization:** every *primitive* P1 needs (reaching-defs, dominance, escape
analysis, the ordering proof) already exists. P1 is **unification + generalization
+ exposure as a shared resolver**, not new dataflow machinery.

## 3. The precise gap

1. **No whole-program constant map.** `unflatten`, `unflatten_param`, and
   `decoder_lift` each re-derive "is this an int literal?" locally (`int_literal`).
   None can ask "does this *identifier* provably equal constant C here?"
2. **The cross-function proof is too narrow.** `global_const_inline` rejects the
   real init shapes in samples 1/4/11 (IIFE-wrapped init, `.call`-invoked init,
   several init functions, `if (!inited) { inited = 1; … }` guards).
3. **The value axis is incomplete** for two cases that are "constant resolution"
   in spirit: array-returning decoders (`EvalValue::Arr`) and decoders whose
   preamble needs a `var x = purecall()` binding (sample_7).

## 4. Design

### 4.1 A whole-program constant lattice + `ConstResolver`

Introduce `src/ir/consts.rs` exposing:

```rust
pub struct ConstResolver { /* SymbolId -> ResolvedConst */ }
pub enum ResolvedConst<'a> { Int(i64), Lit(Expression<'a>) }

impl ConstResolver {
    pub fn build(program, scoping, allocator) -> ConstResolver;
    pub fn int(&self, sym: SymbolId) -> Option<i64>;
    pub fn lit(&self, sym: SymbolId) -> Option<&Expression<'a>>;
    /// Resolve an expression that is a literal OR a bare reference to a resolved
    /// symbol (incl. `-x`). The drop-in replacement for `int_literal`.
    pub fn resolve_int(&self, expr, scoping) -> Option<i64>;
}
```

A symbol `X` maps to constant `C` **iff all** hold (sufficient, conservative —
bail when unsure):

1. **Single value-establishing write.** Exactly one declarator-init **or** bare
   `X = <expr>` assignment program-wide; no compound/update/delete/destructuring/
   for-head write anywhere. (Reuse `analysis.rs` write census.)
2. **The write's RHS is constant.** An integer/inlineable literal, *or* a pure
   expression over already-resolved constants — evaluate with a **small fixpoint**
   (resolve the leaves first; re-run until no change), so `var b = a + 1` resolves
   once `a` does. Pure-expression evaluation reuses the sandbox under the existing
   double-eval determinism guard.
3. **Value can't be mutated through an alias.** `X` (and any table it indexes) is
   in the immutable set from `mutable_aggregate_tables` / `collect_candidates`.
4. **Every read happens-after the write** (the soundness crux — see §4.2).

### 4.2 The happens-before proof, generalized (the crux)

Extract `global_const_inline`'s ordering logic into `src/ir/ordering.rs` as a
reusable `OrderingOracle`, then widen the *sufficient conditions* it accepts:

- **Intra-function read** (write and read in the same function `F`): the write's
  CFG node dominates the read's CFG node. Reuse `Lowerer` + `Dominators` directly.
- **Cross-function read** (write in init `W`, read in dispatcher `R`): require that
  `W` *completes* before `R`'s function is ever entered. v1 sufficient condition,
  generalized from `global_const_inline`:
  - `W` is invoked **unconditionally during module setup** — accept the bare
    `W();`, **and** newly: `(function W(){…})()` IIFE, `W.call(this)/W.apply(this)`,
    and a `W` invoked from within another already-proven-setup function.
  - The write inside `W` runs at conditional-depth 0, **or** is guarded only by a
    *monotone idempotency guard* `if (!g) { g = 1; … }` where `g` is write-once
    after — recognize this idiom (it still runs the write exactly once, before any
    later read).
  - `R`'s enclosing function is **not reachable** from the setup phase before `W`
    completes — i.e. `R` is a dispatcher invoked later. v1: `R`'s function is
    defined but only *called* after the setup region (the same region-index test
    `global_const_inline` already does, extended across the recognized setup set).
- **Build a coarse call graph** (`Map<fn-scope, called-fn-scopes>`, unconditional
  vs conditional call sites) to compute the "setup-completed" set as a fixpoint
  from the module entry. Conservative: any call we can't classify → that callee is
  *not* assumed setup-complete (so reads in it won't resolve). Async/callback edges
  are never setup edges in v1.

This stays **sound by sufficient condition**: we only resolve when we can *prove*
the ordering; everything else is left symbolic (and simply doesn't fold — the
existing safe behavior).

### 4.3 Exposure + materialization

- The **resolver is the engine.** Wire `ConstResolver::resolve_int` into
  `unflatten` (`state_init`, `parse_loop_test`, `parse_cases` labels,
  `is_state_transition`) and `unflatten_param` (the `Census` call-entry detection),
  replacing their local `int_literal`. This makes symbolic labels/entries fold
  *without* mutating the AST first — loose coupling.
- **Materialization stays a separate, optional step** (what `global_const_inline`
  does today): rewrite resolved reads to literals so passes that *don't* take a
  resolver still benefit, and DCE can delete the now-dead table + init function.
  Reimplement `global_const_inline` as a thin client of `ConstResolver` +
  `OrderingOracle` (delete its bespoke census; keep its materialization + DCE).

### 4.4 Value-axis completions (constant resolution of non-scalars)

- **`EvalValue::Arr(Vec<EvalValue>)`** in `src/sandbox.rs` + a splice path in
  `decoder_lift.rs`, **gated on a tight size cap** so it can never bloat (issue 09:
  materializing sample_9's 131 KB bytecode is net-negative — the cap excludes it).
  Enables array-returning decoders that return *small* literal arrays.
- **`var = pureCall()` preamble bindings** for `decoder_lift` (issue 03 / sample_7):
  model a `var v = <pure expression involving calls to pure funcs/globals>` binding
  as a preamble line proven pure-by-evaluation. Combined with admitting the
  global-object-detection idiom (`globalThis`/`window`/`Function("return this")`),
  this unblocks sample_7's Base91 table (1763 call sites).

## 5. Integration & ordering in the pipeline

`ConstResolver::build` runs once per fixpoint round (cheap: census + a small
fixpoint). Pipeline position: build it at the *top* of each round so
`global-const-inline` → `unflatten` → `unflatten-param` → folds all see the same
resolved facts. The resolver is rebuilt each round, so newly-resolved constants
(e.g. after decoder-lift exposes a literal table) feed the next round.

## 6. Soundness & test strategy

The differential equivalence harness (`tests/equivalence.rs`, executes
original-vs-deobfuscated in QuickJS) is the net. For **every** resolution rule, add
an adversarial pair (mirroring the existing `ssa.rs` tests):

| Should resolve | Must NOT resolve (sibling) |
|---|---|
| `var L=19; …switch(s){case L:…}` | `var L=19; L=20; …case L:` (two writes) |
| init `f(){X=5}; f(); read X` | init `if(c) f(); read X` (conditional init) |
| read after unconditional `f()` | read **before** `f()` runs (use-before-init) |
| IIFE/`.call` init | init reachable only via a callback |
| `var b=a+1` after `a` resolves | `var b=a+1; a=9` (a not write-once) |
| pure `var v = dec()` binding | `var v = Date.now()` (nondeterministic — determinism guard rejects) |

Plus: the full sample suite must stay idempotent + reparse (`tests/samples.rs`),
and `tests/global_const_inline.rs` must still pass after the rewrite-as-client.

## 7. Milestones (each lands green)

- **M1 — Refactor, no behavior change.** Extract `OrderingOracle` from
  `global_const_inline`; add `ConstResolver` skeleton wrapping the existing census;
  reimplement `global_const_inline` as a client. Full suite green + parity check
  (output identical on all samples).
- **M2 — Generalize the ordering proof.** IIFE/`.call`/multi-init/idempotency-guard
  recognition + the coarse call graph. Measure `[gci]` firing on samples 1/4/11.
  New adversarial tests.
- **M3 — Wire the resolver into the unflatteners.** `resolve_int` in `unflatten` +
  `unflatten_param`. Measure `switch` drop on samples 1/4/11.
- **M4 — Value-axis.** `EvalValue::Arr` (size-capped) + `var=pureCall()` preamble +
  global-object idiom. Measure sample_7 Base91 lift (`call`/`pick` sites → strings).

## 8. Risks & mitigations

- **The ordering proof is the soundness crux.** Mitigation: only ever *widen the
  sufficient conditions*; never assume ordering we can't prove. Every widening
  ships with its must-NOT-resolve adversarial test. Keep async/callback edges out.
- **Pure-expression fixpoint could loop / mis-evaluate.** Bound the fixpoint
  iterations; require the sandbox determinism guard for any evaluated RHS.
- **`global_const_inline` rewrite-as-client could regress.** M1 is a strict parity
  refactor gated on byte-identical sample output before widening anything.

## 9. Out of scope

General interprocedural constant propagation (only **write-once**); values that
depend on host state; non-integer symbolic resolution beyond inlineable literals.
Full VM bytecode materialization (the size cap excludes it — that is Phase 3).

---

## STATUS (P0 execution)

- **M1/S1 — DONE.** `OrderingOracle` (`src/ir/ordering.rs`) + `ConstResolver`
  (`src/ir/consts.rs`) extracted; `global_const_inline` is a thin materialization
  client. Parity verified (174→175 green); see commit `refactor(P1/S1)`.
- **M2/S2 — partial + re-diagnosed.** Sound widening landed: `var X = (IIFE)()`
  and `X = (IIFE)()` declarator/assignment IIFE bodies are now setup regions
  (adversarial pair `global_const_inline_nested_iife_init`). **But the headline
  premise was wrong** — s1/s4/s11 labels are NOT setup-scoped symbolic constants:
  s1 is a VM-in-a-constructor, s4's labels are already numeric (structural
  blocker), s11's are 2D const-array reads. See `issues/12`. `[gci]` on s1/s4/s11
  is unchanged by design; effort routed to S3 + P2 per P0 §8.
- **M3/S3 — wiring `resolve_int`** into the unflatteners (sound; resolver only
  widens vs the local `int_literal`).
- **M4/S4 — deferred** (value-axis; independent of the firing path).
