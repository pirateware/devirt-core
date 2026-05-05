# Architectural Decisions

## Top-level non-function statements use a source-slice PassThrough fallback

**Date:** 2026-05-05

**Context.** The SSA model is function-centric: instructions and blocks
live inside `ssa::Function`, and `ssa::Module` is essentially a
collection of functions. Before this change, any `Item::Stmt` at module
top-level was discarded by `lower_ssa::discover_module` and never
re-emitted by `lower_tree::lower_module`. The result was silent data
loss for top-level IIFEs, expression statements, top-level `for`,
top-level `class`, etc. — the input parsed, the IR survived round-trip
through `parse_and_lower → emit`, but the SSA round-trip produced an
empty output with no error.

**Decision.** Top-level non-function statements are preserved as
verbatim source slices and threaded through the SSA round-trip:

1. `lower.rs::lower_top_stmt` slices the source for any non-`FunctionDeclaration`
   top-level statement and emits `Item::PassThroughStmt(String)`.
2. `lower_ssa::lower_module` builds a `top_level_layout: Vec<TopLevelEntry>`
   alongside `sm.functions`, preserving source-order interleaving.
3. `lower_tree::lower_module_with_config` walks `top_level_layout` to
   re-emit functions and passthrough statements in the original order.
4. `emit.rs` writes `Item::PassThroughStmt` verbatim.

This mirrors the existing `FuncBody::PassThrough` strategy used for
un-lowerable function bodies — "anything we can't model in IR is
preserved as source text" is now applied uniformly at top-level too.

**Consequence.** Top-level `class Foo {}` and `for (...)` no longer
produce `Error::Unsupported`. Tests `unsupported_class_errors_loudly`
and `unsupported_for_errors_loudly` were replaced by
`top_level_class_passthrough` and `top_level_for_passthrough`. The
"errors loudly on unsupported" guarantee still applies inside Lowered
function bodies via `disposition::decide`.

## Audit: similar "only handle named decl" patterns

Scanned for similar silent-drop patterns. Findings:

- **`lower.rs::lower_top_function_with_decider`** errors on anonymous
  top-level function declarations (`function() {}` with no name). This
  is correct — anonymous *declarations* (not expressions) are
  syntactically illegal in plain script context; OXC accepts them only
  inside `export default`, which is outside our subset. Erroring is
  the right call.
- **`lower_ssa.rs::discover_stmts` / `walk_*`** has many `_ => {}` arms,
  but these are neutral cases (`Constant`, `EmptyStatement`,
  `BreakStatement`, etc.) — not silent drops of meaningful constructs.
- **`pt_free_vars.rs`** has multiple `_ => {}` arms while walking
  passthrough function bodies for free variables. These are scanned for
  audit completeness; none silently drop a top-level item. The
  walker's job is conservative — missing a node means treating it as
  not-a-reference, which is recoverable (over-approximates capture set,
  not under).

No additional silent-drop hazards found at module top-level.

## C-style `for` loops: continue jumps to a per-loop step block; break unsupported

**Date:** 2026-05-05

**Context.** Round 3 of the coverage audit (`docs/COVERAGE_AUDIT.md`)
called for lowering C-style `for (init; cond; step) body`. The
ticket's spec called out a corner case: `continue` inside the body
must still execute `step` before re-checking `cond`. A naive desugar
to `{ init; while(cond) { body; step; } }` skips `step` on
`continue` and infinite-loops.

**Decision.** Tree-IR grew an explicit `Stmt::For { init, cond, step,
body }` (front-end produces it; SSA-back never reconstructs it — the
SSA shape re-emits as `while`). SSA lowering builds a CFG with a
distinct *step block* between the body and the back-edge:

```
pre  ──init──▶ header ─cond─▶ body ─fallthrough─▶ step ─back-edge─▶ header
                  │                                            (if cond false)
                  └──────────────────────▶ exit
```

A per-loop continue-target is pushed on a stack at lower-time; the
new `Stmt::Continue` IR node lowers to `Branch{continue_target}` —
which for a `while` is the header (existing behavior preserved) and
for a `for` is the step block. SSA→tree's loop-shape recognizer
handles this naturally: `continue` becomes a then-branch that
returns control to the synthesized join (= step block), which
re-emits as a regular `while` body containing the original body
followed by the step.

`break` is **not yet supported**. The disposition walker rejects
`BreakStatement` so loops containing `break` fall to PassThrough.
The user's Round 3 test suite doesn't exercise `break`, and adding
support requires SSA→tree's `walk` to recognize jumps to the loop's
exit block as `break` statements (currently `walk` only special-cases
the loop *header* via the `stop` parameter). Deferred to a follow-up
ticket.

## `++` / `--` use generic `+`/`-`: numeric inputs match JS, non-numeric inputs diverge

**Date:** 2026-05-05

**Context.** JS spec for `x++` is `x = ToNumber(x) + 1` — the
operand is coerced to a number first. Our lowering for
`Expr::Update` emits `AddGeneric` / `SubGeneric` which is the same
instruction `+`/`-` use; it follows JS's generic operator coercion,
which for `+` includes string concatenation when one operand is a
string.

**Decision.** Accept the divergence for non-numeric operands. For
numeric inputs the result matches strict semantics. For
string-typed `x`, `x++` would set `x` to `"<old-value>1"` (string
concatenation) under our model, vs. `<numeric-value-of-x> + 1`
under strict JS. This is a correctness gap, but obfuscator-emitted
code in the wild does not perform `++` on non-numeric operands;
addressing it would require either an explicit `ToNumber` IR step
or a dedicated numeric `Inc`/`Dec` instruction kind. Deferred until
a real corpus shows a case that depends on the strict behavior.

## Compound assignment on property targets: receiver/key in synthesized `__tN__` temps

**Date:** 2026-05-05

**Context.** `obj.prop += rhs` and `arr[k] += rhs` must evaluate
`obj` (and `k`) exactly once, even when those expressions are
side-effecting. The literal desugar `obj.prop = obj.prop + rhs`
re-evaluates `obj` twice — wrong if `obj = sideEffect()`.

**Decision.** Front-end (`lower.rs::lower_compound_assign`) emits a
`Stmt::Block` introducing one or two synthesized `let __tN__ = ...`
bindings before the read+write. The block scopes the temps to the
compound-assignment site so they don't leak. Names use the
`__tN__` shape with a function-wide counter on the `Lowerer`. A
collision with user code is theoretically possible but vanishingly
rare; the renamer's standard suffix-disambiguation (`__t0__` vs
`__t0___1`) handles it if it ever arises.

The same single-evaluation rule applies to `++`/`--` on property
targets, but those handle it directly in `lower_ssa.rs`'s
`convert_update` — the IR exposes `Expr::Update` so the SSA
lowering has the imperative context it needs to evaluate
sub-expressions in source order without synthesizing tree-IR temps.
