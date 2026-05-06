# Try Phase A — design

This is the durable reference for the Try Phase A implementation
(Terminator::Try framing block + InstKind::CatchValue + catch-handler
stack at AST→SSA time, no finally support).

The full ticket (motivation, validation criteria, commit structure,
acceptance criteria, time budget) lives in the conversation thread
that produced this work; this doc captures the decisions that future
maintainers will need to read code with.

## Decision: structural framing (Option 3), not per-instruction edges

Three viable shapes were considered:

1. **Per-instruction exception edges (LLVM `invoke`-style):** every
   potentially-throwing instruction in a try-body terminates a block
   with two successors (normal + catch). Faithful, heavy.
2. **Explicit-throw-only routes to catch; implicit throws ignored:**
   simpler, but **wrong** on the corpus — obfuscator.io's
   selfDefending wrappers depend on catching implicit throws (a `dbg()`
   call that throws when DevTools is open). Modeling those as
   not-throwing makes the catch unreachable, which would let DCE
   delete it, defeating the obfuscator's self-defense — i.e., turning
   anti-debug back on. Disqualified.
3. **Structural framing (chosen):** the framing block (the one whose
   terminator is `Terminator::Try`) declares the relationship
   "anything that throws between `try_entry` and `join` is caught at
   `catch_entry`." Per-instruction edges are not represented; the
   framing block's `successors()` returns `[try_entry, catch_entry]`.

## §1 — IR surface

```rust
pub enum Terminator {
    // …
    Try {
        try_entry: BlockId,
        try_args: Vec<Operand>,
        catch_entry: BlockId,
        join: BlockId,  // landmark, not a successor
    },
    // …
}

pub enum InstKind {
    // …
    CatchValue,  // result type: any. Effect: Opaque.
                 // MUST be the first instruction of catch_entry.
}
```

`CatchValue` is an instruction (not a block-arg) so it doesn't
conflate with phi semantics. Catch entry has zero edge args; the
catch-param symbol is bound to the `CatchValue` inst's result.

## §2 — Catch-handler stack at AST→SSA time

Mirrors the break-target stack pattern PR #18 introduced for
Switch+Break. `Converter` holds:

```rust
catch_stack: Vec<CatchFrame>,
struct CatchFrame { catch_entry: BlockId, join: BlockId }
```

Push on entering `Stmt::Try`; pop on exit. Inside try-body, `Throw`
emits `Terminator::Throw { val }` unchanged; the framing block
declares the catch relationship structurally.

## §3 — Effects lattice

`CatchValue` is Opaque purity. A future `READS_EXCEPTION_STATE` flag
is the right precision tier when an optimization pass demands it;
until then, Opaque is honest. Per the project's standing rule on
effect-lattice precision, this is NOT a "collapse precision as a
shortcut" — it's the correct tier for "frontend lacks evidence."

## §4 — Implicit-throw approximation: what we lose, what we keep

Throws from inside try-body to catch_entry are **not** modeled as
per-block CFG edges. Practical consequences for SSA:

1. Catch cannot see SSA values defined inside try-body (no path from
   body to catch in the CFG).
2. Variables that catch reads which are mutated inside try-body must
   be **memory-resident** across the try boundary — they go through
   the **try-address-taken set**, similar to the existing
   `pt_referenced_names` mechanism.
3. Future optimization passes that move instructions across try
   boundaries would be unsound under this approximation. There are no
   such passes today; any future pass that does this either consults
   the address-taken set or waits for Option 1's per-instruction
   precision.

## §6 — body_needs_ssa_fallback predicate (post-Phase-A)

```rust
Stmt::Try { finally: Some(_), .. } => true,            // Phase B
Stmt::Try { catch: None, .. } => true,                  // try-finally only
Stmt::Try { body, catch: Some((_, cb)), finally: None } => {
    body.iter().any(visit_stmt) || cb.iter().any(visit_stmt)
}
```

## Phase B (finally) — explicitly out of scope

Finally adds:
- Three abrupt-completion edges through finally (normal-completion,
  catch-completion, return-through-try) × possible re-throw inside
  finally.
- Finally-block duplication or dispatch-variable encoding for each
  abrupt-completion edge.
- Throw-in-finally overriding pending exception.

Corpus: 91 finally-sites total, all in tier1+tier2 (production
library code: axios, react, vue, jquery, lodash). Zero in tier3+tier4
(obfuscator.io's selfDefending uses bare `try { } catch (e) { }`).
Phase B is for production-library deobfuscation; not blocking the
malware-class goal.

## Phase A → Phase B upgrade path

Phase B does not invalidate Phase A's IR. Adding finally support
means:
- Extending `Terminator::Try` with an optional `finally_entry: Option<BlockId>`
  (or a separate `Terminator::TryFinally` variant if the finally
  case warrants distinct walker handling).
- Adding finally-completion bookkeeping at the catch-handler-stack
  level.
- The `CatchValue` instruction is unchanged.

The `try_entry` / `catch_entry` / `join` landmarks stay; finally is
inserted as additional structure between `try`/`catch` tails and
`join`, without altering existing fields.

---

# Phase B addendum (PR #22, post-merge)

Phase B extends `Terminator::Try` with `finally_entry: Option<BlockId>` and migrates `catch_entry: BlockId` to `Option<BlockId>`. The framing now covers all three shapes:
- try+catch (Phase A)
- try+catch+finally (Phase B)
- try+finally (no catch — Phase B)

## Abrupt-completion approximation

JS runs `finally` before any abrupt completion: `return`, `throw` (when uncaught or rethrown from catch), `break`/`continue` through try. Phase B does **not** model this with block duplication or a dispatch variable. Instead:

- The IR has a finally region. Lowered SSA. Walked, validated.
- Reconstruction emits `try { } catch (e) { } finally { }` JS source. The JS runtime handles abrupt-completion semantics exactly per ECMA-262.
- Same trade-off as Phase A's implicit-throw approximation: SSA optimization passes that move instructions across try boundaries (none exist) would be unsound; for round-trip correctness, JS runtime semantics carry us.

This means: a `Stmt::Return` inside try-body emits `Terminator::Return` directly (no edge to finally in the CFG). The reconstruction emits `return …;` inside `try { }` and the JS runtime runs finally. **The IR lies about control flow but the emitted JS is correct.** Future passes that need accurate control flow must consult `Terminator::Try`'s finally_entry as a sentinel.

## Address-taken extension

Phase A's collector registered "names assigned in try-body" because catch reads them. Phase B extends to handle finally:
- Symbols assigned in try-body remain address-taken (catch and/or finally may read them).
- Symbols assigned in catch-body are address-taken **only when finally is present** (finally can read catch's mutations). Without finally (Phase A shape), catch-body writes don't register — they're in catch's own scope.
- Symbols assigned in finally-body don't register — no consumer reads finally's mutations from outside.

Implementation: same merge into `pt_referenced_names` at construction time. Existing pt_referenced_names machinery handles the StoreProp(GlobalThis, ...)/LoadProp pattern.

## Out of scope (future work)

- **Block duplication or dispatch-variable encoding** for faithful abrupt-completion modeling. Would matter for an optimization pass that wants to reason about which finally-paths a return takes. No pass exists.
- **`READS_EXCEPTION_STATE` lattice flag** — same as Phase A, deferred until a pass needs the precision.
- **finally that itself contains complex control flow** (try/throw/return inside finally) — works structurally because all sub-bodies recurse through `body_needs_ssa_fallback`, but the SSA shape gets tangled enough that future maintenance may want a re-think. The corpus has no such cases.
