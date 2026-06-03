# 03 — decoder-lift GLOBALS allowlist false-negative (sample_7 Base91 chain)

- **Samples:** 7 (HUMAN / PerimeterX)
- **Impact:** Very high · **Effort:** ~~Low~~ **High** (re-scoped, see Status) · **Risk:** Low
- **Pass:** `src/passes/decoder_lift.rs` (GLOBALS `:39-94`); sandbox `src/sandbox.rs`

> **STATUS (2026-06):** The GLOBALS allowlist additions (`globalThis`, `window`,
> `global`, `self`) are **implemented** — they are correct and safe, but they do
> **not** unblock sample_7 on their own. Empirically the lift count and `varNNN`
> count are unchanged. The agent's "just add 4 globals" estimate was incomplete.
> Two further blockers, both architectural and **beyond medium effort**, remain:
>
> 1. **`or1` is statically impure** — its body ends `return var6 || this` (the
>    `this` sets `FreeVars.impure`, rejected at `decoder_lift.rs:802`) and
>    references `new Function("return this")()` (`Function` is deliberately
>    excluded from GLOBALS as an injection vector). Admitting it requires either a
>    "plain function decl, `this` is global/undefined" relaxation or recognizing
>    the exact global-detection idiom.
> 2. **No `var = pureCall()` preamble model** — `pick1` depends on
>    `var9 = or1()||{}`, `var10 = var9.TextDecoder`, … `var15 = (IIFE)()`. These
>    are vars bound to *runtime computations*, which the preamble builder does not
>    model (`statement_is_pure` only emits pure-**data** vars; `classify` only
>    admits pure-data vars and pure functions). Lifting sample_7 needs the
>    preamble to also carry value-bindings proven pure-by-evaluation.
>
> Recommend moving the **full sample_7 unblock** to the high-effort bucket
> alongside issue 01. The GLOBALS widening stays as a standalone correctness
> improvement for guarded detection idioms in other inputs.

## Symptom

sample_7 is flagged "out of scope" in `memory/` due to "encrypted string keys" and
a host-coupled decoder. **This is wrong.** The Base91 `arr1` string table is plain
(not encrypted), pure, deterministic, and runnable in the existing sandbox. Yet
**zero** of the 1,763 `pick(n)`/`pick2(n)` decoder call sites are lifted, leaving a
1,268-entry table (`out/sample_7.out.js:27-1294`) and ~90 Base91 `callN` functions
as opaque scaffolding.

## Root cause

The decode chain is fully self-contained:
`pick2(n)` (memo accessor over private cache `obj`) →
`call1(arr1[n])` (Base91 loop over a hard-coded 91-char alphabet) →
`pick1(bytes)` (UTF-8 byte→string).

`collect_private_caches` (`decoder_lift.rs:1051`) **correctly** recognizes the
`obj[k]===undefined?(obj[k]=call(arr1[k])):obj[k]` memo idiom (verified with
`DL_DEBUG=1`: ~116-126 functions classify as pure). The memo logic is not the
blocker.

The real blocker is upstream purity poisoning. `pick1` (`out:1334`) depends on
`var9 = or1()` (`out:1322`), and `or1` (`out:1296`) detects the global object via
`globalThis`, `global`, `window`, and `new Function("return this")()`. **None of
those four names are in the `GLOBALS` allowlist** (`decoder_lift.rs:39-94`);
`Function` is deliberately excluded as an injection vector. So `or1` is classified
impure, which transitively poisons `var9 → var10/11/12/15 → pick1 → call/call1 →
pick/pick2`. Nothing reaches `callable`, so nothing lifts.

This is a **static-classifier false-negative, not real impurity.** The QuickJS
sandbox is `Context::full` (`sandbox.rs:51`) and already has `globalThis`,
`Function`, `String`, `Array` — `or1` runs fine there. All three `pick1` branches
(TextDecoder / Buffer / pure `var15` fallback, `out:1322-1333`) are byte-for-byte
equivalent UTF-8 decoders including on multibyte input, so whichever branch the
sandbox takes yields the host-identical string. The lift is provably
semantics-preserving.

## Evidence

Extracted and ran the chain standalone in node:
- `call(arr1[0])` → `"DA2i3kpR5zIUP"`
- `call(arr1[1])` → `"txxAPq7TWFsNGw3GrGQYrmSjX45zrV83o"`

Clean ASCII identifiers — plain Base91, not encrypted.

## Recommended fix

Admit the global-object-detection idiom to the purity gate:
- Add `globalThis`, `window`, `global`, `self` to `GLOBALS` (`decoder_lift.rs:39`) —
  read-only globals present in QuickJS.
- For `new Function("return this")()`, either whitelist that exact zero-arg
  `return this` form, or rely on `or1`'s `try/catch` fallback landing on
  `globalThis`.

The double-eval determinism guard (`decoder_lift.rs:1809-1812`) already protects
against anything non-reproducible, so this stays sound.

## Expected payoff

1,763 decoder call sites (1,112 distinct accessor/index pairs; 1,037 distinct
`pick2(N)` alone) collapse to string literals. Kills the 1,268-entry `arr1` table
(~1,268 lines), ~90 `callN` Base91 functions, and their accessors as dead
scaffolding — well over 1,500 lines removed and a large fraction of the 4,597
`varNNN` temps eliminated. Turns opaque `pick2(1003)` into readable property names.

## Scope reclassification

sample_7 is **mis-classified as fully out-of-scope.** Split it:
- **Decodable now** with this gate change — the entire Base91 `arr1` string table.
- **Genuinely host-coupled, stays out of scope** — the sensor logic from
  `out:1637` onward (`window`/`document`/`btoa`/`navigator`) and the `call11`/`call12`
  MD5-HMAC keying (`out:1572-1606`; constants `909522486`/`1549556828` are the MD5
  ipad/opad). These are orthogonal to `arr1` — the table itself is not keyed.

The memory note's claim that "the decoder calls `fw()` which reaches host state" is
inaccurate: `fw` is just the per-scope memo accessor (source-name analog of the
renamed `pick`/`pick2`), not a host call.

## Related

- Issue 04 (nested-scope lifting — complementary decoder-lift improvement).
- Issue 09 (the same scalar-only `EvalValue` limitation that blocks sample_9).
