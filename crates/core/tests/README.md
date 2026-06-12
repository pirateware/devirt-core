# Test & regression suite

This suite tracks **functionality**: each layer catches a different class of
regression, from "this one pass mis-folds a literal" up to "a value-changing pass
no longer preserves observable behavior". Every fixture is inline (hand-written
JS snippets embedded in the Rust tests) — there is no on-disk corpus, so the
whole suite is fast and self-contained.

## Running it

```sh
cargo nextest run --release   # full suite (CI runs this, ~2s)
cargo test --release          # same, without nextest
```

`nextest` runs every test across all binaries in one thread pool, so they overlap
instead of running binary-by-binary; on a multi-core box that roughly halves the
wall time. Config: `.config/nextest.toml`. CI (`.github/workflows/ci.yml`) runs
`cargo nextest run --release --profile ci`, so the suite gates every change.

## Layers

| Layer | File(s) | What it pins |
|-------|---------|--------------|
| **Pass units** | `dce.rs`, `rename.rs`, `unflatten*.rs`, `decoder_lift.rs`, `proxy_inline.rs`, … | Each pass does the right thing in isolation, free of pipeline interactions. |
| **Snippet equivalence** | `equivalence.rs` | Hand-written obfuscated snippets evaluate to the *same value* before and after (Boa). The soundness net for value-changing passes. |
| **Pipeline** | `phase1.rs` | Multi-pass interactions on small inputs reach the expected fixpoint. |
| **Robustness / containment** | `robustness.rs`, `lib.rs` containment tests | Adversarial input degrades gracefully; pipeline panics and stack overflow never crash the host. |

> The real-world sample corpus, the generated obfuscator.io corpus, the golden
> byte-snapshots, and the readability scoreboard have been moved out of this repo.
> This suite covers correctness only.
