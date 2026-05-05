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
