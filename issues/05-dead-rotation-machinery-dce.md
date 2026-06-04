# 05 — Dead rotation / string-table machinery not removed after lift

- **Samples:** 5, 6, 8 (Cloudflare family)
- **Impact:** ~~High~~ **Modest** · **Effort:** Medium · **Risk:** **Medium (soundness)**
- **Pass:** `src/passes/decoder_lift.rs`, `src/passes/dce.rs`, `src/passes/dead_store.rs`

> **STATUS UPDATE (2026-06, branch `ws-c-decoders`): the nested spent-rotation
> removal landed (C4).** `decoder_lift`'s cleanup now (a) recognizes *self-caching
> table providers* (`function call1(p){ p=<lit>.split(";"); call1=()=>p; return
> call1() }`) whose ONLY external references are rotation-IIFE arguments — the array
> escapes only via the provider's return, so if no live code calls it, it reaches
> nothing; and (b) descends through transparent IIFE/block/try wrappers for both the
> liveness scan and the removal, so the *nested* rotation (Cloudflare wraps it inside
> the anti-bot init IIFE) is found and dropped. LivenessScan treats scaffolding
> function declarations as dependency edges, not liveness, and excludes a provider's
> own self-references — so the table is pinned live only by a non-scaffolding call.
> Tests: `removes_nested_spent_rotation_iife`, `keeps_rotation_when_table_still_read`
> (table escapes to host → kept). **Result: sample_6 −237 bytes** (rotation dropped;
> the now self-referential `call1` + its table are left to a future nested-DCE pass).
> sample_5 stays blocked by `fn12` (issue 06's declined case keeps the table live);
> sample_8's `call1` is genuinely-live app code (`getElementById`-style), correctly
> untouched. No regression; the must-NOT-fire test guards the escape case.
>
> **ORIGINAL STATUS (2026-06): DEFERRED — soundness risk outweighs the payoff.**
> Findings:
> - **sample_5 cannot be cleaned** — the rotation accessor (`var1 = get1`) is still
>   read by `fn12` (the 22 unlifted calls of issue 06), so the cluster is live.
>   Coupled to issue 06 (skipped).
> - **sample_6/8** — the accessor is already inlined+DCE'd; only the rotation IIFE
>   + the `call1` table (~2.9 KB) survive. The IIFE *reads* `call1` (keeps it
>   alive) and has a side effect (`list.push(list.shift())` on `call1`'s cached
>   array). Dropping it requires a dedicated recognizer that proves the rotated
>   array **never escapes** and that `call1` is referenced *only* by the rotation.
> - **Why deferred:** a pattern-based DCE that removes a side-effecting IIFE on a
>   "looks self-contained" heuristic is precisely the side-effect-dropping bug
>   class flagged in `memory/soundness-audit.md`. The sample tests assert only
>   reparse + idempotency, **not** semantics, so an unsound drop here would pass
>   CI silently. The payoff (~3 KB on two samples) does not justify that risk
>   without first extending the equivalence harness to cover these samples
>   end-to-end. Revisit alongside the SSA escape analysis in `src/ir/`.

## Symptom

In the Cloudflare family the string-table indirection is **already decoded at call
sites** (decoder-lift runs the accessor in the sandbox after de-rotation — this
works well). But the now-unused machinery is left as dead code: the de-rotation
IIFE, the `;`-joined table string (~280 entries), and the accessor functions all
survive.

## Root cause

The pipeline inlines the *results* but does not delete the producers:

```js
// out/sample_5.out.js:3-11, 432-438
var1 = get1;
(function(var14,var15,var16,var17,list,var18){    // rotation IIFE — never materialized
  for (list = var14(); true;) try {
    if (var18 = 211938, var15 === var18) break;
    list.push(list.shift());                       // present in all 3 outputs
  } catch (var19) { list.push(list.shift()); }
})(call1, 211938);
...
function call1(var437){ var437="OCpXM;AofEk;tabIndex;…".split(";"); call1=function(){return var437}; return call1(); }
function get1(var350,…){ var350=var350-114; return call1()[var350]; }
```

`dce` already drops top-level bindings unused in the file (per
`dce-unused-toplevel-policy`), but two things keep this machinery live:
1. The rotation IIFE is an **expression statement with a side effect**
   (`list.push(list.shift())` on a local) — not recognized as pure-dead.
2. `var1 = get1` keeps `get1` (and transitively `call1`, the table) reachable.

## Recommended fix

After decoder-lift fully consumes an accessor (all reachable calls inlined), mark
the accessor + its backing table + the rotation IIFE that only mutates that table
as removable. Concretely, either:
- In `decoder_lift.rs`, emit a removal of the now-consumed accessor/table/rotation
  triple when the lift is complete; or
- Teach `dce`/`dead_store` that a self-contained rotation IIFE whose array **never
  escapes** is pure-dead, and that an accessor alias (`var1 = get1`) with no
  remaining real reads is droppable.

This removes the single largest chunk of residual noise (the ~280-entry table
string) across samples 5, 6, 8.

## Do NOT build

An **LZString-decompression pass.** The `g`/`h` functions in this family are an
LZString **encoder** — `h(input) → g(input,6,charsetFn)` compresses the *outgoing*
XHR body at runtime (`var239.send(...apTxFHuh(JSON.stringify(payload)))`,
`out/sample_5.out.js:218`). There are **no compressed string literals to decode**.
The `g`/`h` block is live and should be preserved (it is already readable).

## Related

- Issue 06 (sample_5 `fn12` — a local accessor alias that survives because the lift
  doesn't chase `var442 = var1`; complementary to this).
- Issue 01 (the `split("|")` + `switch` flattening dispatchers in this family — 39
  in sample_8 — remain; separate problem).
