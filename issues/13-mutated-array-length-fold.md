# 13 — mutated array's `.length` / element read folds to the empty-literal value

- **Found by:** the generated synthetic corpus (`tests/generated_corpus.rs`).
- **Samples (quarantined in `KNOWN_BUGS`):** `seed_09_base64__{controlflow,deadcode,numbers_keys,strong}`,
  `seed_13_try_catch__{deadcode,strong}`.
- **Impact:** **Soundness** (wrong output) · **Effort:** Medium-High
- **Status:** ✅ FIXED — all six quarantined cases pass; `KNOWN_BUGS` is now empty.

## Symptom

A `const a = []` that is mutated by `a.push(...)` has reads of it folded to the
**empty-literal** value: `a.length` → `0`, `a[i]` → `undefined`. The base64 seed's
`b64encode` returns `""` / `undefined`-garbage; seed_13's `log.length` reads `0`.
The mutation (`a.push`) is present and reachable in the output — the bug is that a
pass treated `a` as the immutable `[]` it was initialised to.

## What was fixed

The corpus first exposed the **simple** form — `const a=[]; a.push(x); … a.length`
in one function — which folded to 0 because *two* escape analyses ignored
mutator-**method** calls (they handled `a[i]=x` member writes and `a[i]++`, but a
call `a.push(x)` slipped through as a clean member read):

- `src/ir/analysis.rs` — `Classifier::visit_call_expression` (used by `ssa-opt` via
  `collect_candidates`) now excludes the receiver of a `push`/`splice`/… call, and
  of any **computed-name** method call (`a[_dec("push")](x)` before the string
  decode lands — conservatively a possible mutator).
- `src/ir/consts.rs` — `Census::visit_call_expression` (used by
  `global-const-inline`) does the same disqualification, plus member-write /
  `delete` / computed-method bases.

This fixed `seed_09` under `minimal` / `strarr_base64` / `strarr_rc4` and made six
*real* samples slightly larger (previously-unsound folds now declined, behavior
preserved — verified by the equivalence harness).

## Root cause (the obscured shapes) and the fix

A per-round AST bisect pinned **`decoder-lift`** as the folder in both remaining
shapes — it bakes a binding's initial `[]`/`{}` into the sandbox preamble and
then evaluates reads of it (`t.length`, `t[i]`) to the empty-literal value. Two
distinct gates let a *mutated* aggregate through:

1. **`seed_09` (function-local `bytes`)** — the SSA escape query
   `mutable_aggregate_tables` (`src/ir/analysis.rs`) failed to disqualify a
   receiver mutated through an **opaque computed-method call**
   (`bytes[_dec("push")](b)`, before the string decode lands). Its
   `visit_call_expression` only treated a *string-literal* computed key as a
   possible mutator and fell through (`_ => None`, a clean read) for a decoder-
   call key — the inverse of conservative. Fixed by assuming an opaque computed
   method name **may** mutate (matching the already-correct logic in
   `src/ir/consts.rs`).

2. **`seed_13` (top-level `const log = []`, pushed to in a `finally`)** —
   `decoder-lift`'s `classify` seeds its pure-data set from top-level
   `is_pure_data` initializers, gated only by `scan_unstable` (bare reassigns /
   `++`). That scan is blind to `.push`/member mutation, so `log` entered the
   pure set, was baked as `[]`, and `log.length` folded to 0. Fixed with a new
   `scan_live_mutated` query (`src/passes/decoder_lift.rs`) that flags an
   aggregate mutated by anything **other than** the canonical length-preserving
   rotation `t.push(t.shift())` / `t.unshift(t.pop())`. A rotation-only table
   stays bakeable (the rotation IIFE is replayed verbatim in the preamble); a
   table the live program grows/edits is excluded.

`scan_live_mutated` is the decoder-lift-specific complement to
`mutable_aggregate_tables`: the latter (correctly) rejects *every* mutated
aggregate and so cannot be used here — it would also drop the rotated string-array
decoder tables this pass exists to lift.

## Verification

All six formerly-quarantined cases now reproduce their expected program output
byte-for-byte; `KNOWN_BUGS` in `tests/generated_corpus.rs` is empty. The
behavior-equivalence and determinism harnesses pass. Two real samples re-blessed
(`sample_7`, `sample_14_recaptcha`): both changes are in the **conservative**
direction (a declined fold, plus an incidental deterministic preamble-layout
shift in a baked stack-trace line number) — never a changed behavior.
