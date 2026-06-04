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
