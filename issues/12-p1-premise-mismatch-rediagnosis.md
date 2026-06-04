# 12 — P1 const-resolution premise mismatches the headline samples (re-diagnosis)

**Status: re-diagnosed (P0/S2). The S1 refactor stands; the S2 ordering-widening
as planned does NOT fire on s1/s4/s11, for the reasons below. Recorded per P0 §8
(route around a measured dead-end; keep the branch green and monotonic).**

## What the plan assumed

P1 (`plans/P1-whole-program-const-resolution.md`) assumed the dispatchers of
samples 1/4/11 stall because their `case <label>:` are **symbolic single-symbol
constants** written in a *setup function* called once at module init, and that a
widened happens-before proof (nested IIFE / `.call` / idempotency-guard) would
resolve them to literals, unblocking the unflatteners.

## What the samples actually are (measured on `out/`)

| sample | case labels | real blocker |
|--------|-------------|--------------|
| **s1** | 465 numeric, 137 symbolic (`case var453:`) | the symbolic ones are written in `fn19()`, which runs **inside a constructor** `var2574` invoked via `new var2574()` (lines 421/21876/22528) — a *VM-in-a-constructor*. The constants are (idempotently) re-initialized on every `new`. This is **not** a module-setup IIFE; GCI's setup-ordering model cannot prove the happens-before soundly. |
| **s4** | **235 numeric, 0 symbolic** | P1 is **irrelevant** — labels are already literals. The dispatcher is `switch (var7 + var4)` with a **computed, stateful discriminant** (`var7 = fn21(new Number(91), …)`, `var6.set(var4)`), plus **prefix statements before the loop**. Neither the linear nor the param-state unflattener models a computed-state discriminant. |
| **s11** | labels are `case var401[58][4]:` | **2D const-array index**, not a symbol. `var401 = arr14[7]` is an *alias* of a nested const array; resolving a label needs deep cross-function const-aggregate folding of `arr14[7][i][j]` — and the tables are large (~250 elements/row), so naive materialization risks bloat (cf. issue 09). Tracked already by issue 11. |

## Conclusion

The single premise — "symbolic single-symbol labels resolvable by a *setup-ordering*
widening" — holds for **none** of s1/s4/s11. Each needs a distinct mechanism:

- s1 → a **constructor-init** happens-before proof (constants written in a ctor,
  read only through instances ⇒ every read is preceded by construction). Novel,
  soundness-delicate; out of scope for a bounded sound widening here.
- s4 → **computed-state CFG recovery** (P2 generalized to a non-identifier
  discriminant + a stateful coercion object) and prefix-statement tolerance.
- s11 → **deterministic const-table resolution** (issue 11) with a bloat cap.

## What was kept (the sound deliverable)

`P0/S1` landed: the ordering proof was extracted to `src/ir/ordering.rs`
(`OrderingOracle`) and the census+analysis to `src/ir/consts.rs`
(`ConstResolver`), with `global_const_inline` reimplemented as a thin
materialization client. Byte-parity verified (modulo pre-existing pipeline
non-determinism on s3/s11); 174 tests green. `ConstResolver::resolve_int` is the
reusable engine the unflatteners consult (P0/S3) — it correctly resolves the
genuinely-setup-scoped symbolic labels (validated by the
`unflatten_generalized_shapes` symbolic-label equivalence case), it just doesn't
reach the three headline patterns above.

## Routing decision (P0 §8)

Effort routed to where sound, equivalence-validated progress is reachable:
- **S3**: wire `resolve_int` into the unflatteners (sound; resolver only widens).
- **P2**: recover the **conditional-transition / back-edge** dispatchers the
  linear `simulate` declines today (`unflatten_declines_unsound_shapes`), proven
  by the differential equivalence battery on synthetic branch/loop/nested
  dispatchers — a new capability that does not depend on the headline-sample
  const-resolution above.

## STATUS (ws-b-resolution / B1) — wall confirmed precisely; sound DECLINE shipped

Refined diagnosis of s1's label-init ordering. The real shape is:

```text
// deep inside a top-level flattened dispatcher var3=fn37(...), in `case 772:`
var2574 = (function FTQAbjUkpF() {
   fn22(); fn31(); fn19();              // fn19() writes the labels var43x..var47x
   var var431..var476;                  // the label bindings
   var dispatch = function(...) { ... switch(s){ case var475: ... } };  // reads
   ...
   return SomeCtor;
})();                                    // IIFE, but NESTED ~6 levels, in a case
```

So `var2574 = (IIFE)()` *is* an IIFE-init, but the IIFE is **not a top-level
statement** — it lives inside `case 772:` of an outer flattened dispatcher. The
`OrderingOracle` only registers regions for the program body and *top-level* IIFE
statements (`iife_region` scans `program.body`), so `fn19();` is never found as a
bare init-call in a recognized region → `writes_before_all_reads` returns false
(verified by instrumentation: every per-symbol gate passes — single write,
unconditional, writer called once, 6 reads — only the region/ordering proof
declines). That is the entire reason GCI reports `inlined=0` of `1083` on that
round.

**Why a sound widening is not landed.** Registering the nested IIFE as a setup
region would require proving the IIFE *runs unconditionally* — but it runs only
when the outer dispatcher's state reaches `772`, a fact that depends on the outer
state machine's reachability, which is exactly the kind of conditional ordering we
cannot statically prove without risking an unsound inline (a label read on a
sibling case can execute before `fn19()` writes it). Per the task's hard
invariant, B1 ships a **sound DECLINE** rather than an unsound inline.

**Adversarial tests locking in the sound decline** (a constant written by an init
that runs inside a conditionally-reached nested dispatcher case must stay symbolic
— a read on an earlier case observes the pre-init `undefined`):
- `conditional_nested_init_constant_stays_symbolic` (`tests/phase1.rs`)
- `conditional_nested_region_init_is_not_inlined` (`tests/equivalence.rs`)

**To actually resolve s1 (future work):** a constructor/nested-setup
happens-before proof that (a) recognizes a nested IIFE/ctor region reached
*unconditionally* from a proven-setup parent and (b) proves no label-reading
function is reachable before the nested init completes — i.e. model the outer
flattened dispatcher's guaranteed execution order. This is the novel,
soundness-delicate mechanism issue 12 flagged as out of scope for a bounded sound
widening, and it remains so.
