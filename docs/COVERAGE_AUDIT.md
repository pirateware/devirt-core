# Lowering Coverage Audit

**Date:** 2026-05-05
**Driver:** `cargo run -- --coverage-stats <file.js>` from `crates/mvp`.
**Corpus:** `crates/mvp/coverage_corpus/` (10 handwritten programs, 14
extracted from `tests/functional_equivalence.rs`).

This is a diagnostic snapshot, not a deliverable. Numbers tell us which
constructs typical JS uses that are *not* in the lowered subset, so we
can decide whether to expand coverage before building optimization
passes.

## Metric

For each function in the tree-IR after lowering, we record:

- **disposition**: Lowered (made it into SSA) or PassThrough
  (preserved as a verbatim source slice).
- **lowered_stmts**: tree-IR `Stmt` count inside the function's own
  body, not crossing nested function boundaries (those are separate
  rows). Approximates "how much structured analysis we get."
- **passthrough_B**: source-slice byte count for PassThrough bodies.

Plus per-module: `top_level_passthrough_stmts` (count of
`Item::PassThroughStmt`s — every non-function top-level statement now
falls here per the recent IIFE-bug fix in `docs/DECISIONS.md`).

A coarse "passthrough share" metric appears at the bottom of each
report: `(passthrough_B + top_level_B) / (~30·lowered_stmts +
passthrough_B + top_level_B)`. The 30 B/stmt proxy is rough — useful
for rough comparison, not precise accounting.

## Results: handwritten corpus (typical small JS)

| program | lowered fns | PT fns | top-level PT stmts | PT share |
|---|---:|---:|---:|---:|
| 01_array_map (numbers.map(...)) | 0 | 0 | 3 | 100% |
| 02_object_literal | 0 | 1 | 1 | 100% |
| 03_arithmetic_only | 3 | 0 | 0 | 0% |
| 04_while_loop | 1 | 0 | 0 | 0% |
| 05_member_chain | 1 | 0 | 0 | 0% |
| 06_callback (each + IIFE-style call) | 1 | 0 | 1 | 32% |
| 07_for_loop | 0 | 0 | 4 | 100% |
| 08_recursive (fact + top-level call) | 1 | 0 | 1 | 19% |
| 09_iife | 0 | 0 | 2 | 100% |
| 10_closure (makeCounter + 2 calls) | 2 | 0 | 3 | 32% |

**Handwritten average passthrough share: ~58%.**

The Lowered-100% programs are notable: simple function declarations
without top-level driver code or array/object literals stay fully
lowered. As soon as a program does what real JS does — define an array
literal, use a `for` loop, write a top-level expression statement, or
call its functions — coverage collapses.

## Results: functional_equivalence corpus (14 tests)

| program | lowered fns | PT fns | top-level PT stmts | PT share |
|---|---:|---:|---:|---:|
| 01_arithmetic_and_log | 1 | 0 | 0 | 0% |
| 02_while_loop_sum | 1 | 0 | 0 | 0% |
| 03_recursive_fibonacci | 1 | 0 | 0 | 0% |
| 04_if_else_branches | 1 | 0 | 0 | 0% |
| 05_class_in_body (PT by design) | 0 | 1 | 0 | 100% |
| 06_generator_in_body (PT by design) | 0 | 1 | 0 | 100% |
| 07_try_catch_in_body (PT by design) | 0 | 1 | 0 | 100% |
| 08_mixed_lowered_passthrough | 1 | 1 | 0 | 88% |
| 09_closure_capture | 2 | 0 | 0 | 0% |
| 10_capture_across_passthrough | 1 | 1 | 0 | 61% |
| 11_nested_pt_via_lowered | 1 | 1 | 0 | 39% |
| 12_nested_pt_sibling_grandchild | 0 | 1 | 0 | 100% |
| 13_pt_observes_reassignment | 0 | 1 | 0 | 100% |
| 14_pt_capture_reassign | 2 | 0 | 0 | 0% |

The functional_equivalence corpus is structured to make functions
self-contained — no top-level driver code, since the test harness
appends the calls externally. The numbers therefore reflect *function-body
coverage only* and are flatteringly high. They are not representative
of typical JS input.

## What blocks lowering today

Reading the disposition walker (`crates/mvp/src/ir/disposition.rs`)
and counting which constructs each handwritten program tripped over,
the gaps that account for nearly all PassThrough fallback are, in
descending order of frequency in the handwritten corpus:

1. **Array literals `[1, 2, 3]`** — appear in 4/10 programs; force the
   enclosing function to PassThrough or, at top-level, the statement.
2. **Object literals `{ a: 1, b: 2 }`** — 1/10 in this corpus, but
   trivially common in real JS.
3. **`for` / `for-in` / `for-of` loops** — 2/10. Both require relooper
   support after lowering, but the parsing-shape walker can accept C-style
   `for` whose desugaring is a while-loop.
4. **Top-level non-function statements (any kind)** — every top-level
   driver line currently passthrough. Post-fix in `docs/DECISIONS.md`,
   they round-trip correctly; they just don't go through SSA.
5. **`new` expression** — 0/10 here, common in real JS.
6. **Compound assignment `+=` / `-=`** — 1/10.
7. **`++` / `--` (UpdateExpression)** — 1/10.
8. **Ternary `?:`**, **`??`**, **template literals** — 0/10 here, common
   in real JS.

Things that *already work* in the lowered subset (and the audit
confirms): member access (`a.b`, `a[i]`), member-call (`obj.method(x)`),
function expressions / arrow functions in argument position, plain
binary/logical/unary ops, `if`/`else`, `while`, `return`, `let`/`var`/`const`,
plain `=` assignment, recursive function calls, nested function
declarations, closures with capture.

## Recommendation

The handwritten corpus is the more representative sample. Its 58%
average passthrough share means optimization passes — when they land —
will fire on roughly half of typical input. That is not zero, but it
also rules out the deobfuscator producing visibly different output on
real programs without further coverage expansion.

**Highest-leverage adds, in order:**

1. **Array literals (read-only construction)**. Lowering target:
   `InstKind::AllocArray` (already exists in `ssa.rs`). The walker
   accepts; the front-end lowering and tree-back currently don't.
   Cost: small. Coverage gain: substantial — array literals appear
   nearly universally.

2. **Object literals (read-only construction)**. Lowering target:
   `InstKind::AllocObject` + StoreProp chain (also already in `ssa.rs`).
   Same cost/value profile as arrays.

3. **`new` expression**. Trivial in the IR (a Call variant with a
   different effect tag); the walker currently rejects.

4. **C-style `for` desugared to a while-loop**. The CFG shape that
   `lower_tree` handles already covers while; `for(init; cond; step)` →
   `init; while(cond) { body; step; }` rewrite at the front-end is
   straightforward. (For-in / for-of are bigger; defer.)

5. **Compound assignment + UpdateExpression**. `x += y` → `x = x + y`,
   `x++` → `x = x + 1`. Front-end desugar; no IR changes.

6. **Ternary `cond ? a : b`** as if/else with a synthesized temp. Pure
   front-end transformation.

Items 1-3 alone would, eyeballing the handwritten corpus, drop the
average passthrough share well under 25% — most programs become
fully lowered. Items 4-6 are mop-up for the rest.

**Not recommended now:**

- `try`/`catch`, `throw`, generators, async/await, classes,
  destructuring, spread, template literals. These are correctness-load
  for an obfuscation undo tool but low-frequency in the obfuscator.io
  output samples we eventually need to handle. Keep them in
  PassThrough; the source-slice round-trip is correct.

**Not in scope of this audit (recommend, do not implement now):**

Per the user's brief on this task, the audit stops at the
recommendation. Implementation of items 1-6 is a follow-up ticket;
direction depends on user approval.
