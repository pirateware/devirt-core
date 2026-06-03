# Deobfuscator — Open Issues

Issues found during the 2026-06 multi-sample audit. Each file documents one
concrete gap: the symptom, the root cause (with `file:line`), evidence, and a
recommended fix. Ranked by ROI (impact ÷ effort).

## Roadmap

| # | Issue | Samples | Impact | Effort | Risk |
|---|-------|---------|--------|--------|------|
| [02](02-demultiplex-slot-gate.md) | Demultiplex `make_slot` gate matches zero real handlers | 10 | **Very high** | Low | Low |
| [03](03-decoder-lift-globals-allowlist.md) | decoder-lift GLOBALS allowlist false-negative | 7 | **Very high** | Low | Low |
| [04](04-decoder-lift-nested-scope.md) | decoder-lift only scans top-level scope | 11, 7 | High | Medium | Low |
| [05](05-dead-rotation-machinery-dce.md) | Dead rotation/table machinery not removed | 5, 6, 8 | High | Medium | Low |
| [01](01-unflatten-too-narrow.md) | unflatten precondition too narrow (0 fires on real samples) | 1, 4, 8, 11 | **High** | High | Medium |
| [07](07-helper-fingerprint-pass.md) | No compiler-helper fingerprint/rename pass | 3 (+all bundles) | High | Medium | Low |
| [08](08-async-state-machine-lift.md) | tslib `__generator`/`__awaiter` not lifted to async/await | 3 | High | High | High |
| [06](06-accessor-alias-follow.md) | Local accessor alias not followed by decoder-lift | 5 | Medium | Medium | Low |
| [11](11-deterministic-table-resolve.md) | Deterministic state-key table (`var403`) not resolved | 11 | Medium | High | Medium |
| [10](10-mba-junk-simplification.md) | MBA predicates / junk guards not folded | 11, 10 | Medium | Medium | Low |
| [09](09-evalvalue-array-materialization.md) | Sandbox `EvalValue` scalar-only blocks array results | 9 | Low (partial) | Medium | Low |

## Scope reclassifications

These overturn current `memory/` notes and are tracked in the relevant issue:

- **sample_7** (issue [03](03-decoder-lift-globals-allowlist.md)) — the Base91 `arr1`
  string table is pure, deterministic, and sandbox-runnable *today*; it is blocked
  only by the GLOBALS allowlist, not by genuine host coupling. Only the post-line-1637
  sensor + MD5-HMAC layer stays out of scope. **Not fully out of scope.**
- **sample_9** (issue [09](09-evalvalue-array-materialization.md)) — full VM lifting
  stays out of scope (research-grade, ~84 general handlers). But the pure `var1`
  varint unpacker is a deferred low-effort materialization win, blocked only on
  array-valued sandbox results.

## Myths busted during the audit (do NOT build these)

- **Cloudflare LZString is an *encoder*, not a decoder.** The `g`/`h` functions
  compress the *outgoing* XHR body at runtime; there are no compressed string
  literals to decode. The rotated string tables are *already* inlined. See issue
  [05](05-dead-rotation-machinery-dce.md).
- **sample_11 is not a webpack bundle.** `obj` is just the export accumulator
  (`obj={}` … `return obj`), there is no `obj[id]=function(module,exports,require)`
  module registry. No unbundle pass is warranted.
