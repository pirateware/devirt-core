# Plan: rename things by what they actually are

## What this does

Right now the output is correct but unreadable because every name is `var123` /
`fn45`. This renames bindings based on what they hold or do — `document` →
`doc`, a regex → `re`, a function that returns `a < b` → `lt`, the result of
`createElement` → `el`, and so on.

## Why it's worth doing

It's the change that actually makes a human look at the output and go "oh, now I
can read this." Byte count barely moves, but readability — which is the real goal
— jumps. And it's the safest thing on the whole list: renaming can't change what
the program does, as long as names stay unique within their scope. There's already
a `rename` pass to build on; this makes it a lot smarter.

## How it works

Extend the existing `RenameByRole` pass. Infer a name from, in rough priority
order:

- **What it's assigned**: a known global (`document`, `window`, `navigator`), a
  literal (string → `s`/named after content, number → `n`), a regex → `re`, a
  `new X()` → named after `X`, an array/object → `arr`/`obj`.
- **What a function does**: returns a comparison → `lt`/`eq`/etc., returns a table
  lookup → `decode`/`get`, returns `new Date` → `now`, forwards to another call →
  named after that.
- **How it's used**: always accessed as `.push` → it's a list; called with DOM
  args → named accordingly; the string keys it's indexed by.

Pick the best guess, fall back to a typed-but-generic name (`str1`, `fnLt`,
`el2`) when nothing's confident. Keep it scope-aware so two unrelated `var1`s in
different functions can get different sensible names.

## Keeping it sound

This one's easy on soundness — renaming preserves behavior. The only things to get
right:

- Names stay unique within each scope (don't accidentally merge two different
  bindings into the same name).
- Don't collide with globals the code reads, reserved words, or `undefined`.
- Don't rename across a binding's real boundary (respect shadowing). The semantic
  scope info we already build covers this.

So there's no value-equivalence risk — the test bar is "unique, valid, re-parses,
idempotent," not "same result." (Equivalence still runs as a backstop.)

## Steps

0. **Spike**: name-from-assignment for the easy cases (globals, literals,
   constructors) and confirm uniqueness + re-parse on all samples.
1. Name-from-behavior for functions (role inference from the body).
2. Name-from-usage (member-access and string-key context).
3. Tests: every renamed scope stays collision-free, no reserved-word/global
   clashes, output re-parses and is idempotent.

## How we know it worked

- The "opaque name ratio" metric in the report drops sharply across every sample
  (that's literally what it measures — fraction of 1–2 char / hex-ish names).
- Still idempotent and re-parsing; equivalence battery green as a backstop.
- Eyeball test: pick any sample, the output reads like real code instead of a wall
  of `var`s.
