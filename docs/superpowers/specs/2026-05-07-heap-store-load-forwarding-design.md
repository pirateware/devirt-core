# Heap Store-Load Forwarding — Design

**Date:** 2026-05-07
**Status:** approved, single ticket / two commits

## Goal

Resolve `LoadProp(obj, key)` and `LoadIndex(arr, Const(i))` against an
`AllocObject`/`AllocArray` whose relevant slot is populated by a
preceding `StoreProp`/`StoreIndex` with a known constant key — at
compile time, replacing the Load's result Value with the stored
Operand. Generic over input class: works equally on Babel object
literals, JsConfuser arrays, obfuscator.io string tables, hand-written
code. The pass operates on SSA shapes, blind to provenance.

`RefinePurityPass` (already landed) is the precision baseline:
unaliased allocations get their dependent heap accesses marked
`Purity::Pure`. Forwarding only touches Pure accesses — that's the
soundness gate.

## Scope: two commits, one ticket

### Part 1 — Intra-procedural forwarding (`ForwardLoadsPass`, FunctionPass)

Same-block heap-shape dataflow. Cross-block deferred to a follow-up
ticket if corpus shows wins.

**Algorithm (per block):**

1. Walk `block.instructions` in order, maintaining
   `HashMap<(alloc_value: u32, key: Key), Operand>` where
   `Key = Str(StrId) | Idx(i32)`.
2. **Store update:** on `StoreProp { obj: Value(a), key, val, .. }` or
   `StoreIndex { obj: Value(a), index: Const(int), val, .. }` where
   `a` is an unaliased alloc (RefinePurity proved Pure on dependent
   accesses):
     * Insert `(a, key) → val`.
3. **Store invalidation:** on `StoreProp/StoreIndex` with non-const
   key OR on a different alloc receiver that the analysis can't prove
   distinct → drop ALL entries for that alloc receiver. Stores on
   provably-distinct allocs (different `Value` ids on different
   AllocObject/AllocArray defs) don't invalidate each other.
4. **Load resolution:** on `LoadProp { obj: Value(a), key, purity:
   Pure }` or `LoadIndex { obj: Value(a), index: Const(int), purity:
   Pure }`:
     * If `(a, key)` is in the map → replace the Load's result Value's
       uses with the stored Operand (`replace_value_uses`), mark the
       Value Removed, unhook the Load from its block, blank the Load's
       operands (DCE convention).
     * Else: leave alone.
5. **Side-effect barriers:** Calls / OpaqueEval / any inst with
   `WRITES_HEAP` not on a tracked alloc → flush the entire map
   (conservative; locks may have aliased through globals).

Bail at block boundaries — map is reset per block.

**Why same-block-only for v1:** Babel-compiled `{foo: 1, bar: 2}.foo`
shape and array-literal-then-immediate-indexed-read both fit. Cross-
block dataflow demands a join lattice (intersect maps across
predecessors) and per-block fixed point — bigger design, deferred.

### Part 2 — Constructor-function inlining (`ConstructorFnInlinePass`, ModulePass)

Extension of the inter-procedural family. Sibling to
`IdentityFnInlinePass`.

**Recognition (function summary):** A function `g` is a
"constructor" iff:

  - Body is `Lowered`, single block, no block params.
  - Instructions are exclusively: ONE `AllocObject`/`AllocArray` + N
    `StoreProp`/`StoreIndex` instructions whose `obj` is the alloc
    Value.
  - Terminator is `Return { val: Some(Operand::Value(alloc)) }`.
  - All store keys are `Const(StrId)` or `Const(Int32/Number)` (no
    dynamic keys).
  - Store `val` operands are `Operand::Const(_)` or `Operand::Value(arg_i)` only
    (no dependence on heap reads / arith / calls — keeps cloning
    deterministic without cloning a sub-DAG).

**Inlining (per Call site):** For each
`Call { callee: Const(Func(g)), args }` where `g` is a constructor:

  1. Allocate fresh Value IDs for the cloned alloc + each store.
     (Stores produce no Value but each has its own InstId.)
  2. Substitute every `Operand::Value(param_i)` in cloned operands with
     `args[i]` (with bounds check; missing args → `Const(Undefined)`).
  3. Fix the `obj` operand of cloned stores to reference the
     freshly-allocated alloc Value.
  4. Insert cloned alloc + stores into the caller's block, immediately
     before the Call's position in `block.instructions`.
  5. Replace uses of the Call's result Value with the cloned alloc's
     Value via `replace_value_uses`. Retire the Call's Value, unhook
     the Call inst, blank operands.

After Part 2, Part 1 sees the cloned alloc + stores in the caller and
folds dependent loads. DCE retires the cloned stores once their
result-receiver loads have folded. DeadFnElim drops the now-unused
constructor functions.

**Soundness:** Each Call site receives its own fresh alloc Value
(distinct identity). JS semantics preserve identity per call (each
call to a constructor returns a fresh object). No identity-sharing
hazard.

## Pipeline placement

```
Module passes (registration order):
  IdentityFnInline → ConstructorFnInline → DeadFnElim
Function passes (per-fn fixed point):
  RefinePurity → RefineArith → ConstFold → BranchFold →
  CopyProp → ForwardLoads → DCE
```

`ConstructorFnInline` runs after `IdentityFnInline` because identity
inlining can simplify some constructor-call argument expressions
(may surface more Const args, exposing more constructor matches).
`ForwardLoads` runs after `ConstFold` (so any Load-key it sees is
already const-folded if possible) and before `DCE` (so cloned stores
that fed forwarded loads get retired).

## Soundness gate

- **Aliasing:** Forwarding only acts on Pure-marked Loads.
  RefinePurity marks loads as Pure only when the receiver alloc is
  unaliased intra-procedurally — no Proxy, no escape.
  Constructor-inlining produces fresh allocs per call site → each
  cloned alloc starts unaliased; RefinePurity's next run upgrades the
  cloned loads to Pure → ForwardLoads can fold them.
- **Side effects:** Stores' val operands may be expression results
  with their own effects, but those instructions are emitted at their
  own positions in the caller — we don't reorder them. Forwarding
  replaces a Load with the Operand that was stored, which references a
  Value already defined upstream.
- **Identity semantics:** Constructor-inlining preserves "fresh
  object per call" by allocating a fresh alloc Value at each call
  site. Sharing one alloc across two call sites would change
  observable identity (`a === b` between two calls would flip from
  false to true) — explicitly avoided.

## Out of scope (deferred)

- Cross-block forwarding within a function (needs join-lattice
  dataflow). Punted.
- Constructors with non-const-arg-derived store values, multi-block
  bodies, conditional construction. The matcher rejects these.
- Function-summary-based cross-procedural forwarding without physical
  inlining. Cleaner long-term but larger surface; rejected for this
  ticket — physical cloning composes elegantly with existing passes.
- Read-after-write forwarding when the receiver could alias another
  Value of the same type but distinct identity (e.g., two distinct
  `AllocObject` results both received as args from outside). Pure
  marking guards against this — RefinePurity only marks loads on
  *unaliased* allocations.

## Test plan

Part 1 (`ForwardLoadsPass`):
- Babel-style: `AllocObject + StoreProp(a,"x",1) + LoadProp(a,"x")` →
  Load resolves to `Const(1)`.
- Array form: `AllocArray + StoreIndex(a, 0, "hello") + LoadIndex(a,
  0)` → resolves.
- Unknown-key store invalidates: `Store(a, k, _) + LoadProp(a, "x")`
  where k is a Value → Load not folded.
- Two distinct allocs don't interfere.
- Non-Pure load (Purity::Throws) is not forwarded.
- Call between store and load flushes (conservative).

Part 2 (`ConstructorFnInlinePass`):
- `function mk() { return {foo: 1}; }` — call sites get inlined alloc+store.
- `function arr3() { return ['a','b','c']; }` — array form.
- `function withArg(x) { return {val: x}; }` — Arg substitution.
- Multi-call: each call site gets a fresh alloc.
- After full pipeline: `mk().foo` ⇒ `1`.

End-to-end:
- `function mkArr() { return ["alpha","beta","gamma"]; } mkArr()[1]`
  collapses to `"beta"` after pipeline runs.
- 28/28 corpus stays green.

## Estimated size

- Part 1: ~250 LoC + tests.
- Part 2: ~300 LoC + tests (cloning is intricate but bounded).

Two commits, both required for full corpus benefit; Part 1 lands
useful on its own (Babel inline-object access cleanups).
