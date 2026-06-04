# Deobfuscator — Issues

Issues found during the 2026-06 multi-sample audit. Each file documents one
concrete gap: the symptom, the root cause (with `file:line`), evidence, and a
recommended fix. **Most are now resolved or settled** — each file's own STATUS
block is authoritative; the table below is the current rollup.

## Roadmap (status as of 2026-06)

| # | Issue | Status |
|---|-------|--------|
| [13](13-mutated-array-length-fold.md) | Mutated-array `.length`/element folds to empty-literal value | ✅ **Fixed** — `KNOWN_BUGS` now empty; all 6 corpus cases pass |
| [01](01-unflatten-too-narrow.md) | unflatten precondition too narrow | ✅ Substantially implemented (CFG unflatten, param-state, string-order); bytecode VMs out of scope |
| [03](03-decoder-lift-globals-allowlist.md) | decoder-lift GLOBALS / Base91 chain | ✅ Resolved (value-binding lift; 1,579/1,611 picks fold). Remaining s7 `this`/`Function` idiom = **sound decline** (high-effort core change) |
| [04](04-decoder-lift-nested-scope.md) | decoder-lift only scans top-level scope | ✅ Implemented (scoped lifting at every depth) |
| [05](05-dead-rotation-machinery-dce.md) | Dead rotation/table machinery not removed | ✅ Nested spent-rotation DCE landed; s5 `fn12` case stays declined (issue 06) |
| [06](06-accessor-alias-follow.md) | Local accessor alias not followed | ✅ General `.bind()` alias-follow landed; s5 sequence-expr alias = **sound decline** |
| [07](07-helper-fingerprint-pass.md) | No compiler-helper fingerprint/rename pass | ✅ Marker-based helper-rename slice implemented |
| [09](09-evalvalue-array-materialization.md) | Sandbox `EvalValue` scalar-only | ✅ Implemented (`EvalValue::Arr`, size-capped). Full sample_9 VM out of scope |
| [02](02-demultiplex-slot-gate.md) | Demultiplex `make_slot` gate too tight | ⛔ **Tried & reverted** — widening is a 2.4–6× regression on sample_10; `single_return` gate is load-bearing |
| [10](10-mba-junk-simplification.md) | MBA predicates / junk guards not folded | ⛔ Deferred — needs a real MBA simplifier, not a peephole |
| [11](11-deterministic-table-resolve.md) | Deterministic state-key table (`var403`) | ⛔ **Sound decline** — member-written table; guarded by must-decline tests |
| [12](12-p1-premise-mismatch-rediagnosis.md) | P1/P2 CFG-recovery premise mismatch | ⛔ **Sound decline** shipped; ordering-widening too soundness-delicate |
| [08](08-async-state-machine-lift.md) | tslib `__generator`/`__awaiter` → async/await | 🚧 **Open** — the one substantive remaining feature. High effort, **highest soundness risk** (full `[op,value]` ABI → structured await/try-catch). Needs explicit scoping + equivalence-harness gating before starting |

The codebase has worked through this backlog under a strict **soundness-over-
coverage** invariant: where a widening could not be proven semantics-preserving it
was deliberately declined (and pinned by an adversarial must-decline test) rather
than shipped. The only untouched feature is #08.

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
