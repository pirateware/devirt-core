# tier4 generation log

Generated: 2026-05-06
Source: `corpus/sources/baseline.js`
Generator: `corpus/scripts/fetch_obfuscated.sh`

Tier4 holds **whole-program** obfuscated samples produced from a single
source program through three structurally distinct obfuscation strategies.
Tiers 1-3 hold per-construct samples derived from real library functions;
tier4 stresses cross-construct interactions (control flow + closures +
state) within one program against three different obfuscator philosophies.

## Sample A — `baseline.obfuscatorio_high.js`

- Tool: `javascript-obfuscator@5.4.2`
- Settings: `{ target: "browser", optionsPreset: "high-obfuscation", seed: 1 }`
- Reproducibility: byte-identical across re-runs (seeded).

## Sample B — `baseline.jsconfuser_high.js`

- Tool: `js-confuser@2.0.1`
- Settings: built-in `presets.high` with `target: "node"` (no overrides).
- Reproducibility: snapshot. js-confuser 2.0.1 does not expose a seed
  parameter, so re-running `fetch_obfuscated.sh` will produce a
  different file. Commit the diff churn.

## Sample C — `baseline.jsconfuser_vm.js`

- Tool: `js-confuser@2.0.1`
- Settings: built-in `presets.high` with `target: "node"` + `rgf: true`.
- Reproducibility: snapshot (same reason as Sample B).
- Option-name note: the ticket calls this "VM mode" and warns that the
  option name has changed across JsConfuser versions. In 2.0.1 the
  relevant option is `rgf` (Reduced Global Function) — it rewrites
  top-level functions into eval'd strings, which is JsConfuser's
  current VM-style protection. The older `lock: { ... }` option in
  the ticket text refers to environment-integrity locks (date /
  domain pinning), a different feature; `rgf` is the structural
  transformation we want for this sample.

## Reproducing

```sh
corpus/scripts/fetch_obfuscated.sh
```

Both tools are invoked via their programmatic Node APIs from
`corpus/scripts/_obfuscate_run.js` because javascript-obfuscator's CLI
rejects paths that contain hidden directory components.
