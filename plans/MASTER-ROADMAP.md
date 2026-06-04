# Master roadmap — full deobfuscation

The umbrella plan over **every** advancement worth making, organized into
workstreams, sequenced by leverage. It supersedes nothing — [P1](P1-whole-program-const-resolution.md)
and [P2](P2-cfg-control-flow-recovery.md) are the *started* slices of workstreams
A and B here; [P0](P0-execution-plan.md) is the orchestration discipline (gates,
revert criteria, equivalence harness) that **every** item below inherits.

The hard invariants never change (P0 §0): soundness is gated by the differential
equivalence harness (`tests/equivalence.rs`, QuickJS), test-first, one
milestone = one green commit, measure every step, revert (don't patch) a
regression. A pass only ever *widens what it accepts* and **declines** otherwise.

---

## 0. The thesis

Every remaining non-trivial sample is one of three things, in layers:

1. **Control-flow flattened** — a dispatcher loop over a state, in one of several
   *dialects* (integer, param, string-array-order, computed/object). Today each
   dialect is a separate narrow pass. **The big win is a single
   dispatcher-agnostic recovery engine** (abstract-interpret the state → recover
   the graph → relooper-structure it). Workstream **A**.
2. **Value-obfuscated** — the constants/labels/strings the recovery keys on are
   hidden behind init functions, constructors, array aliases, decoders, and memo
   caches. Workstream **B** (resolution) + **C** (decoders/strings) feed A.
3. **Bytecode-virtualized** — a fetch-decode-execute interpreter over an opcode
   array (s8/s9/s10). A different recognition + per-opcode semantic lift.
   Workstream **D** — the separate mega-project.

On top sit **high-level reconstruction** (E: helper/async/MBA/rename) and
**engine hardening** (F: determinism, a real shared CFG/SSA substrate, whole-sample
equivalence, performance, sandbox host stubs).

### Per-sample target ("done" means reads like source)

| sample | size | nature | blocking workstreams | realistic ceiling |
|--------|------|--------|----------------------|-------------------|
| s1  | 1.26 MB | VM-in-constructor + CFF | A2, A3, B1 | structured methods; dispatchers → if/while |
| s2  | 1.2 KB | trivial | — | ✅ done |
| s3  | 117 KB | tslib async/generator bundle | E1, C1 | async/await + named helpers |
| s4  | 169 KB | computed-state CFF | A3, A2, A4 | dispatchers → if/while/for |
| s5  | 48 KB | Cloudflare string-array CFF | **A1**, A2 | **near-full** (start here) |
| s6  | 50 KB | Cloudflare string-array CFF | **A1**, A2 | **near-full** (start here) |
| s7  | 746 KB | Base91 table + host sensor | C2, B3, F5 | strings lifted; sensor host-coupled |
| s8  | 402 KB | async state machine + flatten | D, E1, A | partial (VM tail out of scope) |
| s9  | 157 KB | bytecode VM | D | devirtualize (Phase 3) |
| s10 | 2.1 MB | bytecode VM, Closure-min | D, B2, F4 | devirtualize; prop-names floor |
| s11 | 342 KB | DataDome 2D-table CFF + MBA | B2, A2, E2, C1 | dispatchers + tables; MBA-gated |
| s12 | 0.4 KB | — | — | ✅ done (→ console.log) |
| s13 | 0.4 KB | — | — | ✅ done (→ console.log) |

---

## Workstream A — Control-flow recovery (the unified engine)

The current passes (`unflatten`, `unflatten_param`, `unflatten_cfg`) are point
solutions. The end state is one recovery built on a real CFG + abstract
interpretation of the dispatch state, then a relooper. Build it incrementally so
each step lands green; the early items are concrete dialect wins, the later items
are the unification.

### A1 — String-array-order dispatcher unflattener  ★ START HERE
**Samples: 5, 6 (Cloudflare). Impact: very high. Effort: medium. Risk: low.**

The dominant Cloudflare CFF shape — fully static order:
```js
for (var a = "0|8|9|10|4|3|6|7|1|2|5".split("|"), i = 0; true;)
    switch (a[i++]) { case "0": …; continue; case "1": …; continue; }
```
- The order array is a constant literal (or `"…".split(sep)`, or a const-resolved
  array). The index var only ever `i++`/`i+=1`. The discriminant is `a[i++]`.
  Cases end in `continue` (loop back) or `return`/`throw` (terminal).
- Execution order = array order: a static linear chain. Reuse `unflatten`'s
  `simulate`/splice machinery, generalized to a **string** (or int) key drawn from
  a const sequence rather than an integer state var.
- **Soundness gates** (decline otherwise): the order array is provably constant
  and never mutated (reuse `ir::analysis::mutable_aggregate_tables`); the index is
  monotonic and read only as `a[i++]`; the loop is `for(…; true;)`/`while(true)`;
  no `default`; each case body is dispatch-clean (doesn't read `a`/`i`).
- **Adversarial pairs**: order array mutated (`a.push`) → decline; index reset
  (`i = 0` mid-loop) → decline; non-constant array → decline.
- New module `src/passes/unflatten_string_order.rs` or a generalization of
  `unflatten::state_init`/`detect_loop` to a "sequence cursor" state.

### A2 — General relooper / structuring  ★ the structural core
**Samples: 1, 4, 5, 6, 11. Impact: very high. Effort: high. Risk: medium.**

Finish [P2/M2](P2-cfg-control-flow-recovery.md): turn an arbitrary **reducible**
`RecoveredCfg` (already built in `src/ir/cff.rs`) into `if/else/while/for` via a
Dream/relooper algorithm, not just the single self-loop handled today.
- Classify regions with dominators + post-dominators: **Simple** (block+succ),
  **Loop** (back-edge → `while(true){…}` with `break`/`continue` at loop edges),
  **Multiple** (branch → `if/else if`), recursively collapse single-entry/
  single-exit regions.
- Guards come from the recovered `Edge::Guard` (original `if` tests, verbatim).
- **Irreducible** graphs (rare in CFF output, which flattens *structured* source):
  detect (no reducible collapse) and **decline** (sound no-op) or, behind a flag,
  emit labeled blocks. Default: leave the dispatcher. `log()` the skip.
- Heavy equivalence battery: branch, loop (break/continue, loop-carried),
  nested (loop⊃branch, branch arms re-entering the loop), mixed terminators.
- This subsumes `unflatten_cfg`'s self-loop and the linear `simulate`'s acyclic
  chain as degenerate cases — eventually fold all three into one structurer.

### A3 — Computed / object-state recognition
**Sample: 4. Impact: high. Effort: high. Risk: medium.**

s4's main dispatcher is `switch(var7 + var4)` where `var7 = fn21(new Number(91), …)`
and `var6.set(var4)` — a *stateful coercion object*. Generalize recovery to:
- Recognize a discriminant that is an *expression over* a dispatch object + a
  counter, not a bare identifier.
- Abstract-interpret the state object: model its `.set()`/`valueOf()`/arithmetic
  so the next state is statically determined (sandbox-backed where pure).
- Tolerate **prefix statements** before the loop (today `shape_ok` requires the
  loop to be the first statement) — execute/emit them before the recovered body.
- Decline when the object's transition can't be evaluated deterministically.

### A4 — Dispatcher-agnostic recovery substrate (the unification)
**Impact: architectural. Effort: very high. Risk: medium.**

Replace the per-dialect recognizers with one front end:
- A `DispatchModel` trait: "what is the state, what is its initial value, how does a
  case transition it, when does the loop exit." Implementations: integer-state,
  param-state, string-array-cursor (A1), object-state (A3).
- One **abstract interpreter** enumerates reachable states from the entry and emits
  edges into `cff::RecoveredCfg` regardless of model; A2 structures the result.
- Net effect: a new obfuscator dialect is a small `DispatchModel`, not a new pass.

### A5 — Discriminant & transition generalizations
**Impact: medium (long tail). Effort: medium. Risk: low.**
- `switch` on strings/booleans; transitions via `+=`/`-=`/`*=`/lookup-table
  (`s = next[s]`); fall-through cases (no `break`); shared case tails.
- `if/else if` ladder dispatchers (no `switch` at all) and `goto`-via-label CFF.

---

## Workstream B — Constant & value resolution (feeds A)

The recovery in A keys on concrete labels/states. B turns the symbolic ones into
literals. The engine exists (`ConstResolver` + `OrderingOracle`, P1/S1); B widens
*what it can prove*, each widening with a must-NOT-resolve adversarial sibling.

### B1 — Constructor-init happens-before
**Sample: 1. Impact: high. Effort: high. Risk: high (soundness-delicate).**

s1 writes its label constants in `fn19()` inside a constructor (`new var2574()`).
Prove: constants written unconditionally in a constructor `C`, read only within
`C`'s body or through instance methods reachable only via a `C` instance ⇒ every
read is preceded by construction ⇒ the (idempotent) literal is safe to inline.
- Requires modeling `this`/prototype method reachability and that the write is
  idempotent (always the same literal). Adversarial: a static method or a method
  callable before any `new` → decline; a non-idempotent write → decline.

### B2 — Deterministic table resolution (2D / aliased const arrays)
**Samples: 11, 10. Impact: high. Effort: high. Risk: medium.** (issue 11)

s11 labels are `var401[i][j]` where `var401 = arr14[7]` — a nested const-array
alias. Extend the SSA const-aggregate fold cross-function: resolve `T[i][j]` when
`T` is a provably-immutable const array (possibly an alias/element of another).
- **Bloat cap** (issue 09): never materialize a table whose expansion exceeds a
  size budget (s9's 131 KB bytecode array is net-negative — exclude it).
- Decline on any member-write/`.push`/alias-mutation (reuse `mutable_aggregate_tables`).

### B3 — Value-axis: array results + pure-call preambles  (P1/M4, deferred)
**Sample: 7. Impact: high. Effort: medium. Risk: low.** (issue 09)
- `EvalValue::Arr(Vec<…>)` in `src/sandbox.rs` + a size-capped splice in
  `decoder_lift.rs` → array-returning decoders that return small literal arrays.
- `var v = pureCall()` preamble bindings + the global-object idiom
  (`globalThis`/`window`/`Function("return this")`) → unblocks s7's Base91 table.

### B4 — Fixpoint pure-expression resolution
**Impact: medium. Effort: low. Risk: low.**
Already sketched in P1 §4.1 cond 2: resolve `var b = a + 1` once `a` resolves, via
a bounded fixpoint over the const lattice, evaluated under the determinism guard.

---

## Workstream C — Decoder & string layer

### C1 — Scoped / nested decoder lift completion
**Samples: 11, 7. Impact: high. Effort: medium. Risk: low.** (issue 04)
`decoder_lift` only fully scans certain scopes; complete Tier C/D so decoders +
tables nested in function/IIFE scopes lift (the s11 `fn55`/`var597` wall).

### C2 — Memoized / host-coupled decoder handling
**Sample: 7. Impact: high. Effort: high. Risk: high.** (issue 03)
The sample_7 decoder chain bottoms out in `fw() → global`/host state. Either model
the host idiom soundly in the sandbox (F5) or decline cleanly. Lift the parts that
are pure-by-evaluation; leave the sensor-coupled tail.

### C3 — Accessor-alias follow
**Sample: 5. Impact: medium. Effort: medium. Risk: low.** (issue 06)
Follow a local accessor alias (`g = obj.get.bind(obj)`) so decoder-lift sees
through it.

### C4 — String-array rotation/IIFE cleanup
**Samples: 5, 6, 8. Impact: medium. Effort: medium. Risk: low.** (issue 05)
Once decoders are lifted and the order arrays consumed, DCE the now-dead
rotation/shuffle machinery and the spent string tables (today left behind).

---

## Workstream D — Bytecode VM devirtualization (Phase 3, the mega-project)

**Samples: 8, 9, 10. Impact: very high (only path for these). Effort: very high.
Risk: high.** A separate sub-project; do not conflate with A.

A VM is a `while(true){ op = code[pc++]; switch(op){…} }` interpreter over an
opcode *array* with an operand stack/registers. Steps:
- **D1 — Recognition**: detect the fetch-decode-execute loop, the bytecode array,
  the pc/stack/registers. (The recovered-CFG IR from A is the *target* of the lift,
  but the front end differs — it's data-driven, not control-driven.)
- **D2 — Opcode semantics**: build a per-opcode model (push/pop/binop/jump/call/…)
  by abstract-interpreting each handler once in the sandbox.
- **D3 — Symbolic execution**: interpret the bytecode symbolically to recover a
  per-function CFG of real operations (resolve computed jumps to concrete targets).
- **D4 — Re-emit**: lower the recovered operation-CFG to structured JS via the A2
  relooper. Stack slots → SSA temps.
- **Caps & non-goals**: gigantic programs (s10 2.1 MB) need F4 perf work; Closure-
  minified property names (s10) cannot be recovered without source maps — that's
  the floor, document it.

---

## Workstream E — High-level reconstruction

### E1 — Compiler-helper fingerprint + async/generator lift
**Samples: 3, 8. Impact: high. Effort: high. Risk: high.** (issues 07, 08)
- Fingerprint & rename tslib/babel/swc helpers (`__awaiter`, `__generator`,
  `__spread`, `__extends`, …) to readable names.
- Lift `__generator` state machines back to `async`/`await` + `yield` — the inverse
  of the down-level transform. This is itself a (data-driven) CFF recovery.

### E2 — MBA / opaque-predicate simplification
**Samples: 11, 10. Impact: medium. Effort: medium. Risk: low.** (issue 10)
Extend `math_fingerprint` to fold mixed boolean-arithmetic identities and constant
opaque predicates (`(x|0)===x`, `a^a`, `(p&~p)`) so dead guards collapse and the
recovered control flow simplifies (`if(true)` → fold via the existing cascade).

### E3 — Dead machinery DCE
**Samples: 5, 6, 8. Impact: medium. Effort: medium. Risk: low.** (issue 05)
After lifts, aggressively remove spent rotation loops, shuffle IIFEs, and dead
tables that current DCE leaves because of residual (now-dead) references.

### E4 — Renaming by role/type (extend existing)
**All. Impact: medium (readability). Effort: medium. Risk: low.**
Extend `rename-by-role`: infer names from usage (loop counters, accumulators,
DOM/handlers, decoders), string-key hints, and type. The last-mile readability win.

### E5 — Property-name recovery (research / likely floor)
**Sample: 10. Impact: low-medium. Effort: very high. Risk: very high.**
Closure-minified `a.b.c` property names are generally unrecoverable without source
maps or a name dictionary. Document as the floor; optionally a heuristic
dictionary pass for well-known DOM/Web APIs only.

---

## Workstream F — Engine & infrastructure hardening (cross-cutting)

### F1 — Pipeline determinism
**Impact: high (correctness of measurement). Effort: low. Risk: low.**
s3/s11 output differs run-to-run (HashMap iteration order). Make every pass that
iterates a `HashMap`/`HashSet` to *emit* sort by a stable key (SymbolId, span,
name). Then byte-identical parity becomes a usable regression gate again.

### F2 — A real shared per-function CFG/SSA substrate
**Impact: architectural. Effort: very high. Risk: medium.**
Today CFG/SSA (`src/ir/cfg.rs`,`ssa.rs`,`dom.rs`), the recovery (`cff.rs`), and the
structural passes each re-derive their own view. Consolidate onto one lowering +
SSA + dominators that A (recovery), B (resolution), and the folds all share. This
is the foundation that makes A4/B/D cleaner and is the single biggest *internal*
investment.

### F3 — Whole-sample differential equivalence
**Impact: high (soundness coverage). Effort: high. Risk: medium.**
`tests/equivalence.rs` runs *snippets*. Add a harness that executes whole samples
(or instrumented entry points) original-vs-deob in the sandbox with stubbed host,
comparing observable traces — catches sample-scale unsoundness the snippet battery
can't. Gated by F5 (host stubs).

### F4 — Performance & scale
**Sample: 10 (2.1 MB, 16k vars). Impact: medium. Effort: medium. Risk: low.**
Profile the fixpoint on the giant samples; cache scoping where safe across passes
within a round; avoid O(n²) span scans (`child_index`, the unflatten `mentions`
sweep). Needed before D on s10 is tractable.

### F5 — Sandbox host stubs
**Samples: 7, 8, 11. Impact: high (unblocks decoders/VMs). Effort: high. Risk: med.**
Deterministic stubs for the host surface sensor/anti-bot code touches
(`navigator`, `document`, `screen`, timing, crypto) so pure-relative-to-host
decoders can be evaluated, and F3 can run whole samples. Frozen/deterministic by
construction (cf. the Date-freeze latent bug in issue batch).

---

## Sequencing (leverage order, each lands green)

```
Phase I  (concrete sample wins, low risk)
  A1 string-array CFF ........ cracks s5, s6        ← do first
  C3 accessor alias .......... helps s5
  E3 dead machinery DCE ...... cleans s5, s6, s8
  F1 determinism ............. unlocks parity gate

Phase II (the structural core)
  A2 general relooper ........ s4/s5/s6/s11 conditionals + loops
  A5 discriminant/transition generalizations
  B4 fixpoint pure-expr resolution

Phase III (value layer that feeds recovery)
  B3 EvalValue::Arr + pure-call preamble ... s7 strings (P1/M4)
  C1 nested decoder lift ..................... s11, s7
  B2 deterministic 2D-table resolve .......... s11 labels (cap!)
  A3 computed/object-state ................... s4 main dispatcher

Phase IV (unification + high-level)
  A4 dispatcher-agnostic substrate
  F2 shared CFG/SSA substrate
  E1 helper fingerprint + async lift ......... s3, s8
  E2 MBA simplification ...................... s11, s10
  B1 constructor-init happens-before ......... s1
  E4 rename-by-role extension ................ readability, all

Phase V  (the mega-project)
  F5 sandbox host stubs + F3 whole-sample equivalence
  F4 performance/scale
  D  bytecode VM devirtualization ............ s8, s9, s10
  E5 property-name recovery (floor, optional)
```

**Dependencies**: A2 unblocks A3/A4 and the conditional parts everywhere. B feeds
A (resolve labels before recovering the graph that keys on them). F2 makes A4/B/D
clean. D depends on F4/F5/F3. F1 should land early (cheap, makes everything
measurable).

## Definition of done (the whole program)

- Samples 1, 4, 5, 6, 11: dispatchers recovered to `if/else/while/for`; `switch`
  count ≈ 0; `varNNN` collapsed; strings lifted; output reads like source.
- Sample 3, 8: helpers named; `__generator`/`__awaiter` → `async`/`await`.
- Sample 7: Base91 strings lifted; the host-coupled sensor tail documented as the
  boundary.
- Samples 8, 9, 10: devirtualized to structured JS (property-name floor on s10
  documented).
- Engine: deterministic output; whole-sample equivalence net green; one shared
  CFG/SSA substrate; full `cargo test` + clippy + fmt clean; metrics delta recorded.
- Every step shipped sound-by-construction, gated by the equivalence harness, with
  its adversarial must-NOT-fire sibling — exactly the P0 discipline, applied to all.

## Out of scope (the true floor)

- Property names erased by a minifier with no source map (s10) — unrecoverable in
  general; only a DOM/Web-API heuristic dictionary is feasible.
- Runtime/host-coupled behavior that has no deterministic model (live anti-bot
  sensors) — lift the pure parts, leave and document the coupled tail.
