# Plan P2 — CFG-based control-flow recovery + structuring

**Goal:** recover the *original structured control flow* from flattened-switch
dispatchers — including the **branching and cyclic** ones the merged linear
unflattener cannot touch — and re-emit clean `if/else/while/for`. This folds the
largest residual category (control-flow flattening) across samples 1, 4, 5, 6, 11.

**Depends on P1.** The recovered graph is keyed on *concrete state values*; P1
turns symbolic case labels/transitions into literals. P2 without P1 has no nodes.

**Builds on the merged work.** `unflatten.rs` (`detect_loop`/`parse_cases`/
`simulate`) and `unflatten_param.rs` (interprocedural specialization) are the
**special case**: a dispatcher whose recovered graph is an acyclic linear/tree
chain. P2 generalizes `simulate` from a *linear walk* to a *graph algorithm*, and
adds the missing *structuring* step.

---

## 1. Success criteria (measurable)

- `switch` count on samples 1, 4, 11 drops toward **~0** (today 38 / 6 / 37); the
  output is structured `if/else/while/for`, not a dispatcher.
- The recovered code is **semantically equivalent** — a dedicated equivalence
  battery of branching/looping/nested dispatchers passes, and the sample suite
  stays idempotent + reparse-clean.
- Irreducible graphs are **left untouched** (sound no-op), never mis-structured.

## 2. What exists (the substrate is already here)

| Piece | Where | Role in P2 |
|---|---|---|
| Linear dispatcher recognizer + simulator | `unflatten.rs`: `detect_loop`, `parse_cases`, `simulate`, `LoopTest`, `Trans` | Recognize the dispatcher; the acyclic fast-path. Reused as-is. |
| Interprocedural param-state specialization | `unflatten_param.rs` | The front-end for param-state dispatchers; feeds entry constants. Reused. |
| CFG lowering + dominators | `src/ir/cfg.rs` (`Lowerer`, `Cfg`, `Block`, `succs`), `src/ir/dom.rs` (`Dominators`: `idom`, `frontier`, `dominates`) | Lower case bodies; recover loops via back-edges. |
| NodeId (arena `Address`) AST mapping | `cfg.rs` | Splice recovered structure back onto exact AST nodes. |

## 3. The precise gap

`simulate()` bails (correctly, today) on everything that isn't a deterministic
linear/tree chain:

1. **Conditional transitions** — `case 5: …; if (c) s = 7; else s = 9; break;`.
   This is the *recovered branch* — an `if`. The linear walk has no representation
   for "two successors."
2. **Real loops** — a state revisited via a back-edge (`simulate` aborts on
   revisit). That revisit *is* a `while`/`for` in the original.
3. **No structuring** — even with the graph in hand, we must emit readable
   `if/while/for`, not `goto`/labeled-dispatch.

These three are exactly the branching dispatchers dominating samples 1/4/11.

## 4. Design — *recover the graph, then structure it*

A new module `src/passes/unflatten_cfg.rs` (+ `src/ir/structure.rs` for the
re-emission algorithm). Pipeline: runs **after** P1 (labels are literals) and after
the linear `unflatten` (which still handles the easy acyclic cases fast).

### 4.1 Recognize (reuse)

Same recognition as `unflatten` (`detect_loop` over state `s`, body is
`switch(s)`), but **do not require** the chain to be acyclic or transitions to be
unconditional. Param-state dispatchers come pre-specialized by `unflatten_param`.

### 4.2 Recover the state CFG

Build a graph whose **nodes are state values** and whose **edges are transitions**:

```
RecoveredCfg { nodes: Map<i64, CaseBlock>, edges: Map<i64, Vec<Edge>>, entry: i64, exit: synthetic }
CaseBlock  { content: Vec<&Statement> }          // case body minus its dispatch transitions
Edge       { guard: Guard, target: Target }      // Target = State(i64) | Exit
Guard      { Always | If(cond expr) | Else }
```

Per case, **separate dispatch from content** by lifting the case body to a small
sub-CFG with the Lowerer, treating `s = K` as "this path's successor is block K":

- A case ending `…; s = A; break;` → one `Always` edge to `A`, content = the rest.
- A case with `if (c) { …; s = A } else { …; s = B } break;` → an `If(c)` edge to
  `A` and an `Else` edge to `B`; the `if`'s non-dispatch statements become content
  on the respective edges (or the shared prefix becomes block content).
- `return/throw` (anywhere a path ends without re-setting `s`) → edge to `Exit`,
  carrying the terminator.
- The loop test `LoopTest` defines an implicit `Exit` edge from any state where it
  goes false (the merged `continues()` logic, now an edge predicate).
- Computed transitions (`s += K`) resolve to concrete targets when the source
  state is known (we enumerate reachable states from `entry`, exactly as `simulate`
  already does — but recording edges instead of emitting a linear sequence).

**Soundness of recovery:** the recovered CFG is, by construction, the state machine
itself — node = "the statements that run in state K", edges = "the exact state
transitions". Nothing is dropped or reordered; we only *re-describe* the dispatcher
as a graph. (The equivalence harness validates this end-to-end.)

### 4.3 Recover loops

Compute `Dominators` on the recovered CFG. **Back-edges** (an edge `u → v` where
`v` dominates `u`) are loops; the rest is acyclic forward structure. This replaces
`simulate`'s "revisit ⇒ bail" with "revisit ⇒ a loop edge" — the core capability
the linear walk lacks.

### 4.4 Structure (the decompiler step) — `src/ir/structure.rs`

Re-emit structured AST from the recovered CFG using a **relooper / Dream-style**
algorithm over the **reducible** case (the overwhelming majority of CFF output):

- Classify regions: **Simple** (single block + one successor), **Loop** (a region
  with a back-edge → `while (true) { … }` with `break`/`continue` at the loop
  edges), **Multiple** (a branch point → `if/else if`), recursively.
- Use dominator + post-dominator info to find single-entry/single-exit regions and
  collapse them. Condition guards come from the `Edge::Guard` exprs (the original
  `if` conditions, preserved verbatim — not synthesized).
- Emit `while`/`do-while` for natural loops, `if/else` for branches, straight-line
  for Simple chains. The merged linear `simulate` becomes the degenerate "all
  Simple, no Loop/Multiple" output.

**Irreducible CFGs** (rare in obfuscator output, but possible): do **not**
mis-structure. Either leave the dispatcher untouched (sound no-op) or, behind a
flag, emit a minimal labeled-block form. Default: leave untouched. `log()` that it
was skipped (no silent failure).

### 4.5 Splice back

Replace the dispatcher statement(s) with the structured AST (NodeId-addressed),
exactly as `unflatten` splices its linear sequence today. For param-state, emit the
structured body inside the specialized function `unflatten_param` already produces.

## 5. Soundness & test strategy

The differential harness is the proof. New `tests/equivalence.rs` battery — each
case computes a value and must match original-vs-deobfuscated:

- **Branch:** `case 1: if (n>0) s=2; else s=3; break; case 2:…; case 3:…` → `if`.
- **Loop:** a state revisited via a back-edge (`while`), incl. `break`/`continue`
  realized as loop-exit / loop-back edges; loop-carried accumulator.
- **Nested:** loop containing a branch; branch whose arms re-enter the loop.
- **Mixed terminators:** a `return` inside one arm, fall-through `break` in another.
- **Irreducible:** two states with cross-edges (jump into a loop body) → assert the
  pass **declines** (output still equivalent, dispatcher preserved).
- **Adversarial "looks linear but isn't":** a conditional transition that the old
  `simulate` would wrongly linearize → must produce the branch, value matches.

Plus: every sample stays idempotent + reparse (`tests/samples.rs`); add a pinned
test that samples 1/4/11 contain no `while(state)`/`switch(state)` dispatcher after
the pass (analogous to `fully_static_samples_fold_to_console_log`).

## 6. Milestones (each lands green)

- **M1 — Recovered-CFG builder (read-only).** `detect_loop` → `RecoveredCfg` with
  conditional + back edges; a `UNFLATTEN_CFG_DEBUG` dump. Unit tests on synthetic
  branching/looping dispatchers (assert the graph shape). No AST mutation yet.
- **M2 — Structuring for reducible CFGs.** `src/ir/structure.rs`: Simple/Loop/
  Multiple → `if/while`. Wire splice-back. Equivalence battery (branch, loop,
  nested) green.
- **M3 — Pipeline integration after P1.** Run on samples; measure `switch` → ~0 on
  1/4/11; pin the no-dispatcher assertions; confirm idempotency.
- **M4 — Irreducible handling.** Detect irreducibility (no reducible collapse);
  decline soundly (default) or labeled-block form (flag). `log()` skips.
- **M5 — Cleanup folds.** Ensure the emitted `if (true)/if (false)` from resolved
  guards fold via the existing `flatten-else`/`const-fold`/DCE next round (the
  cascade), so recovered code reads like source.

## 7. Risks & mitigations

- **Structuring is the classic hard decompiler problem.** Mitigation: target
  **reducible** CFGs (what CFF actually produces — it flattens *structured* source,
  so the recovered graph is reducible by construction), and **decline soundly** on
  irreducible. Never emit `goto`-soup.
- **Conditional-transition extraction must be exact.** The case body's branching
  *is* the recovered control flow; an off-by-one in "which statements are content
  vs dispatch" changes semantics. Mitigation: lift each case body through the
  existing `Lowerer` (proven) rather than hand-parsing; heavy equivalence coverage.
- **Interaction with the linear unflattener.** Keep `unflatten` as the acyclic
  fast-path; `unflatten_cfg` only engages when `simulate` declines (cyclic /
  conditional). They must not double-transform — gate `unflatten_cfg` on "the
  linear form bailed."
- **Loop `break`/`continue` from inside nested switches** in the original must map
  to the right loop edge. Mitigation: the `Lowerer` already models labeled
  break/continue; reuse its loop-context handling.

## 8. Out of scope (later phases)

- **Bytecode VMs** (samples 8, 9, 10): a fetch-decode-execute loop over an opcode
  *array* is a different recognition + a per-opcode semantic model — Phase 3. The
  CFG IR built here is the *target* for that lift, but the front-end differs.
- **Irreducible control flow** beyond the labeled-block fallback.
- Variable renaming of the recovered structure (the existing `rename-by-role`
  handles it next round).

---

## Appendix — how P1 + P2 make obfuscation "fold away"

One fixpoint round, post-P1/P2, on a flattened sample:

```
decoder-lift     strings/tables → literals          (exists)
const-resolution case varN: / f(varN,…) → numbers    (P1)
unflatten[-cfg]  dispatcher → if/else/while/for       (P2, merged linear = fast path)
const-fold       resolved guards `if(true)` collapse  (exists)
flatten-else/DCE dead arms + dead dispatcher table    (exists)
rename-by-role   varNNN → descriptive names           (exists)
```

The new pieces (P1 const-resolution, P2 CFG recovery) are the two links missing
from that chain today; with them in place the existing passes carry the rest, and
a 27 kLOC dispatcher collapses to readable source.
