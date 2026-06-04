# js-deobfuscator — Cloudflare Worker

Serves the deobfuscator on **https://worker.devirt.dev**. The Rust crate one
directory up is compiled to WebAssembly and called from a thin JS handler
(`index.js`); the `deobfuscate_js` binding lives in `../src/lib.rs`.

## Prerequisites

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-pack          # builds the wasm + runs wasm-bindgen & wasm-opt
npm install                      # installs wrangler (from package.json)
npx wrangler login               # auth; the devirt.dev zone must be on this account
```

## Develop / deploy

```sh
npm run build      # wasm-pack -> worker/pkg/  (also run automatically on deploy)
npm run dev        # local: http://localhost:8787
npm run deploy     # builds + publishes to worker.devirt.dev
```

`wrangler deploy` runs the `[build]` step in `wrangler.toml`, so a clean deploy is
a single command once the prerequisites are in place. The `custom_domain = true`
route provisions the DNS record and TLS cert for `worker.devirt.dev` on first
deploy.

## Use

```sh
curl -X POST --data-binary @obfuscated.js https://worker.devirt.dev
curl -X POST --data-binary @app.ts 'https://worker.devirt.dev?filename=app.ts'
```

`GET /` returns usage text. POST the JS source as the request body; the response
body is the deobfuscated code.

## Limits & caveats

- **CPU time** — big samples take seconds; `wrangler.toml` sets `cpu_ms = 30000`,
  which needs the Workers **Standard (paid)** usage model. The Free plan's 10 ms
  budget will time out on large inputs.
- **Memory** — Workers cap at 128 MiB. Very large samples (≈2 MB obfuscated) may
  not fit. The wasm stack is fixed at 16 MiB in `../.cargo/config.toml`.
- **Boa `i32::MIN % -1` panic** — under wasm's `panic = abort` this traps the
  instance; the handler catches it and returns HTTP 500 for that request. The
  current corpus doesn't trigger it; to remove the risk entirely, pin a patched
  Boa (`wrapping_rem`) via `[patch.crates-io]` in `../Cargo.toml`.
