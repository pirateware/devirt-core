# Engine next wave — parallel improvement tasks

A batch of engine improvements **engineered for parallel execution**: each task
*owns a disjoint set of files* so a fleet of worktree agents can run them at once
and merge with at most trivial `mod.rs`/test-append conflicts — the same
partition-by-file-ownership discipline that let the A/B/C waves merge cleanly
(`plans/metrics-abc.md`).

Everything here inherits the **P0 discipline** unchanged (`plans/P0-execution-plan.md`):
sound-by-construction or **decline**; test-first; every value-changing transform ships
a should-fire **and** an adversarial must-NOT-fire sibling, gated by the QuickJS
differential equivalence harness; one milestone = one green commit; **no sample may
regress** (byte growth or `converged=false` → revert, don't patch); `cargo test` +
`clippy --all-targets` + `fmt` clean before every commit.

Baseline at plan time: **268 tests green**; `src/ir/dispatch.rs` (A4 substrate) landed
additive/unwired; four documented walls (s4 mutating-`valueOf`, s1 nested-IIFE,
s11 mutable-table, s7 host-coupled).

---

## Phase 0 — determinism (FOUNDATION, run solo first)

### T0 · F1 — Pipeline determinism  ★ unblocks everything parity-gated
**Impact: high (makes byte-parity a usable gate). Effort: low. Risk: low.**
**OWNS:** every pass/IR module that iterates a `HashMap`/`HashSet` *to emit or order
output* — the surface is ~20 files (`src/ir/{analysis,cff,cfg,consts,inline,mod,
ordering,relooper,ssa}.rs`, `src/passes/{assigned_once,dead_object_prop,dead_store,
decoder_lift,demultiplex,global_const_inline,math_fingerprint,proxy_inline,rename,
unflatten,unflatten_param,unflatten_string_order}.rs`).

Because it touches nearly everything, **run T0 ALONE before the Phase-1 fan-out** (it
would conflict with every other task otherwise). Replace iteration-order-dependent
emission with a stable sort by a deterministic key (`SymbolId`, source `Span`, then
name). Internal lookups that don't affect output may stay `HashMap`; only *emission/
ordering* must be deterministic.
- **Gate:** s3 and s11 must become byte-stable across 5 consecutive runs (today they
  jitter run-to-run). Add a `tests/determinism.rs` that deobfuscates a fixture twice
  and asserts byte-identical output.
- **Deliverable:** byte-stable corpus; parity-gating re-enabled for T9/T11.

---

## Phase 1 — the parallel fan-out (≈10 tasks, mutually file-disjoint)

Each branches off post-T0 `main`. Listed with **OWNS** (the disjointness contract).

### T1 · F5 — Sandbox host stubs
**Samples: 7, 11, 8. Impact: high (unblocks host-relative decoders + T7). Effort: high. Risk: med.**
**OWNS:** `src/sandbox.rs` (+ optional new `src/sandbox/host.rs`).
Deterministic, frozen-by-construction stubs for the host surface anti-bot code reads:
`navigator`, `document`, `screen`, `location`, timing (`Date`/`performance.now` →
fixed), `crypto.getRandomValues` → seeded-deterministic. Pure-relative-to-host decoders
(the s7 `var9.TextDecoder`/`var9.Buffer` tail) then evaluate.
- **Gate:** every stub is a constant function of nothing (no wall-clock, no entropy) —
  cf. the Date-freeze latent bug. **Adversarial:** a decoder that *writes* host state, or
  branches on a genuinely non-deterministic source, must still DECLINE.

### T2 · E4 — Rename by role / type
**Samples: all. Impact: medium (last-mile readability). Effort: medium. Risk: low.**
**OWNS:** `src/passes/rename.rs`.
Extend `RenameByRole`: infer names from *usage* — loop counters (`for`-init/update),
accumulators (`+=` in a loop), DOM handlers (`addEventListener`/`.on*`), decoders
(call sites that feed member reads), boolean predicates, string-key hints from object
literals. Stable, collision-free, idempotent.
- **Gate:** renames never change semantics or capture; **adversarial:** a name that
  would shadow/collide in scope → keep the original. Must stay idempotent (re-run = no-op).

### T3 · E2 — MBA / opaque-predicate simplification
**Samples: 11, 10. Impact: medium. Effort: medium. Risk: low.** (issue 10)
**OWNS:** `src/passes/math_fingerprint.rs`.
Fold mixed boolean-arithmetic identities and constant opaque predicates: `x^x→0`,
`x&~x→0`, `x|~x→-1`, `(x|0)===x` for int x, `(p&~p)`, `a+~a→-1`, De-Morgan pairs. Constant
guards then collapse via the existing `if(true)`/flatten cascade.
- **Gate:** identities proven over the full int32 battery in the sandbox before rewrite
  (reuse the fingerprint harness). **Adversarial:** a near-identity that fails on one
  input (e.g. on floats/NaN/`-0`) → decline.

### T4 · E1 — Compiler-helper fingerprint + async/generator lift
**Samples: 3, 8. Impact: high. Effort: high. Risk: high.** (issues 07, 08)
**OWNS:** new `src/passes/helper_lift.rs` (self-contained; do its own renaming — do NOT
touch `rename.rs`, that's T2). Register in `mod.rs`.
Fingerprint & rename tslib/babel/swc helpers (`__awaiter`, `__generator`, `__spread`,
`__extends`, `__assign`, …) to readable names by structural signature. Then lift
`__generator` state machines back to `async`/`await` + `yield` (the inverse down-level
transform — itself a data-driven CFF recovery; can reuse the A2 relooper for the body).
- **Gate:** signature match is exact (shape + helper-marker), not name-based.
  **Adversarial:** a user function coincidentally named `__generator` with a different
  body → no rewrite. Land the *fingerprint+rename* milestone green first; the async lift
  is a second commit and may decline complex generators.

### T5 · E3 — Dead-machinery DCE
**Samples: 5, 6, 8. Impact: medium. Effort: medium. Risk: low.** (issue 05)
**OWNS:** `src/passes/dce.rs` + `src/passes/dead_store.rs`.
After the A/B/C lifts, aggressively remove spent rotation loops, shuffle IIFEs, and
dead string tables that current DCE leaves because of residual now-dead references
(decrement the refcount through dead-but-not-yet-removed readers in a fixpoint).
- **Gate:** removal only when provably read-never **and** side-effect-free; **adversarial:**
  a table still read on one path, or a rotation with an observable effect → keep.

### T6 · F4 — Performance & scale
**Sample: 10 (2.1 MB, 16k vars). Impact: medium. Effort: medium. Risk: low.**
**OWNS:** `src/driver.rs` + `src/util.rs` + `src/lib.rs` (the fixpoint loop) + a new
reusable span/child index utility.
Profile the fixpoint on the giant samples; add a reusable O(1) span→node index to kill
O(n²) span scans; cache per-round scoping where safe; short-circuit passes that reported
no change last round until an upstream pass dirties their region.
- **Gate:** output byte-identical on all samples (pure speedup); add a timing assertion
  or `report`-time note. **Adversarial:** none (perf-only) — but a single behavioral
  byte change fails the task.

### T7 · F3 — Whole-sample differential equivalence harness
**Impact: high (catches sample-scale unsoundness). Effort: high. Risk: med.**
**OWNS:** new `tests/whole_sample.rs` (+ a tiny `lib.rs` export hook if needed).
Execute whole samples (or instrumented entry points) original-vs-deob in the sandbox
with stubbed host, comparing observable traces — the snippet battery can't catch
sample-scale unsoundness. Self-contained minimal host stub until **T1/F5** lands, then
switch to it.
- **Gate:** harness must pass on the current corpus (proving today's output is sound at
  whole-sample scale); a real divergence is a P0-blocking finding, not a test to relax.

### T8 · D1 — Bytecode-VM recognition (read-only)
**Samples: 8, 9, 10. Impact: very high (only path for these). Effort: high. Risk: low (read-only).**
**OWNS:** new `src/passes/devirt_detect.rs` (pure analysis + diagnostics; **no rewrite**).
The contained first step of Workstream D: detect the fetch-decode-execute loop
(`while(true){ op = code[pc++]; switch(op){…} }`), the bytecode array, and the
pc/stack/register roles. Emit a structured report (which samples have a VM, array
location, opcode count). No AST mutation — recognition only, so it is zero-risk and
unblocks the D2/D3/D4 mega-project later.
- **Gate:** detection precision tested on s9 (known VM) vs s5 (known not-VM → no false
  positive). **Adversarial:** an ordinary `switch` loop must not be flagged as a VM.

### T9 · A5 — `if/else`-ladder dispatchers
**Samples: long tail. Impact: medium. Effort: medium. Risk: low.**
**OWNS:** new `src/passes/unflatten_ladder.rs`. Register in `mod.rs`.
Recover `goto`-via-`if/else if` ladder CFF (a dispatcher with no `switch` at all):
`while(state){ if(state===A){…;state=B} else if(state===B){…} … }`. Reuse the A2
relooper / the A4 `DispatchModel` integer model as the structuring target.
- **Gate:** same private-dispatch-var soundness gates as `unflatten`; **adversarial:** a
  real `if/else if` chain over a non-state variable → decline. Fires on no current sample
  (sound long-tail widening) — verify zero corpus perturbation.

### T10 · B2 — Deterministic table evaluator (issue 11)
**Sample: 11. Impact: high. Effort: high. Risk: med.** (issue 11)
**OWNS:** `src/ir/consts.rs` + `src/ir/analysis.rs`.
Resolve `var401[i][j]`-style labels where the table is built by *deterministic* member
writes (s11's `arr14[7]` filled in `setTimeout` closures): replay the write sequence in
the sandbox to materialize the immutable snapshot, **with the issue-09 bloat cap** (never
materialize a table whose expansion exceeds a size budget — exclude s9's 131 KB bytecode
array). Decline on any genuinely-mutable / order-dependent / host-coupled table.
- **Gate:** replay must be confluent (write order can't change the snapshot) or decline;
  **adversarial:** a table mutated after first read, or whose fill depends on host state → decline.

### T11 · Latent soundness fix — split-then-mutate fold
**Impact: high (correctness). Effort: low. Risk: low.**
**OWNS:** `src/ir/inline.rs` (the `InlineSingleUse`/fold path).
Fix the bug the A1 agent found: for `var a = "0|1".split("|"); a.push("2"); … a[i++]`,
the fold inlines `a`'s init into the `a.push(...)` call site and drops the `a` binding,
leaving `a[i++]` referencing an undefined `a`. **Add the guard:** never inline a
single-"use" value whose binding is also *mutated in place* (member-write/`.push`/etc.)
after the use site — those are not single-use. Add the adversarial equivalence test that
currently mis-folds.

---

## Phase 2 — parity-gated wiring (after T0)

### T12 · A4-wiring — route a dialect through the substrate
**Impact: architectural. Effort: medium. Risk: med (parity-gated).**
**OWNS:** `src/ir/dispatch.rs` + `src/passes/unflatten.rs`.
Wire the integer-state `unflatten` recognizer through `dispatch::interpret` →
`relooper::structure_general` (the A4 substrate already exists, unwired). Supply a
`relooper::CaseSource` keyed by `StateNumbering::state_of` to re-emit original case
bodies verbatim. **MUST gate on byte-parity** — hence depends on **T0/F1** (so s3/s11
parity is measurable). If parity can't be proven, keep the substrate unwired (still a win).

---

## Disjointness / wave plan

```
Wave 0 (solo):   T0/F1  ........................ determinism (touches ~everything)
                   │ (byte-stable corpus; parity gate live)
                   ▼
Wave 1 (≈10 parallel — each owns disjoint files):
   T1  sandbox.rs                 T7  tests/whole_sample.rs
   T2  passes/rename.rs           T8  passes/devirt_detect.rs (new)
   T3  passes/math_fingerprint.rs T9  passes/unflatten_ladder.rs (new)
   T4  passes/helper_lift.rs(new) T10 ir/consts.rs + ir/analysis.rs
   T5  passes/dce.rs+dead_store.rs T11 ir/inline.rs
   T6  driver.rs+util.rs+lib.rs
                   │ (integrate sequentially: cargo test + report after each merge)
                   ▼
Wave 2:          T12 A4-wiring (dispatch.rs + unflatten.rs; parity-gated on T0)
```

**Only conflict surface in Wave 1:** `src/passes/mod.rs` (T4, T8?, T9 register new
passes) and `tests/equivalence.rs` (additive test appends) — both are trivial
adjacent-line merges the integrator resolves, exactly as in the A/B/C waves.

**Dependencies:** T0 before T9/T12 (parity) and before T7 switching to real host (T1).
T1 strengthens T7 and T10 but neither blocks. T8 is read-only and unblocks the future
D mega-project. Everything else is independent.

## Definition of done (this wave)
- Deterministic, byte-stable output across the corpus (T0); whole-sample equivalence
  harness green (T7).
- s7 host-relative decoders evaluable (T1); s11 table labels resolved under the bloat
  cap (T10); the split-then-mutate unsoundness fixed (T11).
- s3/s8 helpers named + (where feasible) `async`/`await` restored (T4); MBA guards
  collapsed (T3); spent machinery gone (T5); readability names inferred (T2).
- VM recognition reporting for s8/s9/s10 (T8); the A4 substrate wired for the integer
  dialect (T12); the engine measurably faster on s10 (T6).
- Suite green, clippy + fmt clean, every sample still converges, no byte regressions.
