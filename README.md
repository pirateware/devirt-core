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
cargo run --release --bin report                     # readability metrics across the sample corpus
```

`--format` shares the deobfuscator's exact formatter, so a diff against normal
output shows only real deobfuscation changes, not formatting noise.

## Testing

```sh
cargo test                                  # fast unit tests
cargo test --release --features slow-tests  # full net: goldens, equivalence, idempotency, determinism
./scripts/run-samples.sh                    # write in/out pairs to out/ for diffing
./scripts/run-samples.sh sample_5           # side-by-side diff for one sample
./scripts/bless.sh                          # re-bless golden snapshots after an intended change
```

The regression net (see `tests/README.md`) pins every sample's output
byte-for-byte (`tests/snapshots/`) and proves the whole pipeline preserves
observable behavior on the real corpus (`tests/sample_equivalence.rs`), so no pass
change can silently move output. `tests/snapshots/SCOREBOARD.md` tracks readability
metrics over time. Slow tests are gated behind the `slow-tests` feature so local
iteration stays fast; CI runs them.

## Samples

Real-world obfuscated scripts live in `samples/sample_*.js`, alongside a generated
synthetic corpus (`samples/generated/`, built from `samples/seeds/` via
`scripts/gen-samples.js`). The real samples are sourced from public anti-bot and
challenge scripts, including DataDome, reCAPTCHA, hCaptcha, Cloudflare Turnstile,
PerimeterX, and GoDaddy.

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
