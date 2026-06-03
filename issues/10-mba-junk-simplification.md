# 10 — MBA predicates and junk guards not folded

- **Samples:** 11 (DataDome), 10 (reCAPTCHA — downstream of issue 02)
- **Impact:** Medium · **Effort:** ~~Medium~~ **High (research-grade)** · **Risk:** Medium
- **Pass:** `src/ir/` const-fold extension; `src/passes/dead_store.rs`, `src/passes/flatten_else.rs`

> **STATUS (2026-06): DEFERRED — the real shape needs an MBA simplifier, not
> constant folding.** Inspecting the actual sample_11 predicates overturns the
> "both operands constant" premise. The junk-result vars **are read**, as operands
> of opaque MBA guards over *runtime* values:
> ```js
> if (... : -6*(var405 & var410) + 1*(var405 & ~var410) + 1*~(var405 & ~var410)
>          + 7*~(var405 ^ var410) - 8*~(var405 | var410) > -780) { ... }
> ```
> `var405`/`var410` are not constants, so the junk-strip path (drop unread pure
> results) does not apply, and plain const-fold cannot touch this. These are
> **input-independent** linear combinations of bitwise terms — constant for *all*
> inputs by construction. Collapsing them requires a genuine **MBA simplifier**
> (prove a bitwise linear combo is constant via basis evaluation / interpolation),
> which is a research-grade pass, not a const-fold tweak.
>
> The alternative — fold `win.Math.floor(842.73)`→`842` first to make the operands
> constant — requires proving `win` is the global object, which is **not generally
> safe** (`win` could be reassigned/shadowed; the method call could throw). Both
> avenues exceed medium effort. Deferred; the MBA simplifier is the right long-term
> lever and would also help sample_10's 2,708 predicates.

## Symptom

After the higher-priority passes land, two classes of noise remain:

**sample_11** — ~70 mixed boolean-arithmetic (MBA) predicates and 122
result-discarded junk calls:
```js
// MBA opaque predicate (both operands constant after const-prop)
3*(a&b) - 3*(a&~b) + ... < K
// junk guards — pure, value unused
win.parseInt(5.71);  win.Math.ceil(9.51);   // ×122
```

**sample_10** — 2,708 opaque arithmetic predicates gating dispatch handlers
(`((var3074|2)&5)==1`, `var3065<<1&11`, …). These fold automatically **once the
constant selector is bound** by demultiplex (issue 02), so this is mostly downstream
cleanup for sample_10.

## Root cause

const-fold does not evaluate MBA identities where both operands are constant, and
`dead_store` does not strip pure expression statements whose value is discarded
(`win.parseInt(float)` with no use).

## Recommended fix

1. Extend the const-fold logic in `src/ir/` to evaluate constant MBA predicates
   (`k1*(a&b) + k2*(a&~b) + … < C` with both operands constant) to `true`/`false`,
   then let `dce`/`flatten_else` drop the dead arm.
2. Teach `dead_store` to strip result-discarded pure builtin calls
   (`win.parseInt(<literal>)`, `win.Math.ceil(<literal>)`) — ~122 junk statements in
   sample_11.

Lower priority — largely cosmetic once issues 02/03/04 land, but removes ~190 junk
expressions in sample_11 and finishes the predicate-folding for sample_10.

## Related

- Issue 02 (binds sample_10 selectors so its 2,708 predicates become constant-foldable).
- Issue 11 (the `var403` table resolution that also feeds sample_11 predicate folding).
