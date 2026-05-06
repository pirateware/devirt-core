# Fuzzing Harness — Design

**Date:** 2026-05-06
**Status:** Approved (pending user spec review)
**Owner:** vasie1337

## Motivation

The real-world corpus (`corpus/realworld/{tier1,tier2,tier3}`, ~308 files) is the
primary compatibility signal today. It catches what real minifier and obfuscator
output looks like, but it has two structural limits:

1. **Closed set.** It only exercises shapes that happen to occur in the bundles
   we vendored. Adjacent shapes — same constructs in slightly different
   arrangements — are entirely untested.
2. **Weak oracles past tier 1.** Tier 2/3 only check structural roundtrip.
   Idempotence (a second roundtrip producing the same tree) is not checked
   anywhere, so silent drift between passes is invisible.

The roundtrip property of the pipeline (`parse → lower_ssa → lower_tree → emit
→ reparse → structural_diff`) is a near-perfect fuzz oracle: any input that
trips it is a real bug. Coverage-guided fuzzing turns that property into a bug
discovery engine.

## Scope

In scope:

- A coverage-guided fuzz target over the existing pipeline.
- An AST-aware mutator seeded from the real-world corpus.
- A panic-and-property oracle bundle (panic / structural roundtrip / SSA
  validate / idempotence).
- A regression corpus where minimized crash inputs become permanent test cases
  alongside the existing `realworld_stability.rs` suite.
- CI integration: a time-boxed pass per PR plus a nightly long run.

Explicitly out of scope:

- Behavioral equivalence under `node` inside the hot fuzz loop. Too slow.
  Behavioral check is run separately on the minimized crash corpus only.
- A hand-rolled JS grammar generator. Approach C (AST-aware mutation of the
  corpus) is sufficient and avoids the maintenance burden of a grammar.
- Differential fuzzing against another deobfuscator or minifier. Possible
  future work; not in this spec.

## Architecture

A new crate `crates/fuzz/` following the standard `cargo-fuzz` layout:

```
crates/fuzz/
  Cargo.toml          # publish = false; uses libfuzzer-sys
  fuzz_targets/
    roundtrip.rs      # the single fuzz target
  src/
    mutator.rs        # AST-aware mutator: seed selection + mutation ops
    oracles.rs        # the four properties; each panics on violation
    seeds.rs          # loads tier1/tier2 corpus paths at build time
corpus/
  fuzz_seeds/         # curated initial corpus for libFuzzer
  regressions/        # minimized crash inputs, checked in
```

The fuzz target is a single function:

```rust
fuzz_target!(|data: &[u8]| {
    let Some(js): Option<String> = mutator::generate(data) else { return };
    oracles::check_all(&js);
});
```

`mutator::generate` returns a JS source string (or `None` to skip). It
interprets the libFuzzer byte stream as a seeded RNG: the first few bytes
pick a seed file index, the remainder drives mutation-operator choices. It
loads the seed file, applies the sequence of AST edits, and re-emits to a JS
string via the existing `cli::emit::emit` path. Returning `None` (e.g.
nonsensical instruction stream) is how we tell libFuzzer the input was
uninteresting — it discards it from the corpus.

This indirection — bytes → AST mutations rather than bytes → source — is the
key to keeping cycles productive. ~5% of raw byte mutations to JS produce
parseable input; ~99% of AST mutations do.

### Mutator

`mutator.rs` is the core. It walks an `oxc` AST and applies one of N mutation
operators chosen by the libFuzzer-supplied byte stream interpreted as a seeded
RNG.

**Tier 1 operators (initial release):**

- `DeleteStatement` — pick a Stmt in any block, drop it.
- `DuplicateStatement` — pick a Stmt, clone it in place.
- `SwapStatements` — swap two sibling Stmts.
- `SwapBinaryOp` — replace a binary op with another in its class
  (`+`↔`-`↔`*`↔`/`, `&&`↔`||`, `==`↔`!=`↔`===`↔`!==`, `<`↔`<=`↔`>`↔`>=`).
- `NegateCondition` — wrap an `if`/`while`/`for` test in `!(...)`.
- `RenameReference` — replace an identifier reference with another identifier
  visible in the same scope.

**Tier 2 operators (initial release, subset):**

- `WrapInBlock` — wrap a statement in `if(true){...}` / `for(let i=0;i<1;i++){...}` /
  `try{...}catch(e){}`.
- `VarKindSwap` — toggle `var` / `let` / `const` on a declaration.

These two tier-2 ops are included in v1 specifically because they exercise
SSA-fallback paths and closure disposition (per
`memory/project_t2-status.md`) — areas with known active development.

**Deferred to v2 (added once v1 stops finding new bugs):**

- Cross-file subtree splicing.
- Function form conversions (decl ↔ expr ↔ arrow).
- Expression ↔ statement form conversions.
- Comma-expression / sequential-statement conversions.

### Oracles

Each oracle is a function that runs the pipeline and panics on violation. All
panics propagate to libFuzzer as crashes.

1. **No panic.** Implicit; libFuzzer catches any panic in the pipeline itself.
2. **SSA validate.** `ssa_module.validate()` must return `Ok(())`. Panic
   message includes the validation error.
3. **Structural roundtrip.** `roundtrip_check::structural_diff(tree_back,
   reparsed)` must be `None`. This matches the property the existing
   `realworld_stability.rs` test enforces.
4. **Idempotence.** Run the full roundtrip a second time on `tree_back`; the
   second `tree_back2` must structurally equal the first `tree_back`. Catches
   non-deterministic emit and per-pass drift.

Order matters: validate before structural before idempotence. A failure in an
earlier oracle short-circuits later ones to keep crash messages focused on
root cause.

### Seeds and corpus management

Initial seed corpus: `corpus/fuzz_seeds/` populated at setup time by copying
all of `corpus/realworld/tier1/` (small, well-formed, fast to mutate).
Tier 2 minified bundles are too large to be useful direct seeds (libFuzzer
mutates them too slowly), but the mutator may pull subtrees from them in v2
splicing operators.

`cargo fuzz run roundtrip -- -max_len=65536` keeps individual inputs bounded.
Larger inputs are slow and contribute little marginal coverage.

### Regression corpus

When the fuzzer finds a crash, the workflow is:

1. `cargo fuzz tmin roundtrip <crash-file>` to minimize the input.
2. Manually inspect — confirm it's a real bug, not a known issue.
3. Move the minimized file to `corpus/regressions/<descriptive-name>.js`.
4. Add a one-line entry in `realworld_stability.rs` per_file_tests so the
   regression runs as a normal `cargo test`.
5. Fix the bug.

This means every crash discovered by the fuzzer becomes a permanent unit test,
even after the underlying fuzz seed pool changes.

### CI integration

Two GitHub Actions jobs:

- **`fuzz-pr`** — runs on every PR. `cargo fuzz run roundtrip -- -max_total_time=300`
  (5 minutes). Fails the PR if any crash is found. Uses cached corpus from the
  default branch as starting seeds.
- **`fuzz-nightly`** — runs on a schedule. `-max_total_time=3600` (1 hour).
  Posts crashes as GitHub issues with the minimized repro attached.

If `fuzz-pr` proves flaky in practice (e.g. legitimate non-determinism in the
fuzzer itself, or 5min isn't enough to converge), drop it to nightly-only.
The regression-corpus mechanism above is what guards merged-bug-doesn't-regress;
the PR gate is for "did this PR introduce a new bug."

## Data flow

```
libFuzzer byte stream
       │
       ▼
mutator::generate
   │ (seed_idx, RNG)
   ▼
load corpus/fuzz_seeds/[seed_idx]
   │
   ▼
parse → AST
   │
   ▼
apply mutation_sequence (Tier 1 + Tier 2 ops)
   │
   ▼
emit AST → JS string ──────────┐
                               │
                               ▼
                       oracles::check_all
                               │
            ┌──────────────────┼──────────────────┐
            ▼                  ▼                  ▼
     parse_and_lower      lower_ssa          lower_tree
            │             validate ✓              │
            ▼                  ▼                  ▼
       tree_module         ssa_module         tree_back
                                                  │
                          ┌───────────────────────┤
                          ▼                       ▼
                       emit                     emit (again)
                          │                       │
                          ▼                       ▼
                      reparsed              second roundtrip
                          │                       │
                          ▼                       ▼
                  structural_diff ✓        idempotence ✓
                  (tree_back vs           (tree_back vs
                   reparsed)               tree_back2)
```

## Error handling

- Mutator failures (e.g. tried to swap with a non-existent sibling) are not
  bugs. Return `None` from `generate`; libFuzzer skips the input.
- Pipeline panics are bugs. libFuzzer captures them as crashes automatically.
- `validate()` errors, `structural_diff` returning `Some`, and idempotence
  mismatches are converted to panics with descriptive messages so they look
  the same to libFuzzer's crash bucketing.

The original parse failing on a mutated input is *not* a bug — mutations can
produce syntactically invalid JS (e.g. `WrapInBlock` placing a `break` outside
a loop). The mutator should aim to produce parseable output but isn't required
to. Parse failures inside the oracle just return early without panicking.

## Testing

The fuzzer itself is testing infrastructure, but the mutator needs unit tests:

- One test per mutation operator: feed a known input, assert the output AST
  has the expected change.
- A "every-mutation-roundtrips" test: for each operator, applied to a small
  fixed input, assert the result still parses (bounds the mutator's
  invalid-output rate).

Place these in `crates/fuzz/src/mutator.rs` under `#[cfg(test)] mod tests`.

## Open questions

- **Nightly toolchain in CI.** `cargo-fuzz` requires nightly. Confirm the
  CI image already has nightly available, or pin a specific nightly date in
  `rust-toolchain.toml` for the fuzz crate.
- **Crash deduplication.** libFuzzer's default bucketing is by stack trace,
  which can produce duplicates when the same bug surfaces from different
  mutation paths. If duplicate noise becomes a problem, add a small
  post-processing step that hashes by panic message rather than stack.

These are tunable post-launch and don't block initial implementation.

## Success criteria

- `cargo +nightly fuzz run roundtrip` runs cleanly and reaches at least
  100k execs/sec on a developer laptop.
- Whatever bugs the fuzzer surfaces in its first hour of running are
  minimized via `cargo fuzz tmin` and checked into `corpus/regressions/` as
  `realworld_stability.rs` test cases (target: at least one regression
  landed before declaring v1 done).
- CI `fuzz-pr` job runs in <6 minutes end to end.
- `crates/fuzz/README.md` documents how to run the fuzzer locally and how to
  triage a crash, including the `tmin` → regression-corpus workflow.
