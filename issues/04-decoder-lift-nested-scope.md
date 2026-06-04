# 04 — decoder-lift only scans top-level scope (misses nested decoders)

- **Samples:** 11 (DataDome), 7 (HUMAN/PerimeterX — secondary)
- **Impact:** High · **Effort:** Medium · **Risk:** Low
- **Pass:** `src/passes/decoder_lift.rs`, `src/sandbox.rs`

> **STATUS (2026-06): IMPLEMENTED.** The diagnosis was partly wrong: the
> decoder-lift architecture *already* handles nested scopes — `collect_scoped_funcs`
> collects functions at any depth and `scan_invariants` collects pure-data tables
> "anywhere in the program" (nested `arr15` already reached the preamble via
> `local_data`). The **actual** sole blocker was that `call3 = atob(arr15[i])`
> references `atob`, which was neither in `GLOBALS` nor in the QuickJS sandbox
> (base64 is a WHATWG API, not ECMAScript). Fix:
> 1. Added a spec-correct `atob`/`btoa` polyfill to the sandbox (`SANDBOX_PRELUDE`
>    in `src/sandbox.rs`, installed once per context, only if absent).
> 2. Added `"atob"`, `"btoa"` to `GLOBALS` in `decoder_lift.rs`.
>
> **Result on sample_11:** `call3` call sites **553 → 13**; `win[call3(0)]` →
> `win.navigator`, `win.Object.defineProperty`, `win.navigator.plugins`, etc. The
> 13 survivors are genuine runtime *encoders* with dynamic args
> (`call3("InnerErr: " + e.message)`) — correctly left. Full test suite green
> (equivalence + idempotency). Byte size barely moves (decoded names ≈ same length
> as `call3(N)`); the win is **readability**. `fn40` has 4 dynamic-arg survivors.
>
> **C1 RE-VERIFICATION (2026-06, branch `ws-c-decoders`):** confirmed sample_11
> nested-scope lifting is **complete** — there are **zero literal-arg survivors**
> for `call3`/`fn40`/`fn55`; every remaining survivor (13 `call3` + 4 `fn40` +
> 1 `fn55`) has a *dynamic runtime argument* and is a true encoder, correctly
> preserved. The s7 secondary target is host-coupled (`this`/`Function` global
> detector) and is a documented sound decline — see issue 03's STATUS UPDATE.

## Symptom

sample_11's top-level modules are already clean, but the entire anti-bot core (one
giant nested IIFE, `out/sample_11.out.js:~1075-4600`) keeps two un-decoded string
decoders driving **565 call sites**.

## Root cause

decoder-lift only discovers pure bindings via free-var analysis over `Program`
**top-level** statements. The DataDome decoders are **nested locals** inside the
anti-bot IIFE, so they are never collected:

```js
// out/sample_11.out.js:4055  — nested inside the IIFE
function call3(var950, var951) { return atob(arr15[var950]); }
// arr15 = ["TnVtYmVy"=>"Number", "YnRvYQ"=>"btoa", ...]   plain base64
function fn40(...) { ... }   // out:4058 — custom-alphabet base64, arr16
```

- `call3` — pure `atob` over a literal table. **553 call sites**
  (`call3(34)`×61, `call3(4)`×54, `call3(0)`×38). `win[call3(2)]` is `win.Object`, etc.
- `fn40` + `arr16` — custom-alphabet base64, alphabet `"H1DAxCvrj7IaPRL8…"`,
  `arr16` mixes decoy numbers (`546`, `842.73`) with encoded strings. **12 sites.**

Both are trivially evaluable; they are simply outside the scan window.

## Recommended fix

Generalize the "pure binding + pure-callee call" detection in `decoder_lift.rs`
from `Program` top-level to **any function scope**: when entering a function/IIFE
body, collect its sibling pure bindings (`arr15` + the `atob`-based `call3`;
`fn40` + `arr16`) and lift calls whose arguments are literals. The sandbox already
validates by execution, so this stays in-charter for the existing pass — it is a
scan-scope widening, not a new capability.

This must run **before** any attempt to unflatten sample_11's dispatchers
(issue 01/11): the `call3`-decoded names feed the surrounding logic.

## Expected payoff

553 `call3()` + 12 `fn40()` sites collapse to literals — the bulk of the visual
noise in the sample_11 anti-bot core.

## Related

- Issue 03 (the GLOBALS allowlist gap — a different decoder-lift false-negative; a
  nested-scope sample_7 decoder would hit both).
- Issue 11 (resolve `var403` so the dispatchers can then be unflattened).
- Issue 01 (unflatten — downstream of this on sample_11).
