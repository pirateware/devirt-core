# Plan: inline the one-shot stuff

## What this does

Obfuscators blow everything up into variables that are used exactly once and tiny
functions that are called exactly once. This pass folds each of those back into
the place it's used and deletes the leftover. A quick count of the corpus: ~4,600
single-use bindings in sample_10 alone, ~900 in sample_8, hundreds in the rest.

## Why it's worth doing

Two reasons:

1. It shrinks the code and removes a whole layer of pointless indirection.
2. More importantly, it *snowballs*. Once `var t = decode(5); obj[t]` becomes
   `obj[decode(5)]`, the decoder pass can finally see the call and crack it.
   Inlining puts expressions back next to each other so every other pass
   (const-fold, decoder-lift, member-normalize) gets more to chew on. It makes the
   machinery we already have reach further without writing new pattern-matchers.

This is the generic lever — it helps every sample, not one specific scheme. It's
also the same idea as the existing `proxy-inline` pass (which the notes call the
biggest readability win), just without the "only operator tables" restriction.

## How it works

Reuse the CFG/SSA layer in `src/ir/`. For a binding:

- Find bindings with exactly one definition that reaches exactly one read.
- Replace the read with the definition's expression, delete the binding.

For a function: if it's called exactly once and isn't recursive, drop its body in
at the call site (rename its params to the args, watch evaluation order).

## Keeping it sound (this is where inlining bites you)

Inlining is exactly where you drop or reorder side effects if you're sloppy — the
whole reason the SSA layer exists. The rules:

- Only inline when the definition reaches the use with nothing reassigning it in
  between. SSA already tells us this.
- Don't move an expression across other side effects. If `t`'s value came from
  something with effects (a call, an assignment), it can only slide down to the
  use if nothing effectful happens in between. The CFG event order tells us.
- Single use only — never duplicate an effect.
- Skip anything aliased, captured by a closure, or recursive.
- For functions: no funny business with `this`/`arguments`, args evaluated
  left-to-right, params not reassigned in the body in a way that breaks the
  substitution.
- When unsure, leave it. Partial is fine.

## Don't make it worse

Blindly inlining can turn 4,600 named temps into 4,600 giant expressions, which is
*less* readable. So gate it: inline when the definition is small, OR when inlining
clearly unlocks a downstream fold (the def feeds a decoder call, a member access,
a constant test, etc.). Leave big expressions named if inlining them just builds a
monster.

## Steps

0. **Spike**: inline single-use *value* temps in one function, prove it's sound on
   a couple of equivalence cases.
1. General single-use value inlining, SSA-gated, with the "small or unlocks a fold"
   heuristic.
2. Single-call function inlining.
3. Adversarial tests first: effects in the def, effects between def and use,
   aliased/captured/recursive targets, eval order — each must stay correct.

## How we know it worked

- sample_10 / sample_8 shrink and their single-use counts drop a lot.
- The equivalence battery (run obfuscated vs deobfuscated, compare results) stays
  green, including the new adversarial cases.
- All samples still re-parse and are idempotent (second run changes nothing).
- Bonus signal: decoder-lift / const-fold report *more* hits afterward, because
  inlining exposed work to them.
