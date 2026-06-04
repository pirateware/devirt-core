#!/usr/bin/env bash
#
# Re-bless the regression baselines after an *intentional* output change.
#
#   ./scripts/bless.sh
#
# Regenerates, from the current engine:
#   tests/snapshots/<sample>.out.js   byte-exact golden output per sample
#   tests/snapshots/SCOREBOARD.md     readability metrics table
#
# Then review the resulting `tests/snapshots/` diff like any other code change —
# a re-bless is an explicit, reviewable record of how the output moved.

set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

echo ">> regenerating golden snapshots + scoreboard" >&2
UPDATE_SNAPSHOTS=1 cargo test --manifest-path "$ROOT/Cargo.toml" --release --test golden

echo >&2
echo ">> done. Review the diff:" >&2
git -C "$ROOT" --no-pager diff --stat -- tests/snapshots/ || true
