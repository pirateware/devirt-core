# Plan: the Dispatch & Decoder Resolution tier (Tier C/D)

Status: **ROI core (Phases 0–2 / M1–M2) DELIVERED** 2026-06 on branch
`feat/scoped-decoder-lift`. Phases 3–4 assessed and **deferred** (data-driven, see
§11). Phase 5 remains groundwork-only by design.

## 11. Delivered & measured (2026-06)

**Phases 0–2 — scoped decoder lifting — shipped, sound, green.** `decoder-lift`
now classifies pure decoder functions at *every* scope depth (not just
`collect_top_level`), seeds classification with proven pure-data table names, and
emits all pure functions (any depth) into the preamble via a single emitter.
Soundness rides a new symbol-precise escape query
`ir::mutable_aggregate_tables` (reusing `src/ir/analysis.rs`) that flags any
aggregate reassigned / member-written / `.push`d / aliased anywhere; the exclusion
is threaded into `scan_invariants` so the groundedness fixpoint also drops
transitive dependents (identity aliases), keeping the preamble dependency-closed.

Result on the acceptance demo (§7): **sample_11 `fn55(...)` dispatch sites
3178 → 12** (the 12 survivors decode to null/object and are correctly left); the
DataDome DOM code (`.createElement`/`.appendChild`/`.documentElement`) is revealed.
No regression on any sample (idempotent + re-parse) or the equivalence net;
sample_4 base-N materialisation unaffected. New adversarial equivalence cases
(member-mutated / pushed / aliased-mutated / reassigned scoped tables) + positive
behavioural tests cover the gate.

**Phase 4 — apply/call forwarding — assessed NO-GO on this corpus.** Recon of the
deobfuscated sample_1 (3124 forwarding sites) and sample_10 (1218) shows the
forwarding *target* is essentially never a plain identifier: it is threaded
through closure-memoised dispatchers (`fn102().Bt.apply(null, […])`) whose backing
object is populated elsewhere — i.e. interprocedural alias analysis, the §3
non-goal. The trivially-sound subset (`f.apply(null, [a,b]) → f(a,b)` with `f` a
plain identifier) is ~0 sites across both files. A sound, general collapse is not
available; the object-method `.call`/`.apply` *proxy* form is already handled by
`proxy-inline`. Deferred rather than ship a high-risk interprocedural transform.

**Phase 3 — interprocedural SCCP — deferred.** A from-scratch call-graph + SCCP
pass targeting one already-86%-reduced sample (sample_4) is high effort / low
corpus ROI and carries real interprocedural-unsoundness risk against the green
soundness nets. Left open, as the plan intends for Tier C/D remainders.

---

Status (original): proposal. Target start: after the SSA/CFG dataflow layer
(`src/ir/`, delivered 2026-06). Owner: TBD.

This is the successor to the now-removed SSA plan. That work delivered Tier B
(sound intraprocedural flow). This plan attacks the obfuscation that actually
dominates the remaining corpus — **computed-member dispatch and decoder calls
buried inside function scopes** — and is the single highest-leverage body of work
left.

---

## 1. Why — what the corpus is actually hiding now

After Tiers A+B, a scan of the deobfuscated corpus for the dominant remaining
patterns (no `while(true)` dispatchers exist — control-flow flattening is *not*
the obfuscation here):

| sample | computed `obj[expr](…)` | `.apply/.call` forwarding | kept% | regime |
|--------|------------------------:|--------------------------:|------:|--------|
| sample_1  | 3461 | 3124 | 74% | **runtime-keyed** dispatch + apply forwarding |
| sample_11 | 2100 |   21 | 64% | **constant-keyed** decoder dispatch |
| sample_7  | 1161 |  125 | 86% | **constant-keyed** base64 "encrypted" keys |
| sample_10 |  155 | 1218 | 80% | apply forwarding + ternary noise |
| sample_4  |   10 |  394 | 86% | interprocedural state + apply forwarding |

Two distinct regimes fall out, and they need completely different machinery:

- **Constant-keyed** (`obj[fn55(311)]`, `obj[fn55(1)][fn55(2)](fn55(3))`): the
  key is a *literal-argument call to a pure decoder*. Statically resolvable. This
  is the bulk of sample_11 (~2100 sites) and sample_7 (~1160). **This tier's
  primary target.**
- **Runtime-keyed** (`obj[fn31(var379)]`, `var891.call(this, x, [fn112()])`): the
  key/args depend on runtime values threaded through calls. Needs interprocedural
  value propagation or, in the limit, symbolic execution. The harder rump.

### 1.1 The wall: the decoder library is not top-level

`decoder-lift` already has everything needed to resolve the constant-keyed case —
a QuickJS sandbox, a purity classifier, a determinism guard. It resolves nothing
in sample_11. The reason is purely **scope**:

```js
var DataDomeJsTag = (() => {
  …
  var597 = [ /* hundreds of base64 strings */ ],   // ~6 scopes deep
  …
  function fn55(var1144) {                          // the decoder, ~5 scopes deep
    var1144 = var597[var1144];
    if ("string" != typeof var1144) return var1144;
    /* custom-alphabet base64 decode against
       "H1DAxCvrj7IaPRL8GSJZKX3f62e9d0VTilFEOWgUB=/t+QmMwuskNnhpb4oyq5Yzc" */
    return decoded;
  }
}
```

`fn55` is **pure** — deterministic, side-effect-free, a pure function of the
local table `var597` and `String`. It is exactly what the sandbox lifts. But
`decoder-lift::collect_top_level` only walks `program.body` plus *transparent
wrappers* (IIFEs / `try` / bare blocks) and **never descends into function
bodies** (by design: "those are real logic"). The whole library — `var597`,
`fn55`, the arithmetic proxies `fn56/57/58` — lives inside nested function
scopes, so the lifter sees nothing.

**The thesis:** lift decoders *at the scope where they live*, not only at the
program top level. The hard part is no longer "can we evaluate it" (the sandbox
can) but "can we prove it is **sound** to evaluate" — and that proof is exactly
what the Tier-B escape analysis (`src/ir/analysis.rs`) already computes.

---

## 2. Scope — the phase/tier model

- **Phase 1 — Scoped decoder lifting (THE win).** Run the decoder-lift
  partial-evaluation at *every* function scope, gated by a soundness proof that
  the decoder's table is a single-assignment, non-escaping, non-rotated-outside-
  init local. Resolve constant-argument decoder calls to literals. Unlocks
  sample_11 (~2100 sites) and sample_7 (~1160).
- **Phase 2 — Member-access closure.** After lifting, `obj["decodedName"]` →
  `obj.decodedName` via the existing `member-normalize`, and the operator-proxy
  inliner cleans the arithmetic proxies (`fn56/57/58`). Mostly exists; this phase
  is about making the fixpoint converge on the new literals.
- **Phase 3 — Interprocedural constant/state propagation (Tier C).** A call graph
  + selective inlining or interprocedural SCCP for state constants set in `fn1()`
  and read in `fn14()` (sample_4), which also turns some "runtime" keys into
  provable constants that Phase 1 can then resolve.
- **Phase 4 — apply/call dispatch forwarding.** Recognise and collapse the
  `f.apply(this, [id, arguments])` / `f.call(this, x, [args])` dispatcher
  wrappers (sample_1: 3124, sample_10: 1218, sample_4: 394).
- **Phase 5 — Symbolic / VM (boundary, mostly future).** Runtime-keyed dispatch
  `obj[fn(runtimeVar)]` and sample_9's bytecode VM. This is partial execution /
  decompilation, not dataflow. The plan lays groundwork (the call graph, the
  sandbox) but does **not** commit to a full VM lifter.

Phases 1–2 are the ROI core and should ship first. 3–4 are real but smaller.
5 is explicitly open.

---

## 3. Non-goals

- Not a VM lifter (Phase 5 is groundwork only).
- Not resolving genuinely runtime-keyed dispatch where the key is not a provable
  constant (sample_1's `obj[fn31(var379)]`). Leave it; never guess.
- Not lifting any decoder that observes or mutates the host (DOM, `navigator`,
  timers, network) — those are not pure and not reproducible in the sandbox.
- Not a rewrite of `decoder-lift`. Extend it; keep the top-level path working.
- Not performance work. Samples are ≤2 MB, offline.

---

## 4. Architecture

```
            existing decoder-lift (top-level only)
  program ─► collect_top_level ─► classify ─► build_preamble ─► sandbox ─► lift
                                                                              │
        ┌──────────────── THIS PLAN: generalise to per-scope ────────────────┘
        ▼
  for each function scope S (incl. the program top level):
     library(S) = pure decoders + tables defined in S, where the
                  SOUNDNESS GATE holds (see §5), proven by reusing
                  src/ir/analysis.rs escape facts
     preamble(S) = serialize library(S) + run S's rotation IIFEs
     setup sandbox once per S
     lift every  fn(CONST)  / library[CONST]  call in S's subtree
```

Key design choices:

- **Reuse the sandbox + classifier + determinism guard verbatim.** The only new
  thing is *where* we look and *how we prove it is safe*.
- **The soundness gate is the SSA escape analysis.** `src/ir/analysis.rs` already
  decides whether a local symbol is single-assignment, non-escaping, non-mutated,
  non-captured-as-writable. Promote that module from `pub(crate) mod ir` internals
  to a small reusable query: `is_pure_local_table(scope, symbol) -> bool`. The
  table `var597` qualifies iff it is assigned once with a pure-data literal and
  never reassigned/mutated/aliased anywhere in S's subtree.
- **Scope = closure scope, not lexical block.** `fn55` is *called* deep inside
  `setTimeout` callbacks that close over it. A lift of `fn55(CONST)` is valid
  anywhere in S's subtree because `fn55` and `var597` are captured and (proven)
  stable. So Phase 1 lifts across nested closures within S, not just S's own body.
- **Per-scope, cheapest-first.** Try the program top level (existing path), then
  descend into each function scope that *contains a pure-data table and a function
  that indexes it*. Most scopes have no library and are skipped in O(1).

### 4.1 Why this is now sound when it wasn't before

The 2026-06 captured-table bug (`var s=[1,2]; function get(){return s[0]} s=[3,4];
get()` lifting to a stale `1`) is the cautionary tale: baking a *reassigned* table
is unsound, and the determinism guard can't catch it (a stale value agrees with
itself). The fix there was `scan_unstable` — a name with >1 write is never baked.
Scoped lifting widens the blast radius of that hazard, so it **must** ride on the
stronger, symbol-precise escape proof from `src/ir/analysis.rs` (which is
scope-aware and alias-aware), not the name-based heuristic. Soundness is the whole
game; see §5.

---

## 5. Soundness strategy (non-negotiable)

The hard invariant stays: **every transform is semantics-preserving; output
re-parses; second run == first.** Enforced by `tests/samples.rs`; the differential
net is `tests/equivalence.rs` (obfuscated vs deobfuscated run in QuickJS).

A decoder call `fn(CONST)` in scope S is liftable **only if all hold**:

1. **Determinism.** `fn`'s free names are all pure globals (the `GLOBALS`
   whitelist) or pure locals of S. No `Date`/`Math.random` survives the existing
   double-eval guard.
2. **No host observation/mutation.** `fn` reads/writes nothing outside its own
   locals + the proven-pure table. Reuse `FreeVars::writes_free` + the
   mutator-method list; extend it to reject reads of host globals not on the
   whitelist.
3. **Table is a pure local invariant** (the new gate, from `src/ir/analysis.rs`):
   the table symbol is assigned exactly once with pure data, is never reassigned,
   never member-written/mutated, never passed where it could be mutated, and is
   not captured by a writer — *anywhere in S's subtree*. A table that is *rotated*
   qualifies only if the rotation is an init-time IIFE we reproduce in the
   preamble (existing `IifeCollector`, scoped).
4. **Constant argument.** Only `fn(<literal>)` (or `fn(<provably-constant>)` once
   Phase 3 lands) is lifted. Runtime-keyed calls are left untouched.
5. **Type fidelity.** A numeric/boolean result splices as that literal kind, never
   as a string (the existing `EvalValue` discipline).

Discipline carried from the audit (`memory/soundness-audit.md`):
- Effects are never dropped. A non-pure "decoder" simply fails the gate and is
  left in place. Partial coverage is always sound.
- Add the adversarial equivalence case *before* the feature: a scoped decoder over
  a table that is reassigned / rotated / captured-and-mutated / read after a host
  side effect must each be left unlifted, proven by an equivalence case.

---

## 6. Phased implementation

Each phase is independently testable and leaves the tree green behind the
equivalence harness.

### Phase 0 — Spike on sample_11 `fn55` — **do this first**
- Hand-locate `fn55` + `var597` and their scope S. Build the absolute minimum:
  serialize `var597` + `fn55` as a scoped preamble, sandbox-evaluate one
  `fn55(311)`, confirm it returns the expected property name.
- Wire the soundness gate (§5.3) using `src/ir/analysis.rs` to prove `var597` is a
  pure local invariant of S.
- **Acceptance:** a handful of `obj[fn55(CONST)]` in sample_11 become
  `obj["name"]`; output re-parses; `tests/equivalence.rs` gets a scoped-decoder
  case that passes; `tests/samples.rs` stays green. If the gate can't be made
  sound cheaply, **stop and reassess** — that is itself the answer.

### Phase 1 — Generalised scoped lifting
- Refactor `collect_top_level` → `collect_scopes`: yield (scope, statement-set)
  for the program top level and each function body, descending through transparent
  wrappers within each.
- Run classify + preamble + lift per scope; cache sandboxes per scope.
- Gate every table with `is_pure_local_table` (§5.3).
- **Acceptance:** sample_11 + sample_7 measurably reduced (target sample_11
  64% → ~40%, bracket count → near zero); all samples idempotent + equivalence
  green; no regression on the top-level decoders already lifted.

### Phase 2 — Member-access + proxy closure
- Confirm `member-normalize` collapses the freshly-lifted `obj["x"]` → `obj.x` and
  that operator proxies (`fn56/57/58`) inline, to a fixpoint.
- **Acceptance:** sample_11 opaque-name ratio and bracket count drop sharply;
  idempotent.

### Phase 3 — Interprocedural constant propagation (Tier C)
- Build a conservative call graph over top-level + scoped functions. Propagate
  constants across single-call-site / pure-argument boundaries (selective inlining
  or interprocedural SCCP on top of the existing SSA values).
- **Acceptance:** sample_4's state constants resolve; some sample_1 keys that are
  constants-threaded-through-calls become liftable by Phase 1.

### Phase 4 — apply/call dispatch forwarding
- Recognise `f.apply(this, [id, arguments])` / `f.call(this, recv, [args])`
  dispatcher wrappers and forward them where the receiver/args are statically
  determinable.
- **Acceptance:** sample_1 / sample_10 apply-forwarding counts drop; equivalence
  preserved (receiver binding must be respected — the existing `proxy-inline`
  receiver-detach rules apply).

### Phase 5 — Symbolic / VM (groundwork only)
- Document the runtime-keyed dispatch and sample_9 VM as out of scope; ensure the
  call graph + sandbox built above are reusable foundations. No commitment.

---

## 7. Worked target — sample_11 `fn55` (the Phase-0/1 proof)

Input shape (hundreds of sites):
```js
var547 = var598[fn55(1)][fn55(2)](fn55(3));
var548 = var598[fn55(1)][fn55(4)](fn55(5));
```
With scoped lifting:
- `var597` proven a pure local invariant of S (single pure-data assignment, no
  reassignment/mutation/alias in S's subtree).
- `fn55` proven pure (free names: `var597`, `String`).
- Each `fn55(k)` → its decoded string literal, e.g. `"appendChild"`,
  `"createElement"`, …
- `member-normalize`: `var598["createElement"]` → `var598.createElement`.

Expected:
```js
var547 = var598.documentElement.appendChild(var598.createElement("script"));
```
sample_11 collapses from a wall of `var598[fn55(n)][fn55(m)](fn55(k))` into
readable DOM code. **This is the acceptance demo for the whole tier.**

---

## 8. Risks & mitigations

| Risk | Mitigation |
|------|------------|
| Scoped lift bakes a stale/mutated table (the captured-table bug, widened) | Gate on the symbol-precise SSA escape proof (§5.3), not the name heuristic; adversarial equivalence cases for reassign/rotate/capture-mutate before shipping. |
| A "decoder" observes host state (DOM/navigator) → non-reproducible | `GLOBALS` whitelist + reject host reads + double-eval determinism guard; such a function fails the gate and is left in place. |
| Performance: sandbox per scope on a 2 MB file | Skip scopes with no table-and-indexer in O(1); cache sandboxes; lift is memoised by source text already. |
| Phase 3/4 scope-creep into a full VM | Hard boundary at §3; Phases 1–2 deliver the ROI without them. |
| Interprocedural unsoundness (Tier C) | Start with single-call-site / pure-argument propagation only; everything else conservative. |

---

## 9. Milestones / acceptance criteria

- **M0 (spike):** one sample_11 `fn55(CONST)` resolves soundly via a scoped
  preamble gated by the escape proof; go/no-go on the tier.
- **M1:** generalised scoped lifting; sample_11 + sample_7 measurably reduced;
  samples idempotent; equivalence green (incl. scoped-decoder adversarial cases).
- **M2:** member-access + proxy closure; sample_11 brackets/opaque ratio collapse.
- **M3:** interprocedural constant propagation; sample_4 state constants resolve.
- **M4:** apply/call forwarding; sample_1/10 forwarding reduced, equivalence-clean.
- **M5:** Phase-5 boundary documented; `memory/` + this doc updated with results.

Definition of done for the ROI core (M1–M2): scoped decoder lifting is sound by
construction, sample_11's constant-keyed dispatch is resolved to named member
access, and the soundness nets (samples + equivalence) are green. Tier C/D
remainders (runtime-keyed dispatch, VM) stay explicitly open.

---

## 10. Open questions for kickoff

1. Expose `src/ir/analysis.rs` escape facts as a reusable `is_pure_local_table`
   query vs. re-deriving a write/escape scan inside `decoder-lift`. (Prefer reuse
   — one source of truth for "is this local provably stable".)
2. Per-scope sandbox lifecycle: one sandbox per scope (isolation) vs. one shared
   context with scope-prefixed preambles (speed). Start per-scope for clarity.
3. Rotation discovery within a scope: reuse `IifeCollector` scoped, or require the
   table be un-rotated for v1 and add rotation in M2.
4. Phase 3 call-graph granularity: top-level functions only, or all scoped
   functions? Start top-level + the enclosing scope's functions.
