# js-deobfuscator: complete rewrite design

**Date:** 2026-05-07
**Status:** design — not yet implemented
**Decision:** scrap the current ~45k-line codebase and rebuild around partial evaluation, leveraging oxc end-to-end.

---

## Why we're rewriting

The current architecture (custom SSA + custom tree-IR + custom relooper + 14 hand-written passes + handwritten emitter) is structurally wrong for the goal:

- **Goal (per user):** a generic JS deobfuscator that handles *all* obfuscation classes — string tables, control-flow flattening, dispatcher VMs, dead-code stuffing — the way Mergen handles VMProtect/Themida by aggressive optimization until the obfuscation dissolves.
- **What we built instead:** a per-construct lowering pipeline where every JS feature requires a hand-written lift, and analysis precision is gated by an effect lattice that's too coarse to fold across ordinary call sites. Net result on a tier3 corpus sample: `let _t0; let _t1; let _t2; …` chains that hand-written passes mop up cosmetically but never actually collapse.

The work has been dominated by **infrastructure** (parsing glue, disposition, IR mirroring, emit) instead of by the **research problem** (partial evaluation, effect/alias modeling, function summaries). Rewriting around the right primitives gets us to the actual problem with less code.

Don't worry about lost work. The intuitions that drove the current codebase (no pattern matching, no builtin allowlists, effect-lattice precision is load-bearing) carry forward. The code itself does not.

---

## Core idea

**Partial evaluation as the deobfuscator.** No pattern recognizers, no per-obfuscator passes.

The engine walks the program in execution order with an abstract state:

- **Concrete values:** numbers, strings, booleans, frozen arrays/objects we can see end-to-end. Tracked exactly.
- **Symbolic values:** function args, network reads, environmental things. Stay as named variables.

Four rules:

1. **Branch with concrete condition** → take that branch, delete the other.
2. **Call to a function we can see** → evaluate the body. All-concrete args produce a concrete result. Mixed args produce a *summary* ("returns expr involving X, may throw if Y, mutates these heap cells"). Cached. Reused at every call site.
3. **Load from a frozen table** → constant.
4. **Loop whose carried state is concrete** → unroll. The dispatcher in a JS-VM obfuscator carries a state variable around the loop; if state is determined, we know which case fires, we update state, we iterate. The whole VM dissolves into the straight-line code it was hiding.

That's the entire deobfuscator. String tables, CFG flattening, VM dispatchers — they're all "concrete computation hidden behind a layer of indirection." Partial evaluation collapses indirection by definition.

---

## Architecture

```
input.js
   │
   ▼
oxc parse                                  (oxc owns)
   │
   ▼
oxc-transformer                            (oxc owns — collapses modern-JS to kernel)
   │
   ▼
oxc_semantic (scopes, bindings)            (oxc owns)
   │
   ▼
lift to SSA                                (we own — ~2k lines, mechanical)
   │
   ▼
partial-evaluation engine ◄──── QuickJS    (we own engine; rquickjs evaluates concrete subterms)
   │      ▲
   │      │
   │      function summaries cache         (we own)
   │
   ▼
relooper (CFG → structured control flow)   (published algorithm — Bahmann, or Cheerp/Emscripten)
   │
   ▼
oxc AST out
   │
   ▼
oxc_codegen                                (oxc owns)
   │
   ▼
output.js
```

Two strict rules across the architecture:

- **One IR.** SSA only. The current tree-IR layer (which mirrors oxc's AST with cosmetic differences) does not exist in the new project.
- **Lift directly from oxc AST.** No "lower to a normalized intermediate" step. SSA build walks oxc AST.

---

## What we don't write anymore

The biggest single insight from the brainstorm: **most of the current 45k LoC is reinventing things that already exist.** The leverage points, ordered by leverage:

### 1. oxc-transformer for modern-JS desugaring (kills the BlockerKind grind)

The whole "lift fallback" / "support construct X" workstream — sequence expressions, optional chaining, destructuring, default params, classes (private fields, static blocks, decorators), async/await, generators, top-level await, BigInt, regex flags, etc. — **stops existing as a category of work.**

oxc-transformer is in-tree with oxc and lowers all of these to a small kernel of equivalent JS *as a preprocess.* Our backend then knows about ~30 node kinds total, forever. New JS features ship in TC39 → oxc-transformer adds them → we get them for free. No more `BlockerKind` enum, no more disposition layer, no more per-construct lifting tickets.

The only genuinely-undesugarable features are `eval`, `with`, `Function` constructor, dynamic `import()`. The IR treats them as opaque-effect calls. That's the *only* form of "fallback" in the new architecture, and it's a flat ~5–6 cases that never grow.

### 2. Embedded JS engine for concrete sub-evaluation (kills the builtins debate)

When the partial evaluator has an expression with all-concrete inputs — `Math.floor(424 / 16)`, `"abc".charAt(2)`, `table[14]`, the obfuscator's whole string decoder applied to a concrete index — **don't reimplement JS semantics.** Hand the snippet to a sandboxed JS engine (`rquickjs`, ~150KB compiled, single Rust crate, full ES2020), get the answer back, substitute it.

What this dissolves:

- Builtin effect modeling — entirely. We never need to know what `Array.prototype.slice` *does*; if its receiver and args are concrete, real `slice` runs and we get the result. The generic-vs-allowlist debate (see memory: `feedback_no-builtin-allowlist`) goes away — there is no allowlist *and* no abstract interpreter for the spec.
- Reimplementing arithmetic, string ops, regex, JSON, type coercion, NaN edge cases. All the spec's worst corners are someone else's job.
- The string-decoder problem: just *run* the decoder.

The PE engine's core loop becomes: "for each SSA value, ask — are all inputs concrete? If yes, ship to engine, replace with result. If no, keep symbolic." That's the whole engine.

### 3. oxc_semantic for scope/binding analysis (kills the over-pinning bugs)

Today we hand-roll: scope trees, binding resolution, capture tracking, shadowing detection, hoisting. **oxc has all of this built in.** Sibling crate to `oxc_parser`/`oxc_ast`. Builds a complete scope tree, resolves every identifier, identifies free variables in closures.

Deletes ~2–3k lines of our code and fixes a class of subtle bugs (the `let String;` / `let decodeURIComponent;` over-pinning we just observed in tier3 output is a scope-analysis bug — oxc's analyzer wouldn't make it).

### 4. oxc_codegen for emit (kills the handwritten stringifier)

Already filed as issue #55. Build oxc AST nodes directly from SSA-via-relooper, hand to `oxc_codegen`. Retires `is_js_identifier`, the bracket→dot helper, the `0._` numeric-base patch, the `expr_diff` Index↔Member equivalence, the "module re-parse failed" assert, and the entire `Index↔Member` cosmetic distinction in the IR. ~700 LoC deleted.

### 5. Differential testing against a real JS engine (kills the test-maintenance grind)

Replace most hand-written correctness tests with one universal harness:

> Run the original JS in QuickJS with seeded inputs, capture observable output (return values, side effects to stub `console`/`fs`, exception class). Run the deobfuscated output the same way. **Outputs must match.**

Catches every semantic bug — wrong evaluation order, dropped side effect, mishandled exception, incorrect constant folding — automatically. webcrack does this.

Pairs with the engine we're already embedding for evaluation (#2): same engine, two jobs.

### 6. Published relooper algorithm

The CFG → structured-control-flow problem is solved. Bahmann's "Perfect Reconstructability of Control Flow from Demand Dependence Graphs," or the simpler Cheerp/Emscripten relooper. ~500 lines, production-tested in WASM compilers. Don't invent.

### 7. Macro-generate IR-variant boilerplate

Today, adding a new SSA op ripples into pretty-printer, walker, builder, effect classifier, substitutor, validator, snapshot test — 5–8 files per change. A `define_ir!` macro emits all of these from one declaration. ~1 day of `syn`/`quote` work, pays back forever.

### 8. Property-based + snapshot testing as the test strategy

`insta` and `proptest` already in deps. Lean harder:
- **Snapshot per pass** — feed input, snapshot SSA after, eyeball diffs in review. No hand-written assertions like "does the output contain `let x`."
- **Property tests** — generate random concrete programs, run pipeline, execute output in QuickJS, assert semantic equivalence. Discovers bugs no human writes a test for. (We already saw this work — the `0._` bug surfaced from a proptest seed.)

### 9. Real-world corpora instead of curated fixtures

Pull dist bundles from npm — every `node_modules/*/dist/*.js` is a bundled JS file. Webpack/esbuild/Babel outputs. Free, infinite, representative. For obfuscated samples specifically: github code search for `obfuscator.io` / `javascript-obfuscator` hits hundreds of public repos.

A harness that pulls 100 random npm packages and runs the pipeline gives more coverage than any fixture suite a human will curate.

---

## What stays uniquely ours

| Component | Lines (est.) | Purpose |
|---|---|---|
| SSA IR core | ~2,000 | leaner than today; one IR layer, not two |
| Lift oxc AST → SSA | ~2,000 | replaces `parse/lower_tree` + `parse/lower_ssa` |
| **Partial-evaluation engine** | ~3,000 | the actual deobfuscator; small in code, dense in design |
| Function summaries / interproc | ~1,000 | computed lazily, cached, invalidated on heap-effect propagation |
| Effect / alias / escape model | ~1,000 | from day 1, not bolted on |
| Renamer | ~500 | identifier cleanup, orthogonal to PE |
| SSA → oxc AST (relooper wrapper) | ~1,500 | calls the published relooper, builds oxc nodes |
| CLI + corpus harness | ~2,000 | |
| Tests + fixtures | ~3,000 | mostly differential + property-based |
| **Total** | **~16,000** | ~⅓ of current 45k |

Every line is *the actual research problem*, not plumbing.

---

## Non-negotiable principles (carried forward from current project)

These are all backed by saved memory entries; they survive the rewrite unchanged.

- **No obfuscator-specific pattern matching.** We are a generic optimizer. Deobfuscation emerges from sound general analyses, not from matching specific obfuscator idioms. `feedback_no-obfuscator-specific-pattern-matching.md`
- **No builtin/spec allowlist tables.** Don't hardcode `Math.*` / `parseInt` / `String.prototype.*` effect tags. The embedded JS engine subsumes this — we never need to know builtin effects symbolically. `feedback_no-builtin-allowlist.md`
- **Effect lattice precision is load-bearing.** Never collapse a precise effect into a coarser tier as a shortcut. Refactor the lattice instead. `feedback_effect-lattice-precision.md`
- **Verify codebase state via tooling, not recall.** Run a command, read source. Memory hooks rot, debug output is ambiguous. `feedback_verify-via-tooling-not-recall.md`
- **Design-first for architectural work.** This document is exactly that. `feedback_design-first-for-architecture.md`

---

## Migration / cutover

This is a *clean rewrite*, not a refactor. New project lives alongside the existing one until the new one's corpus pass-rate exceeds the old one's. Then old one is deleted in one commit.

- New project at `crates/v2/` (or a sibling repo — TBD)
- Same corpus harness format → both can be benchmarked head-to-head
- Cutover gate: new project ≥ old project on:
  1. semantic-equivalence pass-rate on the corpus (differential test, must be ≥ 99%)
  2. tier3 output line count (must be ≤ current)
  3. the long-tail JS feature surface (must be ≥ — measured by oxc-transformer subsumption, not BlockerKind count)
- After cutover: delete `crates/parse`, `crates/pass`, `crates/ir`, `crates/emit` (current implementations).

Estimated time to cutover: **6–8 weeks of focused work**, dominated by the PE engine and the lift-from-oxc layer. Infrastructure (parser/codegen/transform/scope) is hours, not weeks, because we're not writing it.

---

## Out of scope for this design

These are real questions but separate decisions, deferred:

- Source-map / span fidelity (we synthesize; no original source positions to preserve from a partial-eval pipeline).
- Sandbox security model for the embedded engine — QuickJS sandboxing is good enough for static analysis but worth a dedicated review before we're running attacker-controlled code at scale.
- Performance targets — assume "fast enough that interactive use on a single bundle is sub-second" but no benchmarks committed to.
- Whether function summaries are first-class IR objects or implementation-detail of the PE engine — punt to implementation.
- Renamer policy (heuristics for picking better names than `_t0`) — orthogonal, ship a v1 that just preserves source-level names where they exist.

---

## Self-review notes

- No placeholders. Every section has a concrete recommendation.
- Internal consistency: the architecture diagram, the layered "what we don't write anymore" table, and the "what stays uniquely ours" table all describe the same system from different angles. Cross-checked.
- Scope: this is a single rewrite, not a multi-subsystem program. One implementation plan can cover it.
- Ambiguity: the only deliberate ambiguity is "rquickjs vs Boa vs rust_v8" — the design doesn't pin this; first implementer picks based on actual API ergonomics. Flagged as out-of-scope above.
