# 08 — tslib `__generator` / `__awaiter` state machines not lifted to async/await

- **Samples:** 3 (SWC bundle)
- **Impact:** High (biggest single readability win on sample_3) · **Effort:** High · **Risk:** High
- **Pass:** new lifter (depends on issue 07 for helper recognition)

## Symptom

Real `async` functions in sample_3 are left as raw switch-on-`.label` state machines
using the tslib `[op, value]` generator ABI — ~40 opaque lines per function.

## Evidence

```js
// out/sample_3.out.js:1425-1469  — one `await fetch()` inside try/catch
return fn2(function() {                       // fn2 = _async_to_generator / __awaiter
  return fn14(this, function(var492) {        // fn14 = tslib __generator
    switch (var492.label) {
      case 0: ...; var492.label = 1;
      case 1: var492.trys.push([1, 3, , 4]);
              return [4, fetch(var489, {...})];   // = await fetch(...)
      case 2: var490 = var492.sent();             // = the awaited result
              ...; return [3, 4];
      case 3: var491 = var492.sent(); ...         // = catch block
      case 4: return [2];                         // = return
    }
  });
})();
```

Decoded, this is:

```js
async function f() {
  try { const r = await fetch(var489, {...}); ... }
  catch (e) { ... }
}
```

`unflatten` does **not** handle this — it targets generic `while/switch`
dispatchers, not the tslib `[op, value]` ABI with `.label`/`.sent()`/`.trys`.

## Recommended fix

A dedicated lifter that recognizes the `__generator` step protocol (gated on issue
07 having identified `fn2`=`__awaiter`, `fn14`=`__generator`) and reconstructs:
- `case N:` blocks → straight-line statements,
- `return [4, expr]` + following `.sent()` → `await expr`,
- `.trys.push([try, catch, finally, end])` → `try/catch/finally`,
- `return [2, v]` → `return v`; `return [3, n]` → control-flow goto to label n,
- wrap in `async function`.

Reconstructing the `[op, value]` ABI back into structured `await`/try-catch is a CFG
problem — highest readability payoff but the riskiest pass to keep
semantics-preserving. **Gate behind the equivalence harness.**

## Related

- Issue 07 (prerequisite — helper recognition seeds this).
- Issue 01 (a CFG-based unflattener built on `src/ir/` could share machinery with
  this lifter's control-flow reconstruction).
