# Workspace Refactor — Decisions Log

Captures deviations from the original five-crate split plan and any
cross-crate import oddities surfaced while migrating mvp into ir +
parse + emit + pass + cli.

## Crate move order: emit before parse

The plan listed move order parse → emit → pass. The actual order
shipped was emit → parse → pass.

`crates/parse/src/lower_tree.rs` carries inline `#[cfg(test)] mod
tests` blocks that drive a full SSA → tree-IR → JS round-trip via
`crate::emit::emit`. With parse moved first, parse would have needed
to depend on the still-mvp-resident emit module — a cycle (mvp depends
on parse, parse would depend on mvp). Moving emit first sidesteps the
cycle: emit only consumes `ir::tree`, never anything in parse, so it
can land independently and parse picks it up as a dev-dependency.

## error.rs lives in the ir crate, not parse

The plan said error.rs should land in parse "if parse-specific" or in
ir "if shared." It is shared: the pass manager in `crates/pass/`
constructs `Error::PassFixedPointNotReached` and
`Error::PassValidationFailed`, and parse builds `Error::Parse`,
`Error::Unsupported`, `Error::Malformed`. Putting the type in `ir`
gives both consumers a common dependency without a parse↔pass cycle.
ir itself does not use `Error`; it merely hosts the module so
downstream crates can import `ir::error::Error`.

## `parse_and_lower_with_decider` widened from pub(crate) to pub

This entry point existed solely as a test seam: production always
calls `parse_and_lower`, which delegates with `disposition::decide`.
The seam exists so `parse/tests/pipeline.rs` (formerly
`mvp/src/tests.rs`) can inject hand-crafted "lying" walkers to
exercise the asymmetric debug cross-check in
`lower_top_function_with_decider`.

The plan flagged any new `pub` exposure as a signal worth pausing on.
The honest fix would be to keep this private and host the test inside
the parse crate as an inline `#[cfg(test)] mod tests` rather than as
an integration test. The integration-test placement was chosen here
to keep the move of `mvp/src/tests.rs` mechanical (it was already an
external file consuming the seam). When that test family later moves
inline, this widening can revert to `pub(crate)`.

## node-tests feature plumbing

mvp originally owned the `node-tests` feature gating
behavior-preservation tests that shell out to `node`. After the
split the gated tests live in two places: parse's inline lower_tree
tests, and cli's integration suites. Both crates declare their own
`node-tests` feature. cli's `node-tests` enables `parse/node-tests`
so a single `cargo test --features cli/node-tests` activates the
full set; parse/node-tests can also be enabled directly.

## Cross-crate imports — audit

After all moves the dependency graph is:

```
        ir
       /  \
      /    \
   emit   parse  (parse has dev-dep on emit for inline tests)
    \      /
     \    /
      pass (depends only on ir)
       |
       +-- cli depends on ir, parse, emit, pass
```

ir is a leaf with no workspace dependencies. emit and pass each
depend only on ir. parse depends on ir; its dev-dependency on emit
is required by the inline round-trip tests in lower_tree.rs and is
intentional. cli is the integration point that pulls everything
together. No cycles. emit does not import from parse, parse does
not import from pass, and nothing imports from cli.

## Orphaned regression test

`crates/cli/tests/regressions/unary_neg_panic.rs` is reachable as a
file but is **not** auto-discovered by `cargo test`. Cargo's
integration-test discovery only looks at files directly under
`tests/`, not nested directories. This matches the file's
pre-refactor state (it lived at the repo-root `tests/realworld_
regressions/` and was likewise not part of any crate's auto-
discovered set). If the team wants it to run, either flatten the
directory or add an explicit `[[test]]` entry to
`crates/cli/Cargo.toml`. The refactor preserves the prior
behavior (orphaned) deliberately.

## Test count

Baseline before refactor: 121 tests passing under
`cargo test --features mvp/node-tests` (plus one pre-existing
failure in `realworld_stability::test_tier1_corpus`, an
unrelated unary-negation lowering panic tracked separately).

After refactor: 121 tests passing under
`cargo test --workspace --features cli/node-tests --no-fail-fast`
distributed as 3 (ir) + 41 (parse lib) + 37 (parse pipeline
integration) + 10 (pass) + 30 (cli functional_equivalence). Same
pre-existing failure preserved.

`--no-fail-fast` is required to see all crates because cargo test
aborts the workspace traversal at the first failing target;
without it, parse/pass test runs are skipped due to the
pre-existing realworld_stability failure ordering before them.
