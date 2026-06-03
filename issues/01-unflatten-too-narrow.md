# 01 — `unflatten` precondition too narrow (0 fires on every real sample)

- **Samples:** 1, 4, 8, 11
- **Impact:** High · **Effort:** High · **Risk:** Medium
- **Pass:** `src/passes/unflatten.rs` (~422 lines); IR substrate `src/ir/{cfg,dom,ssa}.rs`

## Symptom

Per run stats, `unflatten` reports **0 changes on every real sample**
(1,3,4,5,6,7,8,9,10,11) and only fires on the toy `sample_12`. Yet the outputs
for samples 1, 4, 8, 11 still visibly contain `while(state){switch(state){…}}`
control-flow-flattening dispatchers. The pass is effectively dead on real input.

## Root cause

`Unflatten::enter_statements` (`unflatten.rs:62`) only matches a **toy canonical**
machine. All of the following must hold:

1. State init is a **numeric literal** — `var s = <int>` (`state_init` L115, `int_literal` L143). Non-literal init bails.
2. Top-level `while(<ident s>){ switch(<ident s>){…} }` — both loop test and discriminant must be the bare identifier (`while_switch` L163); exit only when `s` becomes falsy.
3. No `default` clause (L184).
4. `s` is referenced nowhere else in the statement list — fails for a **function-parameter** state var (`dispatch_only_in_list` L352).
5. Every `case` label is an **integer literal** (`parse_cases` L215).
6. Each case ends in a single top-level `s = <int>` transition or `return`/`throw`; a **predicated** transition (inside `if`/`for`) bails (`is_state_transition` L286, `sole_transition` L270).
7. The simulated chain is acyclic and literal-deterministic; aborts on a revisited state (`simulate` L316).

Real obfuscators violate these in combination — see per-sample below.

## Evidence (real dispatcher shapes)

**sample_4** `fn10` (`out/sample_4.out.js:1916`, input `samples/sample_4.js:1933`):
```js
var var163 = function fn10(var164, var165) {   // state is a PARAMETER
  while (var164 != var672) {                   // test is `!=`, not falsy while(s)
    switch (var164) {
      case var591:                             // label is a VARIABLE, not int literal
        var164 = var672;                        // transition target is a variable
        for (...) { ... } return ...;
      case var829: ...; var164 = var591; ...
```
Fails (1) no literal init, (2) non-falsy `!=` test, (4) param state, (5) var labels, (6) var transition. Another sample_4 machine uses a **composite discriminant** `do{ switch(tT+bT) }while(tT+bT != BD)` (input L427/433).

**sample_1** `fn46` (`out/sample_1.out.js:2190`): parameter-dispatched jump table —
`switch(var582)` with **no `while`** and `case var1479:` variable labels. Other
sample_1 switches use composite discriminants `switch(var705 + var702)` (L3063).

**sample_8** Cloudflare bytecode VM (`out/sample_8.out.js:182`):
```js
var136 = "3|2|4|1|0".split("|"), var137 = 0;
for (...; true;) switch (var136[var137++]) {   // PC into a split() opcode table
  case "0": ...; continue; case "1": ...; continue; }
```
Discriminant is `arr[pc++]`, labels are **string** literals, order encoded in the
`split("|")` array. This is a genuine interpreter loop (40 such switches) — out of
scope for any structured unflattener.

**sample_11** (`out/sample_11.out.js:2584`):
```js
for (var var482 = var403[253][201]; true;) switch (var482) {
  case var403[58][4]: break;
  case var403[85][126]: ...; var482 = var403[57][55]; continue; }
```
State init + case labels + transition targets are all **nested member expressions**
`var403[i][j]` (a runtime-built key table — see issue 11). 31 such dispatchers.
The remaining 6 sample_11 switches (`switch(255 & var79)`, `case "url"`) are
**legitimate source switches** and must NOT be touched.

## Recommended fix

**Option A — incremental relaxation (do first):** edit `unflatten.rs`.
- (5)/(6) Replace the bare `int_literal` requirement in `parse_cases` (L215) and
  `is_state_transition` (L302) with a resolver that also accepts an identifier/member
  proven write-once-literal in scope — reuse the global-const-inline / SSA const
  machinery rather than re-deriving. Unlocks sample_4 `fn10` and var-labeled sample_1.
- (2) Generalize `while_switch` (L163) to accept `for(init; true|cond; )` and
  `do…while(cond)`, model the loop exit as "no case matches and condition false"
  instead of hardcoded `state==0` (`simulate` L326). Accept `a+b` / `x&mask`
  composite discriminants when operands are dispatch-only.
- (4) Allow the state var to be a **function parameter** (`dispatch_only_in_list`
  L352): the entry state is the initial param value.

Each relaxation is independently soundness-gated by the existing "state var is
dispatch-only" invariant (L78/L253) — keep that gate or real switches get corrupted.

**Option B — CFG redesign (if A insufficient):** rebuild on `src/ir/`. Lower the
dispatcher to a CFG with one node per case keyed by its (SSA-resolved) state value,
recover edges from transition assignments **including predicated ones**, use
dominators (`dom.rs`) to find back-edges and reconstruct original loops, then
re-emit structured control flow (`flatten-else` cleans up afterward). Handles the
table-driven branching machines (sample_11) that Option A's linear simulator cannot.

**Out of scope either way:** sample_8's bytecode VM (`arr[pc++]` over a `split("|")`
table) — that's an interpreter, a different problem class.

## Related

- Issue 11 (resolve `var403` so sample_11 labels become literals — unblocks this).
- The `unflatten.rs` doc-comment (L1–27) should record which real shapes are
  explicitly unsupported, so "0 fires" reads as expected-narrow, not a regression.
