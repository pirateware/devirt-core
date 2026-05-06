#!/usr/bin/env bash
# Generate the tier4 corpus: three structurally distinct obfuscated
# samples produced from a single source program (corpus/sources/baseline.js).
#
# Output: corpus/realworld/tier4/{baseline.obfuscatorio_high,
#                                  baseline.jsconfuser_high,
#                                  baseline.jsconfuser_vm}.js
#         + tier4/GENERATION.md (versions, settings, seeds)
#
# Reproducibility:
#   - obfuscator.io is seed-pinned (seed: 1) → byte-identical re-runs.
#   - js-confuser 2.0.1 does not support seeds → snapshots; re-running
#     will produce diff churn for the two jsconfuser_* samples. Same
#     trade-off the existing tier3 makes for randomized output.
#
# Implementation note: both tools are invoked via their programmatic
# Node APIs (corpus/scripts/_obfuscate_run.js), not their CLIs. This is
# because javascript-obfuscator's CLI rejects input paths that contain
# hidden directory components — and worktrees commonly live under
# .claude/, which would break CLI runs there.
#
# This script does not run on CI — it's a one-shot reproducible
# generator. CI runs the corpus harness against whatever's checked in.
#
# Usage:
#   corpus/scripts/fetch_obfuscated.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SRC="$REPO_ROOT/corpus/sources/baseline.js"
OUT_DIR="$REPO_ROOT/corpus/realworld/tier4"
SCRIPTS_DIR="$REPO_ROOT/corpus/scripts"
CACHE_DIR="$SCRIPTS_DIR/.node_cache_tier4"

OBFUSCATOR_VERSION="5.4.2"
JSCONFUSER_VERSION="2.0.1"

mkdir -p "$OUT_DIR"

command -v node >/dev/null || { echo "node missing"; exit 1; }
command -v npm  >/dev/null || { echo "npm missing"; exit 1; }
[[ -f "$SRC" ]] || { echo "source missing: $SRC"; exit 1; }

# Local install for both obfuscator libraries. Cached so re-runs are fast.
if [[ ! -d "$CACHE_DIR/node_modules/js-confuser" \
   || ! -d "$CACHE_DIR/node_modules/javascript-obfuscator" ]]; then
    echo "== installing obfuscator deps (one-time, into $CACHE_DIR) =="
    mkdir -p "$CACHE_DIR"
    if [[ ! -f "$CACHE_DIR/package.json" ]]; then
        cat >"$CACHE_DIR/package.json" <<'PKG'
{ "name": "tier4-cache", "version": "0.0.0", "private": true }
PKG
    fi
    ( cd "$CACHE_DIR" && \
      npm install --silent --no-audit --no-fund \
          "javascript-obfuscator@$OBFUSCATOR_VERSION" \
          "js-confuser@$JSCONFUSER_VERSION" >/dev/null )
fi

run_node() {
    NODE_PATH="$CACHE_DIR/node_modules" \
        node "$SCRIPTS_DIR/_obfuscate_run.js" "$SRC" "$OUT_DIR" "$1"
}

echo "== Sample A: obfuscator.io high (seed=1) =="
run_node obfuscatorio

echo "== Sample B: js-confuser high =="
run_node jsconfuser_high

echo "== Sample C: js-confuser high + rgf (VM) =="
run_node jsconfuser_vm

# ----- Generation log -----
TODAY="$(date -u +%Y-%m-%d)"
cat >"$OUT_DIR/GENERATION.md" <<EOF
# tier4 generation log

Generated: $TODAY
Source: \`corpus/sources/baseline.js\`
Generator: \`corpus/scripts/fetch_obfuscated.sh\`

Tier4 holds **whole-program** obfuscated samples produced from a single
source program through three structurally distinct obfuscation strategies.
Tiers 1-3 hold per-construct samples derived from real library functions;
tier4 stresses cross-construct interactions (control flow + closures +
state) within one program against three different obfuscator philosophies.

## Sample A — \`baseline.obfuscatorio_high.js\`

- Tool: \`javascript-obfuscator@$OBFUSCATOR_VERSION\`
- Settings: \`{ target: "browser", optionsPreset: "high-obfuscation", seed: 1 }\`
- Reproducibility: byte-identical across re-runs (seeded).

## Sample B — \`baseline.jsconfuser_high.js\`

- Tool: \`js-confuser@$JSCONFUSER_VERSION\`
- Settings: built-in \`presets.high\` with \`target: "node"\` (no overrides).
- Reproducibility: snapshot. js-confuser 2.0.1 does not expose a seed
  parameter, so re-running \`fetch_obfuscated.sh\` will produce a
  different file. Commit the diff churn.

## Sample C — \`baseline.jsconfuser_vm.js\`

- Tool: \`js-confuser@$JSCONFUSER_VERSION\`
- Settings: built-in \`presets.high\` with \`target: "node"\` + \`rgf: true\`.
- Reproducibility: snapshot (same reason as Sample B).
- Option-name note: the ticket calls this "VM mode" and warns that the
  option name has changed across JsConfuser versions. In 2.0.1 the
  relevant option is \`rgf\` (Reduced Global Function) — it rewrites
  top-level functions into eval'd strings, which is JsConfuser's
  current VM-style protection. The older \`lock: { ... }\` option in
  the ticket text refers to environment-integrity locks (date /
  domain pinning), a different feature; \`rgf\` is the structural
  transformation we want for this sample.

## Reproducing

\`\`\`sh
corpus/scripts/fetch_obfuscated.sh
\`\`\`

Both tools are invoked via their programmatic Node APIs from
\`corpus/scripts/_obfuscate_run.js\` because javascript-obfuscator's CLI
rejects paths that contain hidden directory components.
EOF

echo
echo "Done. tier4 contents:"
ls -la "$OUT_DIR"
