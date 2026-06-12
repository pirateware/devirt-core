# devirt-core

[![CI](https://github.com/devirt-dev/devirt-core/actions/workflows/ci.yml/badge.svg)](https://github.com/devirt-dev/devirt-core/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Release](https://img.shields.io/github/v/release/devirt-dev/devirt-core)](https://github.com/devirt-dev/devirt-core/releases)

**A generic, sample-agnostic JavaScript deobfuscator built as a compiler.**

It parses input with [oxc](https://github.com/oxc-project/oxc), runs a fixpoint of
semantics-preserving AST rewrites, and prints readable JavaScript. There are **no
per-sample rules** — every pass is a general transform, and the output is verified
to preserve observable behavior.

```sh
deob obfuscated.js > readable.js
```

## Install

Prebuilt `deob` binaries for Linux, macOS, and Windows (x86_64 + arm64) are
attached to every [release](https://github.com/devirt-dev/devirt-core/releases).

```sh
# No Rust toolchain needed — download a prebuilt binary:
cargo binstall --git https://github.com/devirt-dev/devirt-core devirt-cli

# …or build and install from source (needs a Rust toolchain):
cargo install --git https://github.com/devirt-dev/devirt-core devirt-cli
```

Either path puts a `deob` binary on your `PATH`. You can also download an archive
from the releases page and drop `deob` somewhere on your `PATH` manually.

## Usage

```sh
deob <file.js>            # deobfuscate, print readable JS to stdout
deob --format <file.js>   # reprint with NO transforms ("before" side of a diff)
```

Stats and any errors go to **stderr**, so stdout stays clean for piping:

```sh
deob in.js > out.js                                  # just the code
diff <(deob --format in.js) <(deob in.js)            # see only real changes
```

`--format` shares the deobfuscator's exact formatter, so a diff against normal
output shows only real deobfuscation changes, not formatting noise.

## Library

The deobfuscator is also a normal Rust crate you can embed:

```rust
use devirt_core::{deobfuscate, format_only};

let report = deobfuscate(source, "input.js"); // filename only steers source-type inference
println!("{}", report.code);
```

`deobfuscate` returns a `Report` with the transformed `code`, parse errors,
fixpoint stats, and an `error` field that is set (instead of panicking) if the
pipeline fails. `format_only` reprints with no transforms for the "before" side of
a diff.

## How it works

The pipeline runs each pass in order and repeats until a full round changes
nothing (fixpoint). Passes live in `crates/core/src/passes/` and are registered in
`default_pipeline()` (`crates/core/src/passes/mod.rs`). They fall into a few groups:

- **Syntactic normalizers** (member access, sequence splitting) that expose structure.
- **Dataflow core** (constant folding, inlining, dead-store and dead-code
  elimination) that compounds across rounds.
- **Control-flow recovery** (switch-dispatch unflattening, CFG reconstruction).
- **Decoder handling** — a [Boa](https://github.com/boa-dev/boa) sandbox evaluates
  string-decoder functions so their results can be lifted into the source.
- **Renaming**, which assigns meaningful names by inferred role.

Control-flow recovery uses an SSA-based IR (`crates/core/src/ir/`) with dominator
analysis and a relooper to rebuild structured code from flattened dispatchers.

### Robustness

A deobfuscator runs deep machinery (oxc traversal, the SSA IR, the Boa sandbox)
over adversarial input, so two host-crash vectors are contained:

- The pipeline runs on a dedicated worker thread with a large explicit stack
  (`crates/core/src/util.rs`). Boa's recursive parser and VM lean hard on the
  native stack, and a stack overflow aborts rather than unwinds, so the depth
  ceiling is made large and independent of the caller.
- Panics are caught: an internal panic (an oxc bug, or Boa's `i32::MIN % -1`
  overflow) collapses to the worker thread's `join()` returning `Err`, and
  `deobfuscate` falls back to returning the input cleanly reformatted with
  `Report::error` set. Callers never see a crash.

## Repository layout

A Cargo workspace with three crates under `crates/`:

| Crate | Path | Provides |
|---|---|---|
| `devirt-core` | `crates/core` | the library (`devirt_core`): passes, the SSA IR, the sandbox |
| `devirt-cli` | `crates/cli` | the `deob` command-line front-end |
| `devirt-report` | `crates/report` | the `report` corpus-scoring / equivalence tool (maintainer tooling) |

## Development

```sh
cargo build --release          # build the whole workspace
cargo nextest run --release    # full test suite (what CI runs)
cargo test --release           # same, without nextest
cargo clippy --release --all-targets -- -D warnings   # lint gate
```

The suite is self-contained: every fixture is an inline JS snippet embedded in the
Rust tests, so there is no corpus to pull and it covers correctness only. See
[CONTRIBUTING.md](CONTRIBUTING.md) for how to add a pass, and
[SECURITY.md](SECURITY.md) for the threat model and how to report vulnerabilities.

## Corpus

The obfuscated-JS corpus lives **outside this repo**, in a Hugging Face dataset
([`devirt-dev/devirt-corpus`](https://huggingface.co/datasets/devirt-dev/devirt-corpus)).
HF datasets are plain git repos, so no extra tooling is needed; `./samples` is
gitignored. Live readability scores and a browsable input→output table are
published on the [dataset page](https://huggingface.co/datasets/devirt-dev/devirt-corpus).

```sh
scripts/pull-corpus.sh                          # clone/update ./samples from the dataset
CORPUS_REV=<sha> scripts/pull-corpus.sh         # pin a revision for reproducible scores
cargo run --release --bin report                # per-source readability scoreboard
cargo run --release --bin report -- --equiv     # differential behavioral-equivalence gate
cargo run --release --bin report -- --json samples/metrics.jsonl --markdown   # JSONL + card
```

`--equiv` is the soundness counterpart to the readability score: it runs each
sample through the whole-program harness (`sandbox::behavior_signature`) before and
after deobfuscation and requires identical signatures (terminal outcome + console
output sequence), exiting non-zero on any mismatch. Samples that don't run
comparably to begin with (a timeout, or a host the harness can't construct) are
skipped.

The report groups samples by source (`generated`, `real/npm`, `real/tranco`,
`real/httparchive`) and shows `kept%` plus **opaque% in→out** — the fraction of
machine-looking identifiers before vs. after. `kept%` exceeds 100% on real
*minified* input because the engine reformats/expands it; the readability story is
the opaque-% drop, not byte count.

### Growing the corpus

```sh
node scripts/make-seeds.mjs --count 200                     # varied plain seeds
node scripts/gen-corpus.mjs                                 # obfuscate them under ~21 profiles
node scripts/gen-corpus.mjs --profiles strong,controlflow   # …or a subset
```

Real-world samples come from three sources (npm / Tranco crawl / HTTP Archive),
each filtered and deduped the same way — see
[`scripts/real/README.md`](scripts/real/README.md). Generating needs
`javascript-obfuscator` locally (`npm i javascript-obfuscator`); it is local
tooling only, never part of the crate.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
