# 11 — Deterministic state-key table (`var403`) not statically resolved

- **Samples:** 11 (DataDome)
- **Impact:** Medium (unlocks issue 01 for sample_11) · **Effort:** High · **Risk:** Medium
- **Pass:** `src/sandbox.rs` (deterministic-table evaluator); feeds `src/passes/unflatten.rs`

## Symptom

sample_11's 31 control-flow-flattening dispatchers use case labels and transition
targets that are **lookups into a runtime-built 256-element key table** `var403`,
not literals — so they are opaque to const-fold and to `unflatten` (issue 01).

## Evidence

```js
// dispatcher (out/sample_11.out.js:2584) — labels/targets are var403[i][j]
for (var var482 = var403[253][201]; true;) switch (var482) {
  case var403[58][4]: break;
  case var403[85][126]: ...; var482 = var403[57][55]; continue;
}

// the table is built deterministically, in 8 async setTimeout chunks
arr14[m][p] = arr14[(419*p ^ 661*m ^ 749) >>> 0 & 31];
... var397(function(){ var403 = arr14[7]; });
```

The values are fixed; the `setTimeout` wrapping only sequences the fills. decoder-lift
rejects `var403` because it is built through `setTimeout` closures (async, free var
`var397`), not a synchronous pure expression.

## Recommended fix

Add a **deterministic-table evaluator**: recognize the `var403 = arr14[…]`
async-built table, replay the (deterministic) fill formula in the sandbox
(`src/sandbox.rs`) to a concrete `[][]` constant, and fold `var403[i][j]` lookups to
integer literals.

This is gated on issue 09's array-valued sandbox results (the table is a 2-D array),
and once it lands the dispatcher labels become literals — which is the precondition
for `unflatten` (issue 01) to fire on sample_11's 31 dispatchers.

## Do NOT touch

The 6 legitimate source switches in sample_11 (`out:230, 271, 932`, plus the
string-keyed `case "url"`/`case "host"` and bitmask `switch(255 & var79)`). Those are
real control flow, not flattening.

## Related

- Issue 09 (array-valued sandbox results — prerequisite).
- Issue 01 (unflatten — the downstream consumer this unblocks).
- Issue 04 (nested-scope decoder lift — runs first to decode `call3` strings).

## STATUS (ws-b-resolution / B2) — re-diagnosed, sound decline

The roadmap framed B2 as "resolve `T[i][j]` when `T = arr14[7]` is a nested
const-array alias." Investigation shows **that capability already exists and is
already sound** in the current pipeline: a provably-immutable 2-D const array,
including the cross-function aliased shape (`var T = base[k]; … T[i][j]`),
*already* folds to literals (verified: `nested_const_array_2d_resolves_to_literals`
in `tests/phase1.rs`, `nested_const_array_2d_and_alias_fold_preserves_value` in
`tests/equivalence.rs`).

The real blocker for sample_11 is **not** 2-D-alias folding — it is that `arr14`
is **mutable**, built by member writes (`arr14[i][j] = …`) inside **async
`setTimeout` closures**, and `var401 = arr14[7]` is itself assigned inside a
`setTimeout` closure. `mutable_aggregate_tables` (correctly) rejects a
member-written table, so the element reads are left symbolic — the sound outcome.
Locked in by the adversarial sibling `member_written_table_labels_stay_symbolic`
(`tests/phase1.rs`) and `member_written_table_is_not_resolved` (equivalence).

Statically resolving sample_11's labels therefore requires the **deterministic-
table evaluator** this issue originally recommended (replay the deterministic fill
formula `arr14[i][j] = arr14[(419*j ^ 661*i ^ 749) >>> 0 & 31]` in the sandbox to a
concrete `[][]` constant), gated on issue 09's `EvalValue::Arr` — a separate,
larger effort, not in-scope for the B2 alias-resolution item. The bloat cap the
roadmap demands is moot for the existing fold path: element reads are replaced
individually with their literal value, never re-materializing the whole table per
read, so there is no table-expansion bloat (s9's 131 KB bytecode array stays
intact — verified no byte growth).
