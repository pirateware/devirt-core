# 13 — mutated array's `.length` / element read folds to the empty-literal value

- **Found by:** the generated synthetic corpus (`tests/generated_corpus.rs`).
- **Samples (quarantined in `KNOWN_BUGS`):** `seed_09_base64__{controlflow,deadcode,numbers_keys,strong}`,
  `seed_13_try_catch__{deadcode,strong}`.
- **Impact:** **Soundness** (wrong output) · **Effort:** Medium-High
- **Status:** PARTIALLY FIXED — simple shapes fixed; obscured shapes open.

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

## What remains (this issue)

Under **control-flow flattening / dead-code injection**, the same fold still
happens. On the clean repro (`b64encode` obfuscated with the `controlflow`
profile, then deobfuscated) the folding pass is **not** `ssa-opt` or
`global-const-inline` (both report 0 changes and correctly exclude the array).
The passes that *do* change are `const-fold`, `inline-single-use`, `decoder-lift`,
`member-normalize`, `dce`. So in a round *before* the mutation is statically
visible (CFF has scattered `a.push` across switch-dispatched blocks, or the
property is still a string-array index), one of these substitutes `[]` for `a`
into a `.length` / `a[i]` read, then `const-fold` reduces `[].length`→0 /
`[][i]`→undefined.

### Next step

Per-round AST bisect on the clean `controlflow` repro to pin the exact pass and
round, then give that pass the same mutator-aware escape gate (or make the
empty-array-literal substitution refuse a binding that is mutated anywhere in the
program — the `mutable_aggregate_tables` query already computes exactly this set
and could be consulted by the inliner / const-fold member rule).

## Why quarantined, not reverted

The partial fix is sound and a net improvement (it fixes the common shapes and
tightens real-sample folding). The residual obscured shapes are a pre-existing
latent bug the new corpus surfaced; they are quarantined (loudly, in `KNOWN_BUGS`)
so the suite stays green while the remaining fold path is tracked here.
