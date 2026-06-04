# P0 final metrics — delta vs baseline

Branch `feat/cfg-recovery` after S1–S9. Suite: **182 green** (baseline 174;
+8 new adversarial / equivalence cases), clippy + fmt clean.

## Per-sample scoreboard (unchanged from baseline — see below)

| sample | switch | bytes | converged | vs baseline |
|--------|--------|-------|-----------|-------------|
| s1  | 38 | 894310  | true | = |
| s3  | 6  | 103040  | true | = |
| s4  | 5  | 130101  | true | = |
| s5  | 4  | 24133   | true | = |
| s6  | 5  | 24849   | true | = |
| s7  | 26 | 636549  | true | = |
| s8  | 40 | 250183  | true | = |
| s9  | 0  | 157990  | true | = |
| s10 | 86 | 1675367 | true | = |
| s11 | 37 | ~195461 | true | = (±2 B non-determinism) |

**The headline sample metrics did not move — by design, not by regression.** The
P0 §3 targets (s1/s4 switch begins dropping at S3; s1/s4/s11 switch ≈ 0 at S7)
rested on a premise that the re-diagnosis (`issues/12`) falsified: s1/s4/s11 do
not have setup-scoped symbolic single-symbol case labels.

- **s1**: VM-in-a-constructor (constants init'd in `new var2574()`). Needs a
  constructor-init happens-before proof — out of scope for a bounded sound widening.
- **s4**: labels already numeric; its dispatcher is `switch (var7 + var4)` with a
  **computed, stateful** discriminant + prefix statements — neither the linear nor
  the param-state nor the v1 CFG recovery models a non-identifier discriminant.
- **s11**: labels are 2D const-array reads (`var401[i][j]`); needs deterministic
  table resolution with a bloat cap (`issues/11`).

No sample regressed: every sample still converges (idempotent + reparse), and
no bytes grew. The no-regression invariant (P0 §4) held throughout.

## What did land (sound, equivalence-validated, green)

- **S1** — `OrderingOracle` + `ConstResolver` extracted; `global_const_inline` is
  a thin client. Parity verified.
- **S2** — IIFE-init setup regions (`var X = (IIFE)()` / `X = (IIFE)()`); new
  adversarial pair. + the re-diagnosis (`issues/12`).
- **S3** — unflatten init/loop **adjacency relaxed** (the real numeric-label
  blocker, found empirically); a non-adjacent dispatcher now fully unflattens.
- **S5/M1** — read-only recovered-CFG builder (`src/ir/cff.rs`) with back-edge
  detection; 4 graph-algorithm unit tests.
- **S6/M2 + S7/M3** — `unflatten-cfg` pass: recovers a conditional **self-loop**
  (the shape linear `simulate` declines) into `do { … } while (cond)`; gated on a
  recovered back edge so it never double-transforms; hand-written recursion to
  avoid the oxc insertion panic.
- **S8/M4** — declines soundly (no-op) on every unsupported shape (multi-state
  loops, branches, ternary/computed transitions); explicit decline equivalence
  tests; no panics.

## Soundness ledger

Every value-changing step is gated by the differential equivalence harness
(`tests/equivalence.rs`, QuickJS). 8 new cases added across S2/S3/S6/S8, each a
should-fire / must-decline pair where applicable. Zero equivalence regressions.

## Deferred / out of scope (documented for follow-up)

- **S4** (P1 value-axis: `EvalValue::Arr` + `var=pureCall()` preamble) — independent
  of the firing path; deferred per P0 §7.
- Multi-state `while`/branch structuring (P2 beyond the self-loop) — a full
  relooper; bounded v1 declines soundly. Headline samples need the mechanisms in
  `issues/11` and `issues/12` first, so expanding the relooper would not move them.
