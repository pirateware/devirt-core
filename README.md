# js-deobfuscator

A JavaScript deobfuscator built as a compiler.

## Testing & regressions

```sh
cargo test --release                 # full suite (units + equivalence + goldens)
cargo run --release --bin report     # live readability dashboard
./scripts/run-samples.sh             # write in/out pairs to out/ for diffing
./scripts/run-samples.sh sample_5    # side-by-side diff for one sample
./scripts/bless.sh                   # re-bless golden snapshots after an intended change
```

The regression net (see [`tests/README.md`](tests/README.md)) pins every
sample's output **byte-for-byte** (`tests/snapshots/`) and proves the whole
pipeline **preserves observable behavior** on the real corpus
(`tests/sample_equivalence.rs`), so no pass change can silently move output.
[`tests/snapshots/SCOREBOARD.md`](tests/snapshots/SCOREBOARD.md) tracks the
readability metrics over time.

## Samples

Real-world obfuscated scripts (`samples/sample_*.js`) plus a generated synthetic
corpus (`samples/generated/`, from `samples/seeds/` via `scripts/gen-samples.js`).

```
https://js.datadome.co/tags.js
https://www.gstatic.com/recaptcha/releases/ne1iDVwClkE7nKD3uA9Vqsvl/recaptcha__en.js   (sample_14)
https://hcaptcha.com/1/api.js   (sample_15)
https://www.google.com/recaptcha/api.js
https://www.gstatic.com/recaptcha/releases/U5VsmTDhJM1iOJUyw4DEUTYv/recaptcha__en.js
https://grok.com/cdn-cgi/challenge-platform/scripts/jsd/main.js
https://chatgpt.com/cdn-cgi/challenge-platform/h/b/scripts/jsd/883a1f27d85f/main.js
https://challenges.cloudflare.com/turnstile/v0/b/883a1f27d85f/api.js
https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/b/turnstile/f/ov2/av0/rch/lw9l4/0x4AAAAAADFyW2tHYv1jx1Zn/auto/fbE/new/normal?lang=auto
https://www.godaddy.com/6E0qyK/xa/TJ/W2yd/bxZ7lgJLhd/E7aGrpQthYNN0J/TiMYOnwD/dS/0yfDdaJW8B
https://www.godaddy.com/6E0qyK/xa/TJ/W2yd/bxZ7lgJLhd/Jb/IwhwOXwD/K1/cNegQ9Sgsq
https://client.px-cdn.net/PXK3bezZfO/main.min.js
https://sso.godaddy.com/149e9513-01fa-4fb0-aad4-566afd725d1b/2d206a39-8ed7-437e-a3be-862e0f06eea3/p.js?v=20588
```

## WebAssembly / Cloudflare Workers

The sandbox is [Boa](https://github.com/boa-dev/boa) (pure Rust), so the whole
deobfuscator compiles to one `wasm32-unknown-unknown` module — no C toolchain,
no WASI. The crate builds a `cdylib` and exposes a JS entry point:

```rust
// src/lib.rs — only compiled for wasm
pub fn deobfuscate_js(source: &str, filename: &str) -> String
```

### Build

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli        # one-time; generates the JS glue
cargo build --lib --release --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/js_deobfuscator.wasm \
  --out-dir pkg --target bundler
wasm-opt -Oz pkg/js_deobfuscator_bg.wasm -o pkg/js_deobfuscator_bg.wasm  # optional, shrinks ~20-40%
```

Size (release, before `wasm-opt`): ~10.4 MB raw, **~2.4 MB gzip / ~1.5 MB
brotli** — within the Cloudflare Workers compressed-bundle cap on every plan
(3 MiB Free, 10 MiB Paid).

### Cloudflare Worker

A complete, deployable Worker lives in [`worker/`](worker/) (serves
`worker.devirt.dev`). It builds this crate to WASM via `wasm-pack` and exposes a
POST endpoint:

```sh
cd worker && npm install && npm run deploy
curl -X POST --data-binary @obfuscated.js https://worker.devirt.dev
```

See [`worker/README.md`](worker/README.md) for prerequisites and details.

### Constraints to know

- **CPU time.** Deobfuscating large samples takes seconds; the Free plan's 10 ms
  budget won't fit. Use a paid/Standard plan with `cpu_ms` raised. The 10 M
  loop-iteration sandbox budget bounds the riskiest part.
- **Memory.** Workers cap at 128 MiB; the 2 MB monster samples (oxc arena + Boa
  heap) may not fit. The wasm stack is fixed at link time (16 MiB, in
  `.cargo/config.toml`) — raise it if deeply nested input traps, lower it to
  reclaim heap.
- **No threads, `panic = abort`.** On wasm the pipeline runs inline (native uses a
  big-stack worker thread) and panics cannot be caught. Boa 0.21 has one
  process-fatal bug — `i32::MIN % -1` panics (see `src/sandbox.rs`). The one
  deterministic trigger (the math-fingerprint probe battery) is already avoided,
  so the current corpus does not hit it; a decoder whose *own* arithmetic computes
  it would trap that one request. To fully harden, pin a patched Boa:
  `[patch.crates-io] boa_engine = { git = "<your fork with wrapping_rem>" }`.
