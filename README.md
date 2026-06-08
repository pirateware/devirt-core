# devirt-core

A generic, sample-agnostic JavaScript deobfuscator built as a compiler. It parses
input with [oxc](https://github.com/oxc-project/oxc), runs a fixpoint of
semantics-preserving AST rewrites, and prints readable JavaScript. There are no
per-sample rules: every pass is a general transform, and the output is verified to
preserve observable behavior.

## How it works

The pipeline runs each pass in order and repeats until a full round changes
nothing (fixpoint). Passes live in `src/passes/` and are registered in
`default_pipeline()` (`src/passes/mod.rs`). They fall into a few groups:

- Syntactic normalizers (member access, sequence splitting) that expose structure.
- Dataflow core (constant folding, inlining, dead-store and dead-code
  elimination) that compounds across rounds.
- Control-flow recovery (switch-dispatch unflattening, CFG reconstruction).
- Decoder handling: a [Boa](https://github.com/boa-dev/boa) sandbox evaluates
  string-decoder functions so their results can be lifted into the source.
- Renaming, which assigns meaningful names by inferred role.

Control-flow recovery uses an SSA-based IR (`src/ir/`) with dominator analysis and
a relooper to rebuild structured code from flattened dispatchers.

## Build

```sh
cargo build --release
```

## CLI

```sh
cargo run --release --bin deob -- <file.js>          # deobfuscate, print to stdout
cargo run --release --bin deob -- --format <file.js> # reprint with no transforms ("before" side of a diff)
cargo run --release --bin report                     # readability metrics over ./samples (pull it first)
```

`--format` shares the deobfuscator's exact formatter, so a diff against normal
output shows only real deobfuscation changes, not formatting noise.

## Testing

```sh
cargo nextest run --release   # full suite (CI runs this)
cargo test --release          # same, without nextest
```

The suite (see `tests/README.md`) is self-contained: every fixture is an inline
JS snippet embedded in the Rust tests, so there is no on-disk corpus to pull and
the suite covers correctness only. The readability corpus below is a separate,
optional dashboard — not part of the test gate.

## Corpus

The obfuscated-JS corpus lives **outside this repo**, in a Hugging Face dataset
([`devirt-dev/devirt-corpus`](https://huggingface.co/datasets/devirt-dev/devirt-corpus)).
HF datasets are plain git repos, so no extra tooling is needed. `./samples` is
gitignored.

```sh
scripts/pull-corpus.sh                  # clone/update ./samples from the dataset
CORPUS_REV=<sha> scripts/pull-corpus.sh # pin a revision for reproducible scores
cargo run --release --bin report        # per-source readability scoreboard
cargo run --release --bin report --files  # …plus every sample listed
```

The report groups samples by source (`generated`, `real/npm`, `real/tranco`,
`real/httparchive`) and shows `kept%` plus **opaque% in→out** — the fraction of
machine-looking identifiers before vs. after. Note `kept%` exceeds 100% on real
*minified* input because the engine reformats/expands it; the readability story
is the opaque-% drop, not byte count.

### Growing the corpus

```sh
node scripts/make-seeds.mjs --count 200   # generate varied plain seeds
node scripts/gen-corpus.mjs               # obfuscate them under ~21 profiles
node scripts/gen-corpus.mjs --profiles strong,controlflow   # …or a subset
```

Real-world samples come from three sources (npm / Tranco crawl / HTTP Archive),
each filtered + deduped the same way — see [`scripts/real/README.md`](scripts/real/README.md).

Generating needs `javascript-obfuscator` locally (`npm i javascript-obfuscator`);
it is local tooling only, never part of the crate. After generating/fetching,
push `./samples` back to the dataset to publish.

## Library

The crate exposes the deobfuscator as a normal Rust API for embedding in other
tools:

```rust
use devirt_core::{deobfuscate, format_only};

let report = deobfuscate(source, "input.js"); // filename only steers source-type inference
println!("{}", report.code);
```

`deobfuscate` returns a `Report` with the transformed `code`, parse errors,
fixpoint stats, and an `error` field that is set (instead of panicking) if the
pipeline fails. `format_only` reprints with no transforms for the "before" side of
a diff.

## Robustness

A deobfuscator runs deep machinery (oxc traversal, the SSA IR, the Boa sandbox)
over adversarial input, so two host-crash vectors are contained:

- The pipeline runs on a dedicated worker thread with a large explicit stack
  (`src/util.rs`). Boa's recursive parser and VM lean hard on the native stack,
  and a stack overflow aborts rather than unwinds, so the depth ceiling is made
  large and independent of the caller.
- Panics are caught: an internal panic (an oxc bug, or Boa's `i32::MIN % -1`
  overflow) collapses to the worker thread's `join()` returning `Err`, and
  `deobfuscate` falls back to returning the input cleanly reformatted with
  `Report::error` set. Callers never see a crash.
