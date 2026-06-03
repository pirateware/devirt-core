# Plan: a CFG / SSA dataflow layer for the deobfuscator

Status: proposal. Target start: after the 2026-06 soundness-hardening pass
(commits `4ca4318..945e701`). Owner: TBD.

---

## 1. Why — the wall we actually hit

The current pipeline is a set of **structural AST passes** run to a fixpoint
(`driver::run_fixpoint`). That paradigm is now exhausted for the remaining
sample noise, for one reason: **every flow fact a transform needs is proven by
hand, and hand-proven flow facts are unsound by default.**

The 2026-06 audit is the evidence. Five distinct semantics-breaking bugs were
found and fixed (see `memory/soundness-audit.md`), every one of them a flow
question answered structurally:

| Pass | Flow question it got wrong |
|------|----------------------------|
| const-fold | "does this expression have a side effect I'm discarding?" |
| decoder-lift | "does this function write state I can't see in the return?" |
| proxy-inline (drop) | "is this argument's effect dropped on substitution?" |
| proxy-inline (reorder) | "do two effects get swapped?" |
| assigned-once | "does the initializer dominate this read?" |

And the highest-value remaining target — sample_10's `var8 = [3,31,5]; … var8[1] …`
const-arrays — is **unreachable structurally** because the assignments are buried
inside `&&` short-circuits and span multiple statements:

```js
var2 << ((var2 | (((var2 << (((var2 | (var8 = [3,31,5], var8[0])) & 15) >= 8
        && var2 >> 2 < 11 && … < var8[2] …
```

Folding `var8[2]` → `5` requires proving the assignment dominates the read across
short-circuit operators and statement boundaries, plus proving the array never
escapes to be mutated. Those are **use-def chains, a dominator tree, and escape
analysis** — exactly what an SSA/CFG IR gives you for free.

**The thesis of this plan:** the next rung is not "more passes" but "an IR under
the passes." The payoff is primarily *soundness by construction* — flow
transforms stop being things we nervously unit-test and become things the IR
guarantees — and secondarily incremental reduction gains.

---

## 2. Scope — the tier model

Be honest about what an IR does and does not unlock. Reductions fall in tiers:

- **Tier A — structural (DONE).** String/proxy/decoder lifting, const-fold, DCE,
  dead-store, flatten-else, member-normalize, sequence-split, rename.
- **Tier B — intraprocedural flow (THIS PLAN).** Const propagation, copy
  propagation, the const-array fold, flow-sensitive DCE, scalar state-variable
  resolution within a function. **SSA is the clean, sound vehicle.**
- **Tier C — interprocedural.** sample_4's state constants (set in `fn1()`, read
  in `fn14()`), the `apply(this,[id,arguments])` dispatch forwarding. Needs a
  call graph + selective inlining or interprocedural dataflow *on top of* SSA.
  Out of scope for v1; SSA is the foundation for it.
- **Tier D — emulation / symbolic.** sample_9's VM (bytecode array mutated by
  `splice`), computed dispatch `obj[fn().Bt(...)]`, sample_7's encrypted keys.
  This is decompilation / partial execution, **not** dataflow. The existing
  sandbox (`decoder-lift`) is the right tool, gated on a purity proof the IR can
  help supply. Explicitly **not** what this IR is for.

This plan delivers **Tier B** and lays the foundation for Tier C. It does
nothing for Tier D and must not pretend to.

---

## 3. Non-goals

- Not a rewrite. The AST stays the source of truth; structural passes stay.
- Not interprocedural (Tier C) in v1.
- Not a VM lifter (Tier D).
- Not a performance project. Correctness and clarity over speed; samples are
  ≤2 MB and run offline.
- Not "leave the AST." The model is **AST → lower to IR → analyze/transform →
  rebuild AST**, the same shape every optimizing compiler uses.

---

## 4. Architecture

```
            ┌──────────────────────── existing ────────────────────────┐
  source ─► parse ─► AST ─► [structural passes in fixpoint driver] ─► codegen
            └───────────────────────────┬──────────────────────────────┘
                                        │  per function body
                                        ▼
                          ┌──── new IR layer (per function) ────┐
                          │  lower: AST stmts/exprs → CFG        │
                          │  build: dominator tree + DF          │
                          │  build: SSA (phi insert + rename)    │
                          │  analyze: consts / copies / escape   │
                          │  transform: fold / propagate / DCE   │
                          │  raise: SSA → AST (de-SSA + rebuild) │
                          └──────────────────┬──────────────────┘
                                             ▼
                            mutated AST function body, fed back
                            into the same fixpoint loop
```

Key design choices:

- **Function-at-a-time.** Each `Function` / `ArrowFunction` / `Program` top level
  is lowered, optimized, and raised independently. No whole-program IR. This
  keeps v1 tractable and matches Tier-B scope.
- **Reuse `oxc_semantic::Scoping`.** We already build it per pass; symbols and
  references are the anchor for SSA versioning. Do not invent a new symbol table.
- **The IR is an implementation detail of one pass** — call it `SsaOpt` — that
  implements the existing `Pass` trait. It lowers, optimizes, raises, and reports
  `changed`. It slots into `default_pipeline()` like any other pass and
  participates in the fixpoint. No driver changes required.
- **Escape hatch / partiality is fine.** Any construct the lowering doesn't model
  (e.g. `with`, `eval`, labelled continue into odd shapes, generators) ⇒ bail on
  that function and leave its AST untouched. Partial coverage is sound; the pass
  just does less.

### 4.1 `oxc_cfg` vs. a custom CFG

`oxc_semantic` has a `cfg` feature that pulls in `oxc_cfg` (petgraph-based, used
by the linter). **Phase 0 must evaluate it before committing:**

- *Pros:* maintained, integrates with `Scoping`, handles JS control-flow edge
  cases (try/finally, labels, short-circuit) we'd otherwise re-derive.
- *Cons:* its CFG is built for *lint analysis* (reachability, unused code), not
  for transformation + raising back to AST. It may not expose the
  instruction-level granularity (per-assignment basic-block contents) SSA needs,
  and we'd still build dominators + SSA on top.
- *Decision rule:* if `oxc_cfg` gives labelled basic blocks with statement/expr
  membership and a usable edge set, adopt it and build dominators+SSA on top.
  If it only models statement-level reachability, build a **custom minimal CFG**
  over a small lowered instruction list (below). Lean toward custom if in doubt —
  we control the raising step and the granularity.

---

## 5. The IR

A deliberately small, three-address-ish IR is enough for Tier B. Lower each
function body to a list of **basic blocks**, each a sequence of **instructions**:

```
Inst =
  | Assign  { dst: Place, val: RValue }     // x = rvalue
  | Eval    { val: RValue }                 // expression statement / effect
  | Phi     { dst: Place, args: [(Block, Value)] }   // SSA only
Terminator =
  | Goto Block
  | Branch { cond: Value, then: Block, else: Block }
  | Return Value?
  | Throw Value
  | Switch ...
  | Unreachable

Place  = a symbol (SymbolId) + optional version (SSA)   // only *local, non-escaping* names get versioned
RValue = Use Value | Bin op Value Value | Un op Value | Member base key
       | Call callee [args] | New ... | Array [elems] | Object [...] | Lit
Value  = a Place use, or a literal
```

**Critical modelling rule (lesson from the audit):** an `RValue` that may have a
side effect (`Call`, `New`, member-with-getter, `Assign` nested, `await`/`yield`)
is **never** silently dropped or reordered. Effects are first-class `Eval`/`Assign`
instructions in block order. The IR makes "is this droppable / reorderable"
structural again — but *correctly* this time, because order and multiplicity are
explicit.

**Versioning scope.** Only **local, non-escaping, non-captured** symbols are put
into SSA form. A symbol is excluded (kept as an opaque memory cell, never
versioned, never folded) if it is:
- captured by a nested function (closure), or
- ever the base of a member write / passed to a call / `new` (escape → external
  mutation), or
- `arguments` / `eval`-touched.

This escape/capture set is computed once up front (reuse the analysis sketched in
`dead_object_prop.rs` and the free-write detection added to `decoder_lift.rs`).
Excluded symbols simply don't get optimized — sound, just less complete.

---

## 6. Phased implementation

Each phase is independently testable and leaves the tree green. Ship behind the
equivalence harness at every step.

### Phase 0 — Spike (1 IR, 1 transform, 1 sample) — **do this first**
Goal: de-risk the whole plan on a real case before investing.
- Pick **one function** from sample_10 containing the `var8 = […]; var8[k]` shape.
- Build the absolute minimum: lower that function to a CFG, compute a dominator
  tree, build use-def chains for **one** non-escaping local, and fold its
  `V[const]` reads where the def dominates the use.
- Evaluate `oxc_cfg` here (§4.1).
- **Acceptance:** the function's `var8[…]` folds to literals; output re-parses;
  `tests/equivalence.rs` gets a case mirroring the short-circuit shape and passes;
  `tests/samples.rs` still green (idempotent + safe).
- If the spike is disproportionately hard or can't be made sound, **stop here** —
  that is itself the answer, and it cost one function's worth of work.

### Phase 1 — CFG construction (general)
- Lower arbitrary function bodies to basic blocks + terminators.
- Handle: sequences, if/else, for/while/do, switch, break/continue (incl.
  labels), try/catch/finally, logical `&&`/`||`/`??` and `?:` (these create
  branches — exactly where the structural passes went blind), short-circuit eval.
- Bail (leave AST untouched) on: `with`, generators/async for v1, anything
  unmodelled.
- **Acceptance:** round-trip lower→raise with *no* optimization is a semantic
  identity on all samples (re-parse + equivalence battery + idempotence).

### Phase 2 — Dominators + dominance frontier
- Dominator tree (Lengauer–Tarjan, or simple iterative for clarity — functions
  are small). Dominance frontier for phi placement.
- **Acceptance:** unit tests on hand-built CFGs (diamonds, loops, nested).

### Phase 3 — SSA construction
- Insert phis at dominance frontiers for each versioned (non-escaping) symbol;
  rename to versions.
- **Acceptance:** SSA validity checker (every use reaches exactly one def/phi);
  de-SSA round-trips to equivalent AST.

### Phase 4 — Analyses (sound by construction)
- **Constant lattice** over SSA values (incl. literal arrays/objects as immutable
  aggregates, since non-escaping). `V[const]` / `V.key` resolve through a known
  aggregate def.
- **Copy propagation** via use-def (`x = y` with `y` a stable value).
- **Liveness** for DCE.
- **Acceptance:** analysis results asserted on fixtures; no transform yet.

### Phase 5 — Transforms
- Const/copy propagation; const-array & const-object index fold; flow-sensitive
  dead-assignment elimination. All gated by the IR's dominance + escape facts, so
  no hand-proven flow conditions.
- **Acceptance:** the full `tests/equivalence.rs` battery (extended with
  flow-sensitive adversarial cases: short-circuit, reassignment-between, escape,
  capture, branch-merge, loop-carried) passes; report shows movement on
  sample_10; idempotence holds.

### Phase 6 — Raise back to AST
- De-SSA (insert copies at phis where needed; coalesce versions back to the
  original symbol) and rebuild AST statements/expressions, preserving original
  names so `rename` runs last as today.
- **Acceptance:** output re-parses, is idempotent, equivalence-clean.

### Phase 7 — Integration
- Register `SsaOpt` in `default_pipeline()` (position: after `decoder-lift`,
  before `dce`, so it sees lifted literals and feeds DCE). Confirm fixpoint
  convergence and no regression across all 11 samples.

---

## 7. Soundness strategy (non-negotiable — this is the whole point)

The hard invariant stays: **every transform is semantics-preserving; output
re-parses; second run == first.** Enforced by `tests/samples.rs`. The differential
net is `tests/equivalence.rs` (obfuscated vs deobfuscated snippet run in QuickJS,
`typeof`+value compared).

Discipline carried from the audit:
1. **Effects are explicit and never dropped/reordered.** The IR represents every
   call/assignment/`new`/await/yield as an ordered instruction. A transform that
   removes an instruction must prove its result is dead *and* it is pure.
2. **Only non-escaping, non-captured locals are versioned/folded.** Everything
   else is an opaque cell. Default to *keep* / *don't fold*.
3. **Aggregates fold only to immutable elements** (literals). An identifier or
   call element is not re-materialised at a use site (its value could differ).
4. **Add the adversarial case before the feature.** Every Phase-5 transform ships
   with equivalence cases for its failure modes (the §6 list).
5. **Lower→raise identity (Phase 1) is tested independently** of any optimization,
   so a raising bug can't masquerade as a transform bug.
6. **Partial is fine.** Bailing on a function is always sound. Never guess.

---

## 8. Worked target — sample_10 const-arrays (the Phase-0 / Phase-5 proof)

Input shape:
```js
function (…, var8, …) {
  if ((var8 = [3,31,5], cond) < var8[1] && … < var8[2]) { … var8[0] … }
}
```
With the IR:
- `var8` is a non-escaping local param (no member write, never passed/captured).
  → eligible for versioning.
- Lowering makes the `&&` a branch; `var8 = [3,31,5]` is an `Assign` in the entry
  block, dominating the later operands' blocks and the `if` body.
- SSA: one version `var8₁ = [3,31,5]`; every `var8[k]` use references `var8₁`.
- Const-aggregate analysis: `var8₁[0]=3`, `[1]=13`, `[2]="apply"`.
- Transform: replace the uses with the literals. `var8` becomes dead → DCE drops
  the assignment.

Expected: the arithmetic collapses (those `var8[k]` were the only opaque parts of
several predicates), readability of sample_10 jumps. **This is the acceptance
demo for the whole IR.**

---

## 9. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| SSA bug introduces unsoundness (the audit's whole theme) | Lower→raise identity test (Phase 1) + full equivalence battery + samples idempotence; ship phase-by-phase. |
| `oxc_cfg` doesn't fit transformation use | Phase-0 evaluation gate; fall back to custom minimal CFG. |
| JS control-flow edge cases (finally, labels, short-circuit) modelled wrong | Bail-on-unmodelled; extensive Phase-1 fixtures; identity test catches divergence. |
| Scope creep into Tier C/D | Hard non-goals (§3); function-at-a-time; escape set excludes everything cross-function. |
| Big investment, incremental reduction payoff | Frame ROI as *soundness/maintainability* first; Phase-0 spike proves value before commitment; stop after spike if ROI isn't there. |

---

## 10. Milestones / acceptance criteria

- **M0 (spike):** one sample_10 function's const-arrays fold soundly via a throw-
  away CFG; go/no-go decision on the whole plan.
- **M1:** general CFG + lower→raise identity, green on all samples.
- **M2:** dominators + SSA + validity checker.
- **M3:** const/copy/escape analyses with fixtures.
- **M4:** transforms live; sample_10 measurably reduced; equivalence battery
  (incl. flow-sensitive adversarial cases) green; idempotent.
- **M5:** integrated into the fixpoint pipeline; no regression on any sample;
  `memory/` + this doc updated with results.

Definition of done for v1: `SsaOpt` is a registered pass, Tier-B transforms are
sound-by-construction, sample_10's const-array noise is gone, and the soundness
nets (samples + equivalence) are green. Tier C/D remain explicitly open.

---

## 11. Open questions for kickoff

1. `oxc_cfg` adopt vs. custom — resolve in Phase 0.
2. De-SSA strategy: full coalescing vs. naive copies + let later DCE clean up
   (start naive, it's simpler and sound).
3. Dominator algorithm: iterative (clarity) vs. Lengauer–Tarjan (speed) — start
   iterative; functions are tiny.
4. Pipeline position of `SsaOpt` and interaction with `decoder-lift`'s
   sandbox-materialised constants (likely after lift, before DCE).
</content>
