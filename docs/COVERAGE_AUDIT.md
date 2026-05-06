# Coverage diagnostic — what it measures, what it doesn't

The blocker-analysis table emitted at the bottom of
`corpus/realworld/STABILITY_REPORT.md` (and any per-file
`--coverage-stats` output) is a planning aid, not a forecast.

## What FB-PT% measures (and what it doesn't)

`FB-PT%` (function-body passthrough percent) and the `PT functions`
count are computed on the **initial tree-IR** produced by
`parse_and_lower`, before any SSA lowering happens (see
`crates/cli/src/coverage.rs::analyze`). The metric reports AST-level
disposition: a function is "Lowered" if its construct set is in the
supported subset and "PassThrough" otherwise.

**It is not a measure of SSA optimization coverage.** Several
constructs lower into tree-IR cleanly but route through SSA-fallback
(`body_needs_ssa_fallback` in `lower_ssa.rs`) at SSA-build time,
producing a `FunctionBody::PassThrough(source_slice)` that is invisible
to every SSA-level pass (constant prop, DCE, escape analysis, free-var
analysis, etc.):

- `Stmt::Throw` (round 10)
- `Stmt::Try` (since project start)
- `Stmt::ForIn` (round 9)

Constructs that previously fell back but now lower through real SSA:

- `Expr::This` — un-fell-back in round 11 (PR #11; the cloneDeep
  CFG-back-edge bug got fixed by adding a reachability precondition
  to `dominates`).
- `Stmt::Switch` and `Stmt::Break` — un-fell-back in the
  Real-SSA-Switch ticket (Issue #11). Real `Terminator::Switch` with
  break-target stack lives in `lower_ssa::lower_switch`; SSA →
  tree-IR reconstruction lives in `lower_tree::walk_inner`'s Switch
  arm. See "Deferred SSA Switch shapes" below for what the
  implementation does NOT yet handle.

A function using any of these reads as Lowered in the AST disposition
column and counts as 0 toward FB-PT%, but is functionally identical to
the old `Item::PassThroughFunction` shape from the optimizer's point of
view: the body is a verbatim source slice that re-parses at emit time
via `oxc_codegen`. The metric improvement is real — emit handles it,
the rename pipeline sees a single named entity rather than an opaque
top-level item, the function participates in the source-order layout —
but it does not mean the body went through SSA.

If FB-PT% is the input to a planning decision about *optimization*
coverage, subtract the SSA-fallback set first. If it's the input to a
decision about *emit / round-trip* coverage, take it at face value.

When a future ticket promotes one of these constructs from SSA-fallback
to a real SSA representation (e.g., the follow-up `Terminator::Switch`
ticket), this list shrinks and the FB-PT% metric stops over-counting.
Until then: caveat lector.

## Two columns, both bounded in scope

The table reports per `BlockerKind`:

- **`occurrences`** — the number of currently-PassThrough functions that
  contain at least one instance of this blocker. A frequency stat. Cheap
  to interpret: a high occurrence count means the kind is common across
  the corpus.
- **`counterfactual`** — the number of currently-PassThrough functions
  where this blocker is the *sole* member of the function's blocker set.
  These are the functions a single-construct lowering ticket would
  directly unlock if the kind were the next thing implemented.

Both views are computed against the disposition walker as it exists
*right now*, on the corpus as it exists *right now*. They describe
where things stand; they do not predict where things go.

## Why no cumulative-unlock projection

A previous version of the table also reported a greedy cumulative
projection: "if we fix kinds in counterfactual-descending order, how
many functions unlock at each step?" The formula was correct given its
inputs. The inputs were the problem: per-function blocker sets are
not stable across lowering rounds, so projecting future outcomes
against today's snapshot systematically over-predicts.

Two structural reasons it under-counts blockers:

1. **Walker recursion expands when a construct is lowered.** Most
   "node-with-children" blockers (e.g., `AssignmentExpr`, `SequenceExpr`)
   are emitted at a node where the disposition walker stops descending
   — the kind is the terminal blocker for that subtree. Lowering the
   kind almost always entails updating the walker to start recursing
   into its children, which surfaces blockers that were previously
   invisible. Functions the projection counted as "single-blocker, will
   unlock" can be revealed to have additional nested blockers
   post-lowering, so they stay PassThrough.

2. **Coarse blocker buckets get split as kinds gain attention.** The
   `UnsupportedExpr` bucket once covered plain `=`, compound `+=`,
   logical `&&=`, object-property weirdness, and the JS-spec
   catch-all. The projection treated the bucket atomically; lowering
   one of its constituent constructs only partially drains it.

## Empirical demonstration: round 3

PR #2 (`feat: lower SequenceExpr, AssignmentExpr, ForInStmt`,
commit `1dc96ab`) is the canonical demonstration. Pre-PR projection
predicted ~228 functions unlocked through `AssignmentExpr`. Actual
movement out of PassThrough: ~117. The remaining gap was the 176
occurrences of what is now `CompoundAssignmentExpr`, previously
hidden inside the coarse `UnsupportedExpr` bucket and inside
non-recursing assignment-expression handling.

The mechanism is provable from the walker's git history:

- At report-projection time (`829d6f1`),
  `js::Expression::AssignmentExpression` fell through to a terminal
  `_ => UnsupportedExpr` arm. The walker did not descend into
  target/value; nested blockers were invisible.
- Two commits later (`5a80508`), AssignmentExpression got a named
  handler that recurses into its children, and the `UnsupportedExpr`
  bucket was split.
- In `1dc96ab`, the plain-`=` lowering removed `AssignmentExpr` from
  the blocker set entirely, leaving `CompoundAssignmentExpr` to gate
  the same functions the old projection had counted as unlocking.

Round 1 (`MultipleDeclarators`, ~60 predicted, ~60 actual) held only
because lowering MultipleDeclarators required no walker-recursion
change — it is a *property* of a declaration, not a node with
children, and the walker already descended into per-declarator init
expressions.

## Implications for planning

- **Single-construct tickets driven off `counterfactual` are safe.**
  A function with exactly one blocker today either gets that blocker
  lowered (and unlocks) or doesn't (and stays PT). The failure mode is
  bounded by the single kind. The only edge case is if the lowering
  splits the sole blocker into finer kinds — that's recoverable.
- **Multi-construct tickets cannot be sized off projection alone.**
  Each round's actual unlock must be measured against the post-PR
  report, not the pre-PR projection. Plan one construct at a time, or
  bundle on intuition + small-scope rebase budget.
- **Parallel-agent fan-out across multiple constructs at once should
  not be sized off projection.** Cumulative projection's failure mode
  scales with the number of constructs in flight: every additional
  construct is another opportunity for walker refinement to expose new
  blockers and shrink the realized unlock.

## What to do if you want a multi-construct projection anyway

Don't extrapolate from this table. Instead:

1. Implement the lowerings on a feature branch.
2. Regenerate the stability report.
3. Compare PT counts directly.

Or, less rigorously, use the per-function blocker sets (available via
`coverage::analyze` programmatically) to enumerate functions that
*currently* show only the constructs in your planned bundle as
blockers. That set is the upper bound on what your bundle can unlock —
which is closer to the truth than the greedy cumulative was, but still
optimistic for the same walker-recursion reason.

## Deferred SSA Switch shapes

The Real-SSA-Switch ticket landed a `Terminator::Switch` that handles
the corpus shape — every observed switch in `corpus/realworld` lowers
fully through SSA without fallback. The following shapes are
*deliberately* not implemented; each will be ticketed if the corpus
grows the relevant pattern. Re-run `dump_switch_fallthrough.rs` to
verify zero before assuming they're still safe to defer.

- **Fallthrough.** Source: a case body without `break`/`return`/
  `throw`/`continue` falls through to the next case. Currently 0
  in corpus. Today, `lower_switch` panics if it sees a non-terminating
  case body (loud failure). Adding fallthrough means a
  `fallthrough_edges: Vec<(BlockId, BlockId)>` field on
  `Terminator::Switch`, an explicit `Branch{next_case_body}` from
  the end of each non-terminating case, and matching reconstruction
  in `lower_tree`.

- **`default:` not at the end.** Source: `case 1: ...; default: ...;
  case 2: ...`. Currently 0 in corpus (no defaults at all in
  obfuscated dispatch). Today's reconstruction emits the default
  case last in the round-tripped output regardless of source
  position. To preserve position, the SSA terminator would need to
  remember the source-order index of the default case alongside the
  `default: BlockId` field.

- **Labeled break/continue out of switch.** Source: `outer: for (...) {
  switch (x) { case 1: break outer; } }`. Currently 0 in corpus
  *and* still blocked at AST disposition by `LabeledStmt`. The
  break-target stack in `lower_ssa::Converter` doesn't yet walk for
  matching labels — when LabeledStmt is removed from the blocker
  set, the stack walk needs to be added.

- **Side-effecting / non-constant case keys.** Source: `case foo():
  ...`. Currently 0 in corpus — every observed case key is a number
  or string literal. Today, `lower_switch` panics if
  `case_key_constant` returns None. Supporting non-constant keys
  needs a different lowering shape (chain of `cbr eq` instead of a
  single Switch terminator) plus a fallback for keys that mix
  constant and computed.

- **String case keys via the free helper.** Today the free function
  `expr_to_constant` declines string literals because
  `Constant::String` requires `StrId` interning. The Converter
  method `case_key_constant` handles strings by interning at the
  call site. If a future caller wants to convert constants outside
  the Converter, it must either intern up-front or accept the
  string-key limitation.

If any of these go from "0 in corpus" to non-zero, that's the trigger
to ticket the shape — with the relevant section above as the starting
point.
