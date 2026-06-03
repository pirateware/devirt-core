# 07 — No compiler-helper fingerprint/rename pass

- **Samples:** 3 (SWC bundle) — generalizes to **every** Babel/SWC/tslib-compiled bundle
- **Impact:** High · **Effort:** Medium · **Risk:** Low (rename-only, sound by construction)
- **Pass:** new `src/passes/helper_fingerprint.rs`; interacts with `src/passes/rename.rs`

> **STATUS (2026-06): MARKER-BASED SLICE IMPLEMENTED.** Rather than a separate
> pass, the high-confidence recognizer was integrated into `rename.rs`'s `Collector`
> (`helper_marker_name` / `canonical_helper_name`): when a function/var-bound
> function carries a `"@swc/helpers - X"` or `"@babel/helpers - X"` directive
> prologue, it seeds a canonical hint (`instanceOf`, `typeOf`, …) that wins over the
> body-shape heuristic. Sound (rename-only), idempotent (canonical names aren't
> opaque), generalizes to every swc/babel bundle.
>
> **Result on sample_3:** `pick` → `instanceOf`, `pick1` → `typeOf` (18 call sites
> each) — fixing the two *actively-wrong* names the audit flagged. Also covers the
> 8 `@babel/helpers - typeof` markers in other samples. Full suite green.
>
> **NOT done (the larger, fragile part):** the **structural** fingerprint library
> for the ~13 *unmarked* helpers (`_async_to_generator`, `_sliced_to_array`,
> `_object_spread`, …). Only `instanceof`/`typeof` ship a directive marker; the
> rest need α-normalized AST matching against a canonical library that is brittle
> across swc versions. Left as a follow-up — see the inventory table below.

## Symptom

sample_3 is the cleanest real sample (no string/decoder indirection left), but its
entire ~197-line prologue is a stock `@swc/helpers` + tslib runtime where every
helper carries a generic or **actively wrong** name. Readers see `pick(x, Foo)`
instead of `x instanceof Foo`.

## Root cause

`rename.rs` names functions by **body shape** heuristics (ternary → `pick` at L278,
`||` → `or` at L262), with no library of canonical compiler helpers. Two helpers
even keep their self-identifying `@swc/helpers` string marker yet are mis-named:

```js
function pick(var22, var23) {            // out/sample_3.out.js:36 — really _instanceof
  "@swc/helpers - instanceof";
  return var23 != null && typeof Symbol != "undefined" && var23[Symbol.hasInstance]
    ? !!var23[Symbol.hasInstance](var22) : var22 instanceof var23;
}
function pick1(var47) {                  // out:111 — really _type_of
  "@swc/helpers - typeof";
  return var47 && typeof Symbol != "undefined" && var47.constructor === Symbol ? "symbol" : typeof var47;
}
```

## Helper inventory (sample_3, lines 1-197 — all recognizable)

| Current | Line | Canonical helper | Calls |
|---|---|---|---|
| `fn1` | 3 | `asyncGeneratorStep` | — |
| `fn2` | 12 | `_async_to_generator` / `__awaiter` | 2 |
| `fn5` | 27 | `_define_property` | — |
| `pick` | 36 | `_instanceof` | 18 |
| `fn6` | 40 | `_object_spread` | 6 |
| `fn7` | 52 | `ownKeys` | — |
| `fn8` | 63 | `_object_spread_props` | 3 |
| `fn9` | 70 | `_array_with_holes` | — |
| `fn10` | 73 | `_iterable_to_array_limit` | — |
| `fn11` | 92 | `_non_iterable_rest` | — |
| `fn12` | 95 | `_array_like_to_array` | — |
| `fn13` | 100 | `_unsupported_iterable_to_array` | — |
| `or1` | 108 | `_sliced_to_array` | 4 |
| `pick1` | 111 | `_type_of` | 18 |
| `fn14` | 115 | `__generator` (tslib) | 2 |

15 distinct helpers, ~75+ call sites.

## Recommended fix

Add a new pass `src/passes/helper_fingerprint.rs`:
- Build a small canonical library of the ~15 swc/tslib/babel helpers (bodies are
  byte-stable across versions modulo identifiers).
- α-normalize candidate function-decl bodies (rename identifiers to positional
  placeholders) and structurally match against the library.
- Use the embedded `"@swc/helpers - X"` string markers as free high-confidence
  ground-truth anchors.
- Rename matched decls to canonical names **before** `rename-by-role` runs.

Pure structural matching — no sandbox execution, so it is sound by construction
(rename-only, never rewrites behavior), unlike `math_fingerprint.rs`. Canonical
names (`asyncToGenerator`, …) have ≥3 non-`_$` chars so `is_opaque` (`rename.rs:144`)
rejects them, preserving idempotence.

**Slot:** prefer a dedicated pass over seeding `rename.rs`'s `hints` map — keeps the
already-large rename collector clean and lets canonical helpers benefit DCE too.

## Why it generalizes

Every SWC/Babel/tslib-compiled bundle ships this same fixed helper set with
identical bodies. One canonical library covers sample_3 and any future real-app
sample, and corrects the `pick`→`instanceof`, `pick1`→`typeOf`, `or1`→`slicedToArray`
mis-namings.

## Related

- Issue 08 (recognizing `fn2`=`__awaiter` and `fn14`=`__generator` here is the
  prerequisite for the async-state-machine lifter).
- Cheap interim: suppress the ternary→`pick` (L278) and `||`→`or` (L262) heuristics
  for functions carrying an `@swc/helpers` marker, to stop emitting wrong names.
