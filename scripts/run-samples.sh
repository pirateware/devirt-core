#!/usr/bin/env bash
#
# Run the deobfuscator on every sample and produce in/out pairs for diffing.
#
#   ./scripts/run-samples.sh              build + deobfuscate all samples -> out/
#   ./scripts/run-samples.sh <name>       show a side-by-side diff for one sample
#   ./scripts/run-samples.sh --no-build   skip the cargo build
#
# For each sample, out/ gets:
#   <name>.in.js     original, re-printed with the SAME formatter (no transforms)
#   <name>.out.js    deobfuscated output
#   <name>.stats.txt pipeline stats (rounds / per-pass changes)
#
# The ".in.js" side is formatted by the engine itself, so a diff against
# ".out.js" shows only real deobfuscation changes, not formatting noise.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/out"
SAMPLES="$ROOT/samples"
BIN="$ROOT/target/release/deob"

build() { echo ">> cargo build --release" >&2; (cd "$ROOT" && cargo build --release --quiet); }

# ---- side-by-side diff for a single sample --------------------------------
view() {
  local name="${1%.js}"
  local a="$OUT/$name.in.js" b="$OUT/$name.out.js"
  if [[ ! -f "$b" ]]; then echo "No output for '$name'. Run: $0" >&2; exit 1; fi

  if command -v code >/dev/null 2>&1; then
    echo ">> opening VS Code side-by-side diff for $name" >&2
    code --diff "$a" "$b"
  elif command -v delta >/dev/null 2>&1; then
    delta --side-by-side --paging=always "$a" "$b"
  else
    # True side-by-side columns in the terminal. `diff -y` exits 1 when the
    # files differ, which they always do — hence `|| true` under pipefail.
    diff -y --width="${COLUMNS:-220}" "$a" "$b" | ${PAGER:-less} || true
  fi
}

# ---- run all samples ------------------------------------------------------
run_all() {
  mkdir -p "$OUT"
  printf '%-14s %11s %11s %6s   %s\n' sample in_bytes out_bytes kept stats
  printf '%-14s %11s %11s %6s   %s\n' "------" "--------" "---------" "----" "-----"
  for f in "$SAMPLES"/*.js; do
    local name; name="$(basename "$f" .js)"
    "$BIN" --format "$f" > "$OUT/$name.in.js" 2>/dev/null || true
    if ! "$BIN" "$f" > "$OUT/$name.out.js" 2> "$OUT/$name.stats.txt"; then
      printf '%-14s %s\n' "$name" "FAILED (see $OUT/$name.stats.txt)"
      continue
    fi
    local inb outb kept stats
    inb=$(wc -c < "$f"); outb=$(wc -c < "$OUT/$name.out.js")
    kept=$(( inb > 0 ? outb * 100 / inb : 0 ))
    stats=$(tail -1 "$OUT/$name.stats.txt" | sed 's/\[deob\] //')
    printf '%-14s %11s %11s %5s%%   %s\n' "$name" "$inb" "$outb" "$kept" "$stats"
  done
  echo
  echo "Pairs written to: $OUT/   (<name>.in.js  |  <name>.out.js)"
  echo "Side-by-side diff: $0 <name>     e.g.  $0 sample_5"
}

# ---- dispatch -------------------------------------------------------------
case "${1:-}" in
  ""|--no-build)
    [[ "${1:-}" == "--no-build" ]] || build
    run_all
    ;;
  -h|--help)
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
    ;;
  *)
    # Treat the argument as a sample name to diff.
    view "$1"
    ;;
esac
