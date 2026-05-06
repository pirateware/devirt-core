# ForIn — design

ForIn lowering uses **Option A** from the design conversation: three
first-class iteration InstKinds plus a regular while-loop CFG. Per-
instruction iteration semantics are preserved in the IR; reconstruction
pattern-matches the four-block shape and re-emits `for (k in obj) body`.

## Why Option A

The corpus has 10 ForIn instances backing 2 unique source functions
(`lodash_cloneDeep::arrayLikeKeys` and `baseKeys`). Three options were
considered:

- **A — first-class InstKinds (chosen).** Three new `InstKind`s:
  `ForInInit`, `ForInDone`, `ForInNext`. CFG is the standard four-
  block while-loop pattern. Reconstruction is mechanical.
- **B — desugar to `Object.keys(obj)` + for-loop.** No new InstKinds,
  reuses existing call/loop machinery. **Disqualified** by semantic
  divergence: `Object.keys` returns own keys only, materializes
  eagerly, and skips Symbol-bridge cases that for-in includes. Round-
  trip would be correct (re-emits `for (k in obj)` from the pattern),
  but any future SSA pass operating on the desugared form would be
  wrong on prototype-chain iteration.
- **C — structural framing with `Terminator::ForIn`.** Doesn't fit
  cleanly because for-in iteration distinguishes init / done / next as
  three operations, which a single Terminator can't carry without
  conflating instruction-level and CFG-level concerns. Mentioned for
  completeness; rejected on the same grounds Option 2 was rejected for
  Try (doesn't model the construct's semantics).

The trade-off cuts: A's IR-surface cost (~150 lines) is disproportionate
to current corpus weight, but B's semantic divergence is a permanent
commitment in the IR. Option A is reversible (passes can ignore the
opaque InstKinds); Option B isn't (passes inherit the divergence).

## §1 — IR surface

```rust
InstKind::ForInInit { obj: Operand },     // returns iterator handle
InstKind::ForInDone { iter: Operand },    // returns Bool (true = exhausted)
InstKind::ForInNext { iter: Operand },    // advances; returns next key (string)
```

All three are `Effect::OPAQUE`. A future `MAY_OBSERVE_GETTERS` flag
would refine — getters / Proxy traps / mutations during iteration —
when an optimization pass demands the precision.

## §2 — Construction shape

```
pre_header (current_block before lowering ForIn):
  iter = ForInInit(obj)
  Branch header

header:
  done = ForInDone(iter)
  CondBranch done, exit, body          // then=exit (taken when done), else=body

body:
  key = ForInNext(iter)                // tagged with source-name = user's binding
  ...write key to user's binding symbol...
  ...lower user's body stmts...
  Branch header                        // back-edge

exit:
  // continue after for-in
```

The CondBranch ordering (`then=exit, else=body`) is intentionally
reversed from the typical `lower_while` shape. This avoids needing a
`Not` instruction in the header — `ForInDone` returns "exhausted",
which is exactly the "stop iterating" condition.

The `binding` source-name is set on the `ForInNext` result Value via
`set_source_name`, mirroring how `lower_for_of` sets the user's loop
binding name.

## §3 — Reconstruction

`detect_forin_loop` (in `lower_tree.rs`) pattern-matches:
- Header has exactly one inst: `ForInDone(iter)`.
- CondBranch's cond is that inst's result; `then_b == info.exit_b`,
  `else_b == info.body_b`.
- Body's first inst is `ForInNext(iter)` with the same iter operand.
- Pre-header's last inst is `ForInInit(obj)` whose result equals iter.

If matched, `emit_for_in` produces `Stmt::ForIn { kind: "var",
binding, obj, body }`. The corpus uses `var` consistently for `for-in`
LHS; if a future for-in needs let/const distinction, thread the kind
through via a side-channel (corpus doesn't motivate it now).

## §4 — Body restriction

Phase A of ForIn supports only **straight-line bodies with a single
back-edge**. A ForIn body containing further control flow (if /
nested loops / switch) inside the body would have a non-back-edge
terminator on `body_b` itself, which `emit_for_in` currently panics
on with a clear diagnostic. The corpus's two functions
(`arrayLikeKeys`, `baseKeys`) both fit the simple case.

If a future ForIn-using sample lands with body control flow, extend
`emit_for_in` to walk the body's CFG up to the back-edge (mirror
`emit_loop`'s body-walking logic).

## §5 — `kind` field

Always emit `"var"` at reconstruction. The corpus uses `var` for
all 10 instances; ES5 semantics make var/let/const indistinguishable
at function scope (which is the corpus shape — Babel/esbuild
compile-down to ES5). If future input requires let/const distinction,
add a `kind: u8` field to `ForInInit` and thread through.

## Out of scope (future work)

- Body with internal control flow (see §4).
- `MAY_OBSERVE_GETTERS` lattice flag (see §1).
- Destructuring / multi-declarator / non-declaration LHS — disposition
  blocks these at parse time (see disposition.rs).
- For-of unification: ForOf currently desugars at lower_ssa time using
  the iterator protocol (real JS calls). ForIn uses opaque InstKinds
  because for-in has no public iteration protocol. Unifying these is
  not currently motivated.
