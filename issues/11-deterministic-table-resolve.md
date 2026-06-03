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
