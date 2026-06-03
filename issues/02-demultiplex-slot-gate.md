# 02 — Demultiplex `make_slot` gate matches zero real handlers

- **Samples:** 10 (reCAPTCHA runtime — the worst residual sample, 16,055 distinct temps)
- **Impact:** ~~Very high~~ **Negative (reverted)** · **Effort:** Low · **Risk:** Low
- **Pass:** `src/passes/demultiplex.rs`

> **STATUS (2026-06): TRIED AND REVERTED — the widening is a net regression.**
> Dropping the `single_return` gate (accepting `[effect; return]` / `switch` /
> multi-statement bodies, with a soft statement cap) was implemented and measured
> on sample_10:
>
> | gate | output bytes | distinct `varNNN` | `__demux_*` emitted |
> |------|-------------|-------------------|---------------------|
> | baseline (`single_return`) | 1,675,367 | 16,055 | (trivial slots only) |
> | widened, cap 24 stmts | **10,059,459** (6.0×) | 16,931 | 4,776 |
> | widened, cap 2 stmts | **3,950,428** (2.4×) | 16,275 | 1,933 |
>
> **Why the agent's analysis was wrong in practice:** binding the selector *does*
> fold the dispatch guard (`(25 | (false && …, 24)) == 25 && …`), but the reСAPTCHA
> handlers do real, non-foldable work (DOM queries, XHR `send`, cross-handler
> calls). Removing the guard leaves the large live body intact, and that body is
> **duplicated once per distinct selector** — e.g. `__demux_Bo_6_25` and
> `__demux_Bo_6_26` are near-identical full DOM-query handlers. Only the original
> trivial `single_return` slots fold to nothing; for every other shape,
> specialization multiplies code. The `single_return` heuristic was load-bearing,
> not incidental. **Reverted to baseline.**
>
> A real win for sample_10 would require a different lever (call-site inline + fold
> + prove the whole array dead and delete it — net-negative here since the inlined
> bodies exceed the array), i.e. this sample is closer to irreducible than the
> audit assumed. Deprioritized.

## Symptom

sample_10 is a **function-array VM dispatcher** obfuscation (obfuscator.io /
BotGuard class) layered on Closure minification. `demultiplex` exists for exactly
this pattern but reports **`("demultiplex", 0)`** — zero specializations. The
sample keeps 16,055 distinct `varNNN` temps (~5.3 uses each) and 86 switches.

## Root cause

The handler arrays are collected fine; the immutability gate
(`collect_immutable_fn_arrays`) is satisfied. The blocker is one over-tight body
shape check in `make_slot` (`demultiplex.rs:288-300`):

```rust
let single_return = body.statements.len() == 1
    && matches!(&body.statements[0],
        Statement::ReturnStatement(r) if r.argument.is_some());
if !single_return || references_arguments(body) { return None; }
```

Handler-body shape census (356 handlers scanned, representative):

| shape | count | accepted today? |
|---|---|---|
| single `return` | **0** | ✅ (matches nothing) |
| `[effect-stmt; return]` | 48 | ❌ rejected (2 statements) |
| `>2` statements | 76 | ❌ rejected |
| leading `switch`/`if` | 232 | ❌ rejected |

The accepted shape occurs **zero times**. Every real handler is
`[guarded-effect; return arg]` or `switch(selector){…}`.

## Evidence

```js
// out/sample_10.out.js — handler array (one of ~12)
var4208 = (function() { return [
  function(var3074, var3075, var3076, var3077, var3078) {
    ((var3074 | 2) & 5) == 1 && (call73.call(this, var11463, var3075), this.size = var3076);
    return var3077;                       // var3074 is the constant SELECTOR
  }, ... ]; })();
// 11,276 call sites:  var4208[36](34, ...)  var4208[36](10, ...)  var4208[36](9, ...)
```

- 11,410 dispatch call sites `arr[N](...)`; **11,276 (98.8%)** have a constant
  selector `arr[N](C, …)`.
- 2,708 opaque arithmetic predicates gate the real op. Once the selector is bound
  to its constant, the predicate folds: `((2|2)&5)==1 → false` drops the effect,
  call reduces to `return arg3`; `((9|2)&5)==1 → true` keeps it. const-fold /
  flatten-else / DCE finish the job. Bound selector also collapses the 86
  `switch(selector)` to a single case.
- 63% of temps are multi-use **because** they are handler params / array names —
  inline-single-use, assigned-once, and ssa-opt are structurally unable to touch
  them. Demultiplexing is the **only** lever that moves this sample.

## Recommended fix

In `make_slot` (`demultiplex.rs:259-303`) accept multi-statement bodies of the form
`[leading-effect-statement(s); return <arg>]` and `switch(selector){…}` bodies, not
just `single_return`. The pass already specializes soundly by prepending
`var <sel> = C;` and dropping param 0; let const-fold / flatten-else / DCE collapse
the predicates on the bound bodies in subsequent fixpoint rounds.

Watch the cap: `MAX_SPECIALIZATIONS = 4000` (`demultiplex.rs:66`) — 11k call sites
will hit it. Dedupe by `(array, k, selector)` (the `spec_name` machinery at L459
already does this); the number of **distinct** `(k, C)` pairs is likely far below
11k. Raise the cap and/or tighten the dedupe.

## Notes / risk

- The apparent `var4208[…] =` element-write at `out/sample_10.out.js:22789` is a
  comma-expression false positive — the real assignment target is `var14990.g`,
  not the array — so the immutability gate is correct to keep the array collected.
- This is the single highest-leverage change for the worst sample. Renaming/inlining
  tuning is wasted effort here (issue: those temps are genuine multi-use).

## Related

- Issue 10 (fold the 2,708 opaque predicates once selectors are bound).
