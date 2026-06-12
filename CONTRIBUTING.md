# Contributing

Thanks for your interest in devirt-core. This document covers how to build, test,
and add transforms to the deobfuscator.

## Ground rule: every transform is sound and sample-agnostic

devirt-core is a compiler, not a collection of per-sample unpackers. Two
invariants hold for all contributions:

- **No per-sample rules.** Every pass must be a *general* transform that keys off
  structure/dataflow, never off a fingerprint of one obfuscator's output.
- **Semantics are preserved.** A pass may only rewrite code into observably
  equivalent code. Soundness beats aggressiveness: if a rewrite is correct only
  *usually*, it doesn't go in.

Correctness is enforced by a differential equivalence harness, not by reviewer
trust — see [Testing](#testing).

## Project layout

A Cargo workspace with three crates under `crates/`:

- **`devirt-core`** (`crates/core`) — the library: passes, the SSA IR, the sandbox.
- **`devirt-cli`** (`crates/cli`) — the `deob` CLI.
- **`devirt-report`** (`crates/report`) — the `report` corpus/equivalence tooling.

## Building

```sh
cargo build --release                  # whole workspace
cargo build --release -p devirt-core   # a single crate
```

## Testing

```sh
cargo nextest run --release            # full suite (what CI runs)
cargo test --release                   # same, without nextest
```

All fixtures are inline JS snippets embedded in the Rust tests — there is no
on-disk corpus to pull, and the suite covers correctness only. The gate has three
layers:

- per-pass unit tests in `crates/core/tests/<pass>.rs`;
- the differential snippet-equivalence harness (`tests/equivalence.rs`) and the
  whole-program / adversarial equivalence harnesses, which run a fixture before
  and after deobfuscation and require identical behavior signatures;
- the robustness / panic-containment battery.

The optional readability corpus (the `report` tool, the Hugging Face dataset) is a
dashboard, **not** part of the test gate. See the README's *Corpus* section.

## Linting

CI treats warnings as errors. Before opening a PR:

```sh
cargo fmt --all
cargo clippy --release --all-targets -- -D warnings
```

## Adding a pass

1. Create `crates/core/src/passes/<your_pass>.rs` and implement the `Pass` trait
   (`crates/core/src/pass.rs`): `name()` and
   `run(&mut self, program, allocator) -> bool` (return `true` if it changed the
   tree — the driver uses this to detect the fixpoint).
2. Register it in `default_pipeline()` in `crates/core/src/passes/mod.rs`. Order
   matters: normalizers expose structure, the dataflow core compounds, control-flow
   recovery runs after. Place it where its inputs are ready.
3. Add `crates/core/tests/<your_pass>.rs` with inline before/after snippets, and
   make sure the change is covered by an equivalence fixture so soundness is
   machine-checked.

Keep new passes idempotent at the fixpoint (a second run over already-clean code
should report no change), or the pipeline won't converge.

## Pull requests

- Keep PRs focused; one logical change per PR.
- The full suite + clippy must be green.
- Explain *why* a rewrite is always sound, not just that the tests pass.

## License

By contributing, you agree that your contributions are dual licensed under
[MIT](LICENSE-MIT) and [Apache-2.0](LICENSE-APACHE), matching the project license.
No separate CLA is required.
