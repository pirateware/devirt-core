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

---

## Round 2 (2026-05-05): array / object / `new` lowered

Implemented items 1–3 from the recommendation: array literals, object
literals (static / string / numeric / computed / shorthand keys), and
`new` expressions. Method-call discrimination was also added (`obj.f(x)`
now lowers to `CallMethod` instead of `LoadProp` + `Call`) — the latter
silently broke `this`-binding for any instance method call inside a
lowered body and was a prerequisite for the new functional tests to
pass.

### Handwritten corpus (after)

| program | lowered fns | PT fns | top-level PT stmts | PT share | Δ |
|---|---:|---:|---:|---:|---:|
| 01_array_map | 0 | 0 | 3 | 100% | — |
| 02_object_literal | 1 | 0 | 1 | **34%** | −66 pp |
| 03_arithmetic_only | 3 | 0 | 0 | 0% | — |
| 04_while_loop | 1 | 0 | 0 | 0% | — |
| 05_member_chain | 1 | 0 | 0 | 0% | — |
| 06_callback | 1 | 0 | 1 | 32% | — |
| 07_for_loop | 0 | 0 | 4 | 100% | — |
| 08_recursive | 1 | 0 | 1 | 19% | — |
| 09_iife | 0 | 0 | 2 | 100% | — |
| 10_closure | 2 | 0 | 3 | 32% | — |

**Handwritten average passthrough share: ~42%** (was 58%).

The single big win was 02_object_literal — `describe()` is now fully
lowered. The other programs were either already at 0% or are dominated
by *top-level driver statements* (`var x = ...; foo(x); console.log(x)`),
which the metric counts as passthrough regardless of expression form.
01_array_map, 07_for_loop, and 09_iife are entirely top-level — they
don't have a function whose disposition could change. Lowering helped
where it could; the rest is gated on different work.

### Functional_equivalence corpus (after)

The 14-program functional_equivalence corpus is unchanged: those
programs don't use array/object literals or `new` in their bodies.
Average passthrough share remains ~49% (most weight on the four
inherently-PT cases: class, generator, try/catch, and the listify
mixed test).

### Verdict

42% > 35% — per the spec's decision rule, **recommend implementing
items 4–6 (C-style `for`, compound assignment, `++`/`--`, ternary)
before Milestone 4**. The audit makes clear that `for` is the next
single biggest lever: 07_for_loop and 09_iife both stay at 100%
purely because of `for` constructs in their top-level driver code,
and 06/08/10 each carry a top-level driver line that today is
forced to PassThrough but would lower trivially under either (a)
top-level statement lowering or (b) a `for` desugar.

The new entry-point bug fixed in this round (Construct → bare `Call`
in ssa-back) means `new Date()` etc. now actually round-trip
correctly; this was latent because nothing emitted Construct before.

**Decision pending:** the user's brief instructs Round 2 to stop
here and surface the result. No further implementation.

---

## Round 3 (2026-05-05): C-style for, compound/update, ternary lowered

Implemented items 4–6 from the recommendation: C-style `for` loops
with `continue` semantics (break is intentionally deferred — see
`docs/DECISIONS.md`), compound assignment (`+=`, `-=`, `*=`, etc.)
including the single-evaluation rule for property targets, prefix /
postfix `++` and `--` in both statement and expression position, and
ternary `?:` lowered as a control-flow diamond.

The desugar choices and a small set of correctness divergences are
recorded in `docs/DECISIONS.md` (continue jumps to a per-loop step
block; break unsupported; `++`/`--` use generic `+`/`-` so non-numeric
inputs diverge from strict `ToNumber` coercion).

Side-bug fixed during this round: `find_diamond_join` in
`lower_tree.rs` could pick a diamond branch as its own join when
both branches transitively reach themselves through a surrounding
loop's back-edge, leading to infinite recursion at SSA→tree time.
Two fixes: exclude `b1`/`b2` from candidates (a block trivially
reaches itself but is not a join), and score by SUM of distances
rather than MIN so a true post-dominator beats per-branch tails on
ties. The bug was latent — pre-Round-3 corpus had no for/while body
that exercised the shape.

### Refined metric

`--coverage-stats` now reports two new lines:

- `function_body_passthrough_pct` — passthrough bytes from
  function bodies (top-level `Item::PassThroughFunction` and nested
  PT inner literals) over the function-body byte budget. This is
  the input to the milestone-decision rule. Top-level driver
  statements are excluded entirely so a program dominated by `var x
  = ...; foo(x);` doesn't drag the score down.
- `top_level_passthrough_pct` — informational only, top-level PT
  statement bytes over total-byte proxy.

The total `passthrough share` line is unchanged (sum of both, over
total proxy) and is retained for compatibility with Rounds 1–2.

### Handwritten corpus (after)

| program | lowered fns | PT fns | top-level PT stmts | fn-body PT% | top-level PT% |
|---|---:|---:|---:|---:|---:|
| 01_array_map | 0 | 0 | 3 | — (no fns) | 100% |
| 02_object_literal | 1 | 0 | 1 | 0% | 34% |
| 03_arithmetic_only | 3 | 0 | 0 | 0% | 0% |
| 04_while_loop | 1 | 0 | 0 | 0% | 0% |
| 05_member_chain | 1 | 0 | 0 | 0% | 0% |
| 06_callback | 1 | 0 | 1 | 0% | 32% |
| 07_for_loop | 0 | 0 | 4 | — (no fns) | 100% |
| 08_recursive | 1 | 0 | 1 | 0% | 19% |
| 09_iife | 0 | 0 | 2 | — (no fns) | 100% |
| 10_closure | 2 | 0 | 3 | 0% | 32% |

**Handwritten `function_body_passthrough_pct` average: 0%** across
the 7 programs that have at least one function. Every function in
the handwritten corpus now lowers its body fully. The 01 / 07 / 09
programs are pure top-level driver code with no function to measure
— they contribute only to `top_level_passthrough_pct`.

### functional_equivalence corpus (after)

| program | fn-body PT% |
|---|---:|
| 01_arithmetic_and_log | 0% |
| 02_while_loop_sum | 0% |
| 03_recursive_fibonacci | 0% |
| 04_if_else_branches | 0% |
| 05_class_in_body (PT by design) | 100% |
| 06_generator_in_body (PT by design) | 100% |
| 07_try_catch_in_body (PT by design) | 100% |
| 08_mixed_lowered_passthrough (PT listify by design) | 88% |
| 09_closure_capture | 0% |
| 10_capture_across_passthrough (PT inner by design) | 61% |
| 11_nested_pt_via_lowered (PT b by design) | 39% |
| 12_nested_pt_sibling_grandchild (PT by design) | 100% |
| 13_pt_observes_reassignment (PT by design) | 100% |
| 14_pt_capture_reassign | 0% |

The functional corpus is unchanged — every nonzero entry is a
program authored specifically to exercise the PT pipeline (class,
generator, try/catch, or a deliberately-PT inner literal). The PT%
on those entries is part of the test's specification, not a
coverage gap.

### Verdict

Decision rule: `function_body_passthrough_pct` averages **0% across
the handwritten corpus**, well under the 25% bar. **Recommend
proceeding to Milestone 4.** Remaining passthrough on the
functional corpus is by-design (class / generator / try-catch /
nested-PT cases authored to validate the PT pipeline) — not a
missing coverage feature.

Top-level passthrough remains 100% on the three pure-driver
programs (01, 07, 09) — informational only per the spec. Top-level
lowering would require routing non-function statements through a
synthesized module body, which is out of scope and orthogonal to
the optimization work that Milestone 4 enables.

The audit stops here. Implementation of Milestone 4 (first
optimization pass) is a follow-up ticket; direction depends on user
approval.
