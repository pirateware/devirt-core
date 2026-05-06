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
