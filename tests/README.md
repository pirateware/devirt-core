# Test & regression suite

The suite is layered: each layer catches a different class of regression, from
"this one pass mis-folds a literal" up to "the whole pipeline changed what a real
obfuscated sample *does*". Everything runs under `cargo test --release` and gates
CI.

## Layers

| Layer | File(s) | What it pins |
|-------|---------|--------------|
| **Pass units** | `const_fold`-style files, `rename.rs`, `unflatten*.rs`, `decoder_lift.rs`, … | Each pass does the right thing in isolation, free of pipeline interactions. |
| **Snippet equivalence** | `equivalence.rs` | Hand-written obfuscated snippets evaluate to the *same value* before and after (QuickJS). The soundness net for value-changing passes. |
| **Sample invariants** | `samples.rs` | Every sample's output re-parses and is idempotent (a second run is a no-op). |
| **Determinism** | `determinism.rs` | Byte-identical output across runs (frozen sandbox clock + stable emission order). |
| **Golden snapshots** | `golden.rs` + `snapshots/` | Byte-exact output per sample, plus a readability `SCOREBOARD.md`. Catches *any* drift across the corpus. |
| **Sample behavior-equivalence** | `sample_equivalence.rs` | The *whole pipeline* preserves *observable behavior* (console output + terminal outcome) on the *real* corpus, run under a deterministic browser-host shim. |
| **Robustness / containment** | `robustness.rs`, `lib.rs` containment tests | Adversarial input degrades gracefully; pipeline panics and stack overflow never crash the host. |

## Golden snapshots — the diff baseline

`tests/snapshots/<sample>.out.js` is the committed, byte-exact output of each
sample. Because the pipeline is deterministic, any pass change that moves any
sample's output turns `golden.rs` red and the PR diff shows exactly what moved —
no silent drift.

`tests/snapshots/SCOREBOARD.md` is a committed, human-readable metrics table
(bytes / bracket-access / opaque-name ratio / rounds / convergence) so a reviewer
sees the *shape* of a change at a glance.

### Blessing an intentional change

When you intend to change output (a new/improved pass), regenerate the baseline:

```sh
./scripts/bless.sh
# equivalently:
UPDATE_SNAPSHOTS=1 cargo test --release --test golden
```

Then review the `tests/snapshots/` diff like any other change. A re-bless is an
explicit, reviewable record of how the output moved — and the scoreboard diff
tells you whether it moved the *right* way (fewer bytes, fewer brackets, lower
opaque%).

## Behavior-equivalence — the soundness net for the real corpus

`sample_equivalence.rs` runs each sample and its deobfuscated form under the same
**permissive, deterministic browser-host shim** (`sandbox::behavior_signature`):
every browser global (`window`, `document`, `navigator`, …) is a deep proxy that
resolves any access, and the clock + RNG are frozen and timers are inert. The
sample need not do anything *meaningful* — only the *same* thing before and after.

The captured **behavior signature** is the terminal outcome (completed / threw
`<ErrorName>` / timed-out) plus the exact console output. Equal signatures ⇒
observably equivalent. A sample that won't terminate under the shim (a polling /
animation loop) is recorded as *skipped with a reason* — never silently dropped —
and the test fails if the comparable set unexpectedly collapses.

Tunables: `DEOB_EQUIV_BUDGET_MS` (per-run wall-clock budget, default 5000).

## Local diff tooling (not a test)

```sh
./scripts/run-samples.sh          # build + deobfuscate all samples -> out/
./scripts/run-samples.sh sample_5 # side-by-side diff for one sample
cargo run --release --bin report  # live readability dashboard
```
