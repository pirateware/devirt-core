# Design: SwitchStmt + BreakStmt lowering

**Status:** Design, not ticket. Switch breaks the preserve-as-syntactic pattern that's carried every round since phase 2; this doc settles three decisions before any code lands. Ticket(s) follow once the decisions are made.

**Coverage stake:** 19 PT functions (all of the remaining 19), co-blocked between SwitchStmt and BreakStmt. Counterfactual is 0/0 — the sole-blocker methodology has no signal here. Validation shape changes accordingly (see §4).

---

## 1. IR representation

**Decision required:** Switch as a first-class IR node, vs. desugar to labeled-block + chained if-else at lower-tree time.

### Option A — first-class `Stmt::Switch`

```rust
Stmt::Switch {
    discriminant: Expr,
    cases: Vec<SwitchCase>,
    default_index: Option<usize>,
}
SwitchCase { test: Option<Expr>, body: Block }  // None test == default
```

Round-trips cleanly (preserve-as-syntactic, consistent with every prior phase). Emit is a direct mapping. SSA needs a new terminator or a desugaring pass at SSA-construction time.

**Pros:** Source fidelity. Matches established pattern. Cheap emit. Lower-tree round-trip is mechanical.

**Cons:** Pushes the hard work into SSA. Switch dispatch in SSA is non-trivial — multiple incoming edges per case, fallthrough edges, default-as-fallback. SSA either gains a `Switch` terminator (real work, cascades into loop_info, dominator analysis, every walker) or desugars at the SSA boundary (and we've moved the problem one layer down).

### Option B — desugar to if-else chain at lower-tree

Switch becomes, at lower-tree time:

```js
// switch (e) { case a: ...; case b: ...; default: ... }
// becomes:
{
  let _disc = e;
  if (_disc === a) { ... /* case a body, possibly falling through */ }
  else if (_disc === b) { ... }
  else { ... /* default */ }
}
```

with break-out-of-switch becoming break-out-of-labeled-block.

**Pros:** SSA stays simple — no new terminator, existing if/else machinery handles it. Downstream passes don't need to know switch exists. Fallthrough is explicit in the desugaring, no special-case in SSA.

**Cons:** Source fidelity lost on round-trip — switch becomes if-chain on output. Phase 11 audit's structural-equality gate would *fail* on every switch unless we add a tolerance, and adding a tolerance for "all switches" is exactly the kind of audit hole we tightened away from. Also: fallthrough desugaring is genuinely tricky (see §3) and gets harder, not easier, when expressed as if-chains.

### Recommendation: **Option A.**

Three reasons:

1. **The structural-equality gate is non-negotiable.** Option B requires either weakening the gate (regression on the phase-11 work) or adding a switch-shaped tolerance that's broad enough to hide real bugs. Neither is acceptable.

2. **The "downstream passes don't need to know switch exists" benefit is theoretical.** We don't currently have downstream passes that would care. We have an SSA IR and an emit path. Adding complexity to SSA to preserve source shape is the trade we've made every prior round and it's been the right one.

3. **SSA Switch terminator isn't as bad as it sounds.** It's a multi-target branch with a known finite set of equality tests. CFG construction handles it as N+1 successor edges (N cases + default). Loop_info doesn't care — switches aren't loops. Dominator analysis treats it identically to a chain of if/else for dominance purposes. The walker checklist from phases 8/10 applies but is bounded.

**One caveat on Option A:** the SSA work *is* real, not preserve-cost. Estimate it as bigger than any prior single-construct ticket. Probably comparable to phase 8's CompoundAssignment work in lines but more conceptually loaded because it's the first new control-flow primitive in SSA since the project started.

---

## 2. Break disambiguation

**Decision required:** How `break` distinguishes break-from-loop vs. break-from-switch.

This is forced by Option A. `break` inside a switch targets the switch; `break` inside a loop targets the loop; `break` inside a switch-inside-a-loop targets the *switch*, not the loop. Same keyword, different IR meaning.

### The disambiguation has to happen at lower-tree time, not parse time

At parse time we don't know the surrounding scope cheaply. At lower-tree time we're walking statements in context and know whether we're inside a switch, a loop, or both. So:

```rust
// Tree IR
Stmt::Break { label: Option<Ident> }  // unchanged

// Lower-tree time, when entering a switch or loop, push onto a context stack:
enum BreakTarget { Loop(LoopId), Switch(SwitchId) }

// On encountering a Break with no label, target = top of stack.
// On encountering a Break with a label, walk the stack to find the labeled construct.
```

SSA emits the right edge based on resolved target. Round-trip back to tree-IR re-emits `break` with optional label — no information loss because at re-emit time we're inside the corresponding switch/loop again and the keyword is unambiguous.

### Round-2 hypothesis check

The round-2 hypothesis was "Break clears once Switch lands." That's only true for break-inside-switch. **Break-inside-loop is a separate construct that this work does not address.** The 19 functions are all switch-using (per the dump), so all 19 Breaks should clear, but if the corpus ever grows a function with a non-switch break-out-of-loop, it'll surface as a new blocker. Don't claim Break is "done" after this round — claim "break-inside-switch is done."

### Continue

Same disambiguation problem doesn't apply: `continue` only targets loops. But while we're here, audit that `continue` inside a switch-inside-a-loop correctly skips the switch and targets the loop. If our current handling assumes break and continue use the same target stack, that's a bug we'd surface during this work. Worth a unit test even if it currently passes.

---

## 3. Fallthrough

**Decision required:** Represent fallthrough explicitly, or reject as unsupported and lower only switches that fully terminate each case.

### What fallthrough actually looks like

```js
switch (x) {
  case 1:
  case 2:
    foo();
    // no break — falls through
  case 3:
    bar();
    break;
  default:
    baz();
}
```

`case 1` falls through to `case 2`'s body (and `case 1` has no body of its own). `case 2`'s body falls through to `case 3`'s body. `case 3` terminates. `default` is reached only if x ∉ {1,2,3}.

### Option A — explicit fallthrough in IR

`SwitchCase::body` is just a Block. The CFG edge from end-of-case-body to start-of-next-case is implicit unless terminated by break/return/throw/continue. SSA construction adds the fallthrough edge automatically when building successors.

Cost: SSA needs to know "what's the next case." Cheap — cases are an ordered Vec.

### Option B — require explicit termination, reject implicit fallthrough

Lowering checks each case body for a terminating statement. If absent, fall back to passthrough for that function. Switches that don't fall through (the common case) lower fine; switches that do fall through stay as PT until we extend.

Cost: incomplete coverage. But: fallthrough is rare. Need to confirm with the corpus dump whether any of the 19 functions actually use it.

### Recommendation: **Option A, with a telemetry note.**

Option B's "rare" claim needs validation, and we have the diagnostic tooling to validate it cheaply. But:

- If we ship Option B and 17 of 19 cases work, we're back to "almost done with switch, two stragglers" which is exactly the state we've been clearing all project. Not worth the savings.
- Option A's cost is one extra edge in CFG construction. That's negligible relative to the rest of the SSA Switch work.
- Fallthrough handling is the part of switch semantics that's most likely to harbor subtle bugs. Doing it now, with the rest of the construct in fresh memory, is cheaper than coming back to it.

### Pre-work diagnostic

Before writing the ticket, run a one-shot dump: how many of the 19 functions use fallthrough, and what shapes? If zero, Option A is free insurance. If non-zero, we know what we're committing to. Either way the answer is Option A; the dump just sizes the testing surface.

---

## 4. Validation shape

Counterfactual is 0/0. The exact-match validation that's worked for phases 8, 9, ticket-`in`/`instanceof`, and RegExpLiteral does not apply.

**Replacement:** post-PR check is "did all 19 PT functions clear, and if not, which didn't and why."

- 19/19 clear → expected outcome, methodology held in a co-blocked-cluster setting (new data point).
- 17 or 18 clear → dump the holdouts. Likely fallthrough we missed (if Option B) or a second blocker we didn't see (genuine round-3-class miss).
- <17 clear → stop and investigate before any further work. Means a design assumption was wrong.

**One additional check** specific to this round: confirm that some non-switch construct didn't regress. The Switch work touches lower-tree statement walking, SSA construction, and CFG edges. A bug there could cause an unrelated function to start passing through. Check: PT count goes from 19 to 0, *not* 19 to 0 with two new entries from elsewhere. The corpus harness already reports total PT, so this is a one-line check, but it's worth being explicit because the surface area is bigger than any prior round.

---

## 5. Implementation order

Three PRs, not one. Switch is too big and too novel to bundle.

**PR 1: Tree IR + parse + emit, no SSA.**
Add `Stmt::Switch`, `SwitchCase`, parser routing, emit, lower-tree round-trip. Function bodies containing switch continue to passthrough (SSA still rejects). Validation: structural roundtrip 308/308 holds, FB-PT% unchanged, every prior unit test passes. This is the safe foundation — if it breaks roundtrip we know the parse/emit layer is the issue, isolated from SSA work.

**PR 2: SSA Switch terminator + CFG + walker checklist.**
Add the SSA construct, the CFG edges (including fallthrough per §3), the break-target disambiguation per §2. Function bodies *still* passthrough at this stage (lower_ssa doesn't yet route Switch through real lowering). This PR is verified by SSA-level unit tests, not corpus impact. Walker checklist applies in full — every walker site verified for the new SSA InstKind/terminator.

**PR 3: Wire lower_ssa to route Switch, validate cluster clear.**
Final step. The 19 PT functions drop to 0. This is where the validation from §4 happens. If anything goes wrong, we know it's in the routing layer, not the IR or SSA layer, because PRs 1 and 2 already passed their own gates.

The temptation is to bundle PR 1 and PR 2 because "it's all switch work." Don't. The phase-7 lesson was that bundling work at different walker depths exposes blockers we couldn't see at planning time. Switch IR (PR 1) and Switch SSA (PR 2) are at different depths; isolating them means the bug-discovery signal is clean.

---

## 6. Out of scope for this design

- **Break-inside-loop.** Not currently a blocker. Will become one if the corpus grows. Don't pre-fix.
- **Labeled break / continue.** The disambiguation in §2 handles labels mechanically, but corpus-validating labeled-break behavior is its own work. Assume it works after this round; ticket separately if a specific function fails.
- **The CFG back-edge bug** (parked from cloneDeep). Switch may or may not exercise the relevant code path. Wait until after PR 3 to revisit; the diagnostic might change.
- **`build_top_level_layout`.** Unrelated, still parked.

---

## 7. Decisions to confirm before ticketing

1. Option A (first-class Switch IR) — confirm.
2. Lower-tree-time break disambiguation with target stack — confirm.
3. Option A on fallthrough (explicit IR), pending one-shot dump for sizing — confirm and run dump.
4. Three-PR split (Tree, SSA, Wire) — confirm.

If any of these is wrong, this is the point to push back. Once these are settled, the tickets are mechanical: PR 1 is small, PR 2 is the algorithmic work, PR 3 is small.
