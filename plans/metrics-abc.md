# Workstreams A/B/C — execution results

Autonomous multi-agent execution of MASTER-ROADMAP workstreams **A** (control-flow
recovery), **B** (constant/value resolution), **C** (decoder/string layer). Landed
on `main` as a fast-forward of `integration/abc`. Every step sound-by-construction,
gated by the QuickJS differential equivalence harness, P0 discipline throughout.

## Suite
- **254 tests green** (baseline 182; +72 new should-fire / must-decline / equivalence cases).
- `cargo clippy --all-targets` 0 warnings; `cargo fmt --check` clean.
- Whole-sample safety test (`tests/samples.rs`, idempotency + convergence over the
  corpus) green; every sample still `converged=true`.

## Per-sample metrics (baseline → after)

| sample | baseline out | after out | Δ bytes | switch | note |
|--------|-------------:|----------:|--------:|--------|------|
| s1  | 894310  | 887746  | **−6564** | 38 | B3 array-axis materialization |
| s4  | 130101  | 130282  | +181 | 5 | cosmetic: fewer vars (948→947)/lines, more array literals; equivalence-validated |
| s5  | 24133   | 22944   | **−1189** | 4→**1** | A1 string-array-order |
| s6  | 24849   | 23881   | **−968**  | 5→3 | A1 + C4 rotation cleanup |
| s8  | 250183  | 232913  | **−17270** | 40→**18** | A1 string-array-order (bonus) |
| s11 | 195463  | 195458  | −5 (noise band) | 37 | within pre-existing ±5 B nondeterminism |
| s7,s9,s10,s2,s3,s12,s13 | — | — | 0 | — | byte-identical |

No sample regressed (s4's +0.14% is a readability-positive var/line reduction, not bloat).
Stacking all passes produced **no combinatorial regression**.

## What landed (sound, equivalence-validated, green)

### Workstream A — control-flow recovery
- **A1 — string-array-order unflattener** (`unflatten_string_order.rs`, new). The
  Cloudflare `for(a="0|8|…".split("|"),i=0;true;)switch(a[i++])` shape → straight-line
  chain. Soundness gates: const+immutable order array, monotonic `a[i++]` cursor,
  `while(true)`, no `default`, dispatch-clean bodies. **Cracked s5 (switch 4→1), s6,
  and bonus s8 (switch 40→18).** 13 structural + 4 equivalence tests.
- **A2 — general relooper** (`ir/relooper.rs`, new). Reducible `RecoveredCfg` →
  nested `if/else` + `while(true)` with break/continue, via dominator-tree structured
  walk (Simple/Loop/Multiple). Reducibility check (colored DFS), declines irreducible.
  Subsumes the linear chain + self-loop as degenerate cases. 5 unit + 7 equivalence tests.
- **A3 — computed/object-state recognizer** (`computed_state.rs`, new). `switch(t+b)`
  with a pure constant-coercing object → normalized integer dispatcher. **s4's actual
  dispatcher declines soundly** (per-read *mutating* `valueOf` = stateful coercion
  stream; needs abstract interpretation of the transform — documented wall).
- **A5 — discriminant/transition generalizations** (`unflatten.rs`). `*=` transitions,
  fall-through/shared-tail cases, string/boolean discriminants, and `s = next[s]`
  immutable-lookup-table transitions. 18 new tests. Each with its must-decline sibling.

### Workstream B — constant/value resolution
- **B4 — fixpoint pure-expression resolution** (`ir/consts.rs`). `var b = a+1` resolves
  once `a` does, via a bounded const-lattice fixpoint evaluated in exact JS int32/f64
  semantics in Rust (total + deterministic — strictly safer than double-eval). 4 tests.
- **B2 — deterministic 2D/aliased const-array resolution.** Capability confirmed +
  regression-locked both directions (`T=base[k]; T[i][j]` resolves; member-written
  tables stay symbolic). **s11 declines soundly** (its `arr14` is built by member
  writes in async closures = mutable; needs issue-11 table evaluator).
- **B3 — value-axis: `EvalValue::Arr` + global-object idiom.** Small array-returning
  decoders materialize (size-capped, **s1 −6564**); the `this`/`Function("return this")`
  global idiom modeled as the frozen sandbox global where pure-relative-to-host.
  The `var v = pureCall()` preamble was implemented then **reverted** (regressed s1/s8/s11
  via classification-fixpoint reordering — P0: revert, don't patch). s7's Base91 tail
  stays host-coupled.

### Workstream C — decoder/string layer
- **C3 — accessor-alias follow.** Decoder-lift sees through `var g = dec.bind(args)`.
- **C1 — nested decoder lift.** Re-verified s11 already complete (all literal-arg
  decoders lifted; 18 survivors are dynamic runtime encoders, correctly preserved).
- **C4 — rotation/IIFE cleanup.** DCE of spent rotation machinery nested in init IIFEs
  (**s6 −237**).
- **C2 — memo/host-coupled handling.** s7 chain bottoms out in `var9.TextDecoder`/
  `var9.Buffer` host properties — pure parts lift, sensor-coupled tail declines + documented.

## Declined walls (out of scope for *sound bounded* widening — documented, not forced)

These require mechanisms the P0 invariant forbids approximating unsoundly; each ships
as a sound decline + adversarial test + issue note:

- **s4 main dispatcher** — stateful mutating-`valueOf` coercion stream (needs A3 + an
  abstract interpreter of the transform `cT`).
- **s1 label constants** — written in `fn19()` inside a constructor, nested ~6 levels
  in a flattened dispatcher case; needs nested-IIFE unconditional-reachability proof (B1).
- **s11 labels** — `var401[i][j]` over a member-write-built (mutable) table; needs a
  deterministic table evaluator with a bloat cap (B2/issue-11).
- **s7 Base91 table** — host-property-coupled tail; needs a sound host model (F5).

## Not attempted in this pass
- **A4 dispatcher-agnostic substrate** — the architectural unification (`DispatchModel`
  trait + one abstract interpreter). Best-effort isolated attempt launched separately,
  strictly gated to zero-regression-or-additive.
- Workstream **D** (bytecode VM, s8/s9/s10) and **E/F** — out of the A/B/C scope.
