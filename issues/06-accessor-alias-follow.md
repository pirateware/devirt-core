# 06 — decoder-lift does not follow local accessor aliases

- **Samples:** 5 (Cloudflare)
- **Impact:** ~~Medium~~ **Low** · **Effort:** ~~Medium~~ **High (fiddly)** · **Risk:** Medium
- **Pass:** `src/passes/decoder_lift.rs` (supported by `assigned_once`/`inline_single_use`)

> **STATUS UPDATE (2026-06, branch `ws-c-decoders`): the general `.bind()`
> accessor-alias follow landed (C3); the s5 `fn12` sequence/param-table case stays
> declined.** `collect_aliases` now recognizes `var g = dec.bind(<pure-args>)` (and
> the assignment form) and lifts calls through the bound alias of a classified-pure,
> `this`-free decoder — the bind receiver/leading args are pure-data-gated so the
> sandbox reproduces the alias exactly, only the first chain edge may carry a
> `.bind`, and the emit is rebuilt against the resolved pure-func name. Tests:
> `lifts_through_bind_accessor_alias`, `does_not_lift_reassigned_bind_alias`. No
> current sample's surviving decoder uses that exact bound-alias form, so sample
> bytes are unchanged — it is a correctness widening for inputs that do.
>
> The specific s5 `fn12` case below remains **declined** (sequence-expr alias +
> param-as-table), unchanged from the original analysis:
>
> **ORIGINAL STATUS (2026-06): SKIPPED — not worth the risk/payoff.** Inspecting the
> real `fn12` shows the alias is **not** the simple `var442 = var1` the doc assumed.
> It is:
> ```js
> function fn12(var438, var439, var440, var441, var442, …) {  // var441,var442 are PARAMS
>   var442 = (var441 = { g:…, z:…, K:… }, var1);              // RHS is a SequenceExpression
>   … var2[var442(var441.g)] …                                // 22 calls
> ```
> `collect_aliases` disqualifies `var442` because its RHS is a comma expression,
> not a bare identifier. Even if alias-through-sequence were added (sound: the
> assignment's value is the last operand), the call **arguments** are `var441.K`
> where `var441` is a *parameter* reassigned to an object literal — so it is not
> captured by `scan_invariants` as a local key table, and the argument would fail
> the "allowed names" gate. Lifting these 22 calls would require (a) alias-through-
> sequence **and** (b) param-as-local-table support, both soundness-sensitive, for
> a 22-call payoff in one function of one sample. Deprioritized below the
> general-purpose wins.

## Symptom

In sample_5 the string table is decoded everywhere **except** inside `fn12`
(`out/sample_5.out.js:440-475`), where 22 indirect lookups survive — the single
remaining block of string indirection in the sample. (Samples 6 and 8 have their
equivalent helper fully literalized, e.g. sample_6 `fn11` at `out:445-467` is plain.)

## Root cause

Inside `fn12` the table accessor is re-aliased to a function-local copy and called
through it:

```js
// out/sample_5.out.js:440-475
function fn12(...) {
  var var442 = var1;                                  // local alias of the accessor
  ... var2[var442(var441.g)][var442(var441.K)] ...    // 22 calls via var442
}
```

decoder-lift's call-site classifier only recognizes the accessor by its **direct
callee name**; it does not chase a same-scope single-assignment alias
(`local = <pureAccessorName>`), so the 22 `var442(...)` calls are never lifted.

## Recommended fix

In the call-site "is this callee a pure decoder" check in `decoder_lift.rs`, resolve
same-scope single-assignment aliases before classifying: if `var442` is assigned
exactly once from a known pure accessor (`var1`) and never reassigned, treat
`var442(x)` as `var1(x)` for lifting purposes.

This is a targeted extension to the existing pass, not a new pass. The
`assigned_once` / `inline_single_use` passes already perform alias folding and could
feed this (or run before it in the fixpoint).

## Related

- Issue 05 (dead rotation machinery — once `fn12`'s lookups are lifted too, the
  whole table/accessor becomes droppable in sample_5).
