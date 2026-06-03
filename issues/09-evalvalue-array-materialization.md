# 09 — Sandbox `EvalValue` is scalar-only; blocks array/object decoder results

- **Samples:** 9 (custom bytecode VM) — also any decoder returning an array/object
- **Impact:** ~~Low (partial)~~ **Net-negative on its only target** · **Effort:** Medium · **Risk:** Low
- **Pass:** `src/sandbox.rs` (`EvalValue` `:33-37`), `src/passes/decoder_lift.rs` (splice `:1821-1830`)

> **STATUS (2026-06): DEFERRED — would bloat sample_9, the only beneficiary.** The
> diagnosis is correct (the `var1` unpacker is pure and blocked only by scalar-only
> `EvalValue`), but materializing the result is **net-negative**: `list2` is the
> VM's bytecode table, decoded from a **~131 KB** packed string into tens of
> thousands of integers. As an array literal `[8,12,5,…]` that is comparable-to-
> larger than the packed string, so the output **grows**, and the VM still
> interprets it (full lift stays out of scope). Replacing a compact packed string
> with a giant int-array literal is not a readability or size win. No other current
> sample has an array-returning decoder that would benefit. Adding an `EvalValue`
> variant + array-splice path also widens the sandbox/equivalence-test surface for
> zero positive payoff today. Revisit only if a future sample has a *small*
> array-returning decoder, and gate any array splice on a tight size limit so it
> can never bloat.

## Symptom

sample_9's data unpacker `var1(...)` is fully pure and trivially sandbox-runnable,
but decoder-lift refuses it: the sandbox value model has no array representation, so
an array-returning decoder cannot be materialized.

## Root cause

```rust
// src/sandbox.rs:33-37  — scalar only
enum EvalValue { Str(String), Num(f64), Bool(bool) }
```

`decoder_lift.rs:1821-1830` only splices `Str`/`Num`/`Bool` back into the AST. The
unpacker returns an **integer array**, which has no `EvalValue` representation, so
the lift's own "execution is the validator" gate rejects it — even though purity is
not the problem.

```js
// out/sample_9.out.js:3-17  — pure varint unpacker (LEB-style)
var var1 = function(list, list1, var2) { ... return arr; };   // returns int[]
var list2 = var1("8>>q>X…" /*131 KB packed*/, ">NJqHXBxhoeF…" /*alphabet*/, 40);
```

All three args are literals; body uses only `Map`/`Array`/arithmetic (on the GLOBALS
allow-list). No free vars, no host access. The file's only host couplings
(`window`/`Math.random`, lines 26-29) feed the *running* VM, not the data decode.

## Recommended fix

Extend `EvalValue` with an array variant (`Arr(Vec<EvalValue>)`, or a dedicated
numeric-array fast path), run `var1(...)` in the sandbox, and emit
`list2 = [ …integers… ]` replacing the `var1` call + the 131 KB packed string.
Mirror the splice in `decoder_lift.rs:1821-1830`.

## Scope: this is a PARTIAL win only

- **Available now (this fix):** materialize the bytecode — deletes the 131 KB packed
  string and the varint decoder, exposing the raw opcode stream as a literal int
  array. A legitimate normalization win, independent of ever interpreting the VM.
- **Out of scope (research-grade):** full VM lifting. sample_9 is a genuine
  register/closure bytecode interpreter — ~84 general handlers
  (`out/sample_9.out.js:118-476`) covering the full JS instruction set, an embedded
  regenerator runtime (`var499`, lines 480-793), exception unwinding, and a
  Proxy-based opcode-index permutation (lines 462-476). Lifting it to source is a
  bespoke custom-VM decompiler, not a pass.

**Memory update:** reclassify sample_9 from flat "VM / out of scope" to: *full VM
lift out of scope (research); pure varint unpacker `var1` is a deferred low-effort
materialization win blocked only on array-valued sandbox results.*

## Related

- Issue 03 (the other decoder-lift false-negative — different cause, same sample
  family of "pure-but-not-lifted").
