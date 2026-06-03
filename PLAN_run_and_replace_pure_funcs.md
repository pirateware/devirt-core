# Plan: run pure functions and replace them with the answer

## What this does

Two related moves, both built on the QuickJS sandbox we already have:

1. **Fingerprint the fake math.** Obfuscators replace `a + b` with garbage like
   `fn56(a,b) = 4*(b|a) - 3*(~b&a) + 6*~(b|a) - 2*~(b^a) - ~a - (b|~a) - ~b + 1`.
   These are everywhere. Instead of trying to simplify the algebra, just *run the
   function* on a big batch of input pairs and see if its output matches a simple
   operation (`a+b`, `a-b`, `a^b`, `a&b`, `a|b`, `a<b`, etc.) every single time.
   If it matches, replace the function — and all its calls — with the simple
   operator. This works even when the call arguments are runtime variables, which
   is the whole point: you don't need the args to be constants.

2. **Evaluate constant calls in general.** If any side-effect-free function is
   called with constant arguments, just run it and drop the result in. Decoders
   are one special case of this; lots of other helper functions are too.

## Why it's worth doing

The math proxies are a huge, generic noise category across sample_5/6/8/11 and
others. Fingerprinting kills them outright and turns `fn56(x, y)` back into
`x + y`, which then lets const-fold and everything downstream work. The
constant-eval half generalizes decoder-lift beyond just "decoders" to any pure
helper.

## The memoized-decoder sub-case (sample_7)

Some decoders look impure because they cache their results:
`cache[i] === undefined ? cache[i] = decode(table[i]) : cache[i]`. The cache write
doesn't change the answer and nobody else reads the cache, so the function is
"pure enough." Treat a function as pure if its only side effect is writing to a
private object that never escapes and is only ever touched as `obj[key]`. That
unlocks sample_7's encrypted-key wall (~1,300 sites) using the same sandbox.

## How it works

Reuse the sandbox and the existing purity/free-variable analysis.

**Fingerprinting:**
- Find small functions of 1–2 args with no side effects (besides a private cache).
- Run them against a fixed battery of inputs — include the nasty ones: 0,
  negatives, large ints, mixed signs, and for strings a few representative values.
- Compare the outputs to a table of candidate simple operations.
- Require an *exact* match (value and type) across the whole battery. If it
  matches, rewrite the function body (or just rewrite every call site to the
  operator).

**Constant eval:**
- A pure function called with literal args → run `preamble; theCall` in the
  sandbox, splice the result back as the right kind of literal.

## Keeping it sound

- The function has to be genuinely pure: only its args, the safe-globals list, and
  a private cache. No DOM, no `this`, no randomness, nothing that escapes.
- Fingerprinting needs enough inputs (including edge cases) that a coincidental
  match is basically impossible — and run it twice to be sure it's deterministic.
  A near-match is not a match; one disagreement and we bail.
- Numbers stay numbers, strings stay strings — never let a numeric result splice
  in as a string.
- If anything's off, leave the function alone.

## Steps

0. **Spike**: fingerprint one real math proxy (e.g. fn56) and confirm it collapses
   to the right operator, equivalence-checked.
1. Fingerprint binary/unary math + comparison ops generally.
2. Generalize constant-arg evaluation to any pure function (not just decoders).
3. Add the private-cache exception so memoized decoders count as pure (sample_7).
4. Adversarial tests first: a function that *almost* matches `a+b` but differs on
   negatives must NOT be replaced; an impure one must be left; a cache that's read
   elsewhere must block the memoized case.

## How we know it worked

- The `fnNN(a,b)` math proxies turn back into plain operators; sample_5/6/8/11 get
  noticeably cleaner.
- sample_7's `obj[fn2(123)]` style sites resolve to real property names.
- Equivalence battery green, all samples idempotent and re-parsing.
