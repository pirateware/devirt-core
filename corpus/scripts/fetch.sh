#!/usr/bin/env bash
# Fetch / generate the realworld corpus.
#
# Output layout (relative to repo root):
#   corpus/realworld/tier1/   — small, unminified library modules
#   corpus/realworld/tier2/   — minified production bundles (vendor + locally minified)
#   corpus/realworld/tier3/   — obfuscated samples (generated from tier1 via javascript-obfuscator)
#
# Each *.js gets a *.source.txt sidecar recording origin + license + fetch date.
#
# Requirements:
#   - bash, curl, jq, node, npm/npx
#   - npx will pull terser and javascript-obfuscator on first run
#
# Usage:
#   corpus/scripts/fetch.sh                # full fetch
#   TIER=tier1 corpus/scripts/fetch.sh     # one tier only
#   FORCE=1 corpus/scripts/fetch.sh        # re-fetch even if file exists

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CORPUS="$REPO_ROOT/corpus/realworld"
TODAY="$(date -u +%Y-%m-%d)"
WANT_TIER="${TIER:-all}"
FORCE="${FORCE:-0}"

mkdir -p "$CORPUS/tier1" "$CORPUS/tier2" "$CORPUS/tier3"

want() { [[ "$WANT_TIER" == "all" || "$WANT_TIER" == "$1" ]]; }

skip_if_exists() {
  local path="$1"
  if [[ "$FORCE" != "1" && -f "$path" ]]; then
    return 0
  fi
  return 1
}

write_sidecar() {
  local js_path="$1" src="$2" license="$3" desc="$4"
  cat >"${js_path%.js}.source.txt" <<EOF
Source: $src
License: $license
Fetched: $TODAY
Description: $desc
EOF
}

# ---------------------------------------------------------------------------
# Tier 1 — unminified library modules + full unminified bundles.
#
# Two sub-strategies:
#   (a) Per-module: pull individual lodash modules + a handful of date-fns
#       modules straight off unpkg. Each file is small and self-contained.
#   (b) Full bundle: pull the un-minified full distribution for a few
#       libraries. These are large single files (lodash.js is ~17K LOC) and
#       exercise the construct mix we *don't* see in the per-module split:
#       prototype chains, fall-through control flow, real-world var-scoping
#       quirks, and (for axios/zod) ES2015+ shapes (classes, async,
#       destructuring) the per-module corpus avoids. They are the FB-PT% /
#       TL-PT% reality check — if a library's full source PassThroughs
#       heavily, that's a coverage gap regardless of what the per-module
#       view says.
# ---------------------------------------------------------------------------

LODASH_VERSION="4.17.21"
LODASH_MODULES=(
  cloneDeep debounce throttle merge get set has isEqual isPlainObject
  flatten chunk groupBy keyBy orderBy partition uniqBy zip
  memoize curry partial flow
  camelCase kebabCase snakeCase startCase capitalize escape
  pick omit mapValues mapKeys invert
  sortBy uniq compact difference intersection union without
  range times constant identity noop
)

# date-fns v2 keeps the `<name>/index.js` per-module layout that's easy
# to fetch individually; v3 restructured into a flatter ESM-only tree.
DATEFNS_VERSION="2.30.0"
DATEFNS_MODULES=(
  format/index.js parseISO/index.js addDays/index.js differenceInDays/index.js
  isAfter/index.js isBefore/index.js startOfDay/index.js endOfMonth/index.js
  subDays/index.js addMonths/index.js getDay/index.js formatDistance/index.js
)

# Full un-minified bundles. Format: pkg@version:dist-path:license:description
# Pinned versions (mirrors TIER2_VENDOR's pinning rationale: identical input
# across runs makes the stability report comparable across commits).
#
# **What "un-minified" means in practice:** these are dist bundles, which
# means Babel / esbuild / Rollup compiled the source down to ES5 before
# publishing. Even packages whose source uses classes / async / optional
# chaining ship a UMD with prototype-based pseudo-classes,
# regenerator-runtime state machines, and explicit ternaries instead.
#
# This matches the project's actual input distribution. Malware in the
# wild ships as compiled / minified / obfuscated dist bundles, not as
# source. The de-sugared shape Babel emits IS the shape obfuscators
# consume IS the shape we're built to handle. We deliberately do NOT
# corpus-test against `src/` or ESM-only `module` entrypoints — those
# represent an input class we never see.
#
# - lodash full: ~17K LOC, hand-written ES5, prototype chains, no
#   async/await/classes. Pairs with tier2/lodash.min.js for
#   matched-pair (un-min ↔ min) analysis.
# - axios un-min: transpiled UMD — classes-as-prototypes, async-as-
#   regenerator. Pairs with tier2/axios.min.js. Stresses the pipeline
#   on the de-sugared shape obfuscators / minifiers actually see.
#
# Each entry must point at a path that is *not* already covered by
# TIER2_VENDOR — duplicate URLs waste corpus space and conflate the
# minified-vs-unminified signal.
TIER1_UNMIN_BUNDLES=(
  "lodash@4.17.21:lodash.js:MIT:Lodash full unminified bundle"
  "axios@1.7.2:dist/axios.js:MIT:Axios full unminified UMD bundle"
)

fetch_tier1() {
  echo "== Tier 1: lodash modules =="
  for m in "${LODASH_MODULES[@]}"; do
    local out="$CORPUS/tier1/lodash_${m}.js"
    skip_if_exists "$out" && { echo "  skip $m"; continue; }
    local url="https://unpkg.com/lodash.${m,,}@${LODASH_VERSION}/index.js"
    if curl -fsSL "$url" -o "$out" 2>/dev/null; then
      write_sidecar "$out" "$url" "MIT" "Lodash $m module"
      echo "  ok   lodash.$m"
    else
      # Fallback: not all lodash methods ship as standalone packages.
      url="https://unpkg.com/lodash@${LODASH_VERSION}/${m}.js"
      if curl -fsSL "$url" -o "$out" 2>/dev/null; then
        write_sidecar "$out" "$url" "MIT" "Lodash $m (from main package)"
        echo "  ok   lodash/$m (fallback)"
      else
        rm -f "$out"
        echo "  miss lodash.$m"
      fi
    fi
  done

  echo "== Tier 1: date-fns modules =="
  for m in "${DATEFNS_MODULES[@]}"; do
    local name="${m%/index.js}"
    local out="$CORPUS/tier1/datefns_${name}.js"
    skip_if_exists "$out" && { echo "  skip $name"; continue; }
    local url="https://unpkg.com/date-fns@${DATEFNS_VERSION}/${m}"
    if curl -fsSL "$url" -o "$out" 2>/dev/null; then
      write_sidecar "$out" "$url" "MIT" "date-fns $name module"
      echo "  ok   datefns.$name"
    else
      rm -f "$out"
      echo "  miss datefns.$name"
    fi
  done

  echo "== Tier 1: full un-minified bundles =="
  for spec in "${TIER1_UNMIN_BUNDLES[@]}"; do
    IFS=':' read -r pkgver dist license desc <<<"$spec"
    local pkg="${pkgver%@*}"
    local safe="${pkg//[^a-zA-Z0-9]/_}"
    local out="$CORPUS/tier1/${safe}.full.js"
    skip_if_exists "$out" && { echo "  skip $pkg.full"; continue; }
    local url="https://unpkg.com/${pkgver}/${dist}"
    if curl -fsSL "$url" -o "$out" 2>/dev/null; then
      write_sidecar "$out" "$url" "$license" "$desc"
      echo "  ok   $pkg.full"
    else
      rm -f "$out"
      echo "  miss $pkg.full ($url)"
    fi
  done
}

# ---------------------------------------------------------------------------
# Tier 2 — minified production bundles.
# Two sub-strategies:
#   (a) Vendor: pull the official .min.js artifact for popular packages.
#   (b) Local: re-minify each tier1 file with terser + esbuild + swc, so we
#       get matched-pair (tier1 source ↔ tier2 minified) ground truth.
# ---------------------------------------------------------------------------

# pkg@version:dist-path:license:description
TIER2_VENDOR=(
  "react@18.3.1:umd/react.production.min.js:MIT:React production UMD"
  "react-dom@18.3.1:umd/react-dom.production.min.js:MIT:React-DOM production UMD"
  "vue@3.4.27:dist/vue.global.prod.js:MIT:Vue 3 global production"
  "jquery@3.7.1:dist/jquery.min.js:MIT:jQuery minified"
  "axios@1.7.2:dist/axios.min.js:MIT:Axios minified UMD"
  "lodash@4.17.21:lodash.min.js:MIT:Lodash full minified"
  "moment@2.30.1:min/moment.min.js:MIT:Moment.js minified"
  "rxjs@7.8.1:dist/bundles/rxjs.umd.min.js:Apache-2.0:RxJS UMD minified"
  "three@0.140.0:build/three.min.js:MIT:three.js minified (last version with build/three.min.js)"
  "underscore@1.13.6:underscore-min.js:MIT:Underscore minified"
  "redux@4.2.1:dist/redux.min.js:MIT:Redux 4 minified (v5 dropped UMD)"
  "immer@9.0.21:dist/immer.umd.production.min.js:MIT:Immer 9 minified UMD"
  "preact@10.22.0:dist/preact.umd.js:MIT:Preact UMD"
  "marked@12.0.2:marked.min.js:MIT:Marked markdown parser"
  "uuid@8.3.2:dist/umd/uuidv4.min.js:MIT:uuid v4 minified (v9 dropped UMD)"
  "chart.js@4.4.3:dist/chart.umd.js:MIT:Chart.js UMD"
  "d3@7.9.0:dist/d3.min.js:ISC:D3 minified"
  "ramda@0.30.1:dist/ramda.min.js:MIT:Ramda minified"
  "luxon@3.4.4:build/global/luxon.min.js:MIT:Luxon minified"
  "dayjs@1.11.11:dayjs.min.js:MIT:dayjs minified"
  "core-js-bundle@3.37.1:minified.js:MIT:core-js polyfill bundle (minified)"
  "zod@3.23.8:lib/index.umd.js:MIT:Zod UMD"
  "mobx@6.12.3:dist/mobx.umd.production.min.js:MIT:MobX minified UMD"
  "tslib@2.6.3:tslib.js:0BSD:TypeScript runtime helpers"
)

fetch_tier2_vendor() {
  echo "== Tier 2: vendor minified bundles =="
  for spec in "${TIER2_VENDOR[@]}"; do
    IFS=':' read -r pkgver dist license desc <<<"$spec"
    local pkg="${pkgver%@*}"
    local safe="${pkg//[^a-zA-Z0-9]/_}"
    local out="$CORPUS/tier2/${safe}.min.js"
    skip_if_exists "$out" && { echo "  skip $pkg"; continue; }
    local url="https://unpkg.com/${pkgver}/${dist}"
    if curl -fsSL "$url" -o "$out" 2>/dev/null; then
      write_sidecar "$out" "$url" "$license" "$desc"
      echo "  ok   $pkg"
    else
      rm -f "$out"
      echo "  miss $pkg ($url)"
    fi
  done
}

fetch_tier2_local() {
  echo "== Tier 2: locally minified (terser) =="
  command -v npx >/dev/null || { echo "  npx missing, skipping"; return; }
  for src in "$CORPUS"/tier1/*.js; do
    [[ -f "$src" ]] || continue
    local base
    base="$(basename "$src" .js)"
    local out="$CORPUS/tier2/${base}.terser.min.js"
    skip_if_exists "$out" && { echo "  skip $base"; continue; }
    if npx --yes terser "$src" --compress --mangle -o "$out" 2>/dev/null; then
      write_sidecar "$out" "(generated from tier1/${base}.js via terser)" \
        "MIT (derived)" "Locally minified pair for matched-pair eval"
      echo "  ok   $base.terser"
    else
      rm -f "$out"
      echo "  miss $base.terser"
    fi
  done
}

# ---------------------------------------------------------------------------
# Tier 3 — obfuscated samples generated from tier1 via javascript-obfuscator.
# Three presets give a coverage gradient: low / medium / high.
# Each output keeps a sidecar pointing back at the tier1 original, so tests
# can score "did we recover it" against ground truth.
# ---------------------------------------------------------------------------

OBFUSCATOR_PRESETS=(low medium high)

fetch_tier3() {
  echo "== Tier 3: obfuscated samples (javascript-obfuscator) =="
  command -v npx >/dev/null || { echo "  npx missing, skipping"; return; }

  for src in "$CORPUS"/tier1/*.js; do
    [[ -f "$src" ]] || continue
    local base
    base="$(basename "$src" .js)"
    for preset in "${OBFUSCATOR_PRESETS[@]}"; do
      local out="$CORPUS/tier3/${base}.${preset}.js"
      skip_if_exists "$out" && { echo "  skip $base.$preset"; continue; }

      local args=(--target node --compact true)
      case "$preset" in
        low)
          args+=(--string-array true --string-array-threshold 0.5)
          ;;
        medium)
          args+=(--string-array true --string-array-threshold 0.75
                 --string-array-encoding base64
                 --identifier-names-generator hexadecimal
                 --rename-globals false)
          ;;
        high)
          args+=(--string-array true --string-array-threshold 1
                 --string-array-encoding rc4
                 --string-array-rotate true --string-array-shuffle true
                 --control-flow-flattening true --control-flow-flattening-threshold 1
                 --dead-code-injection true --dead-code-injection-threshold 0.4
                 --identifier-names-generator hexadecimal
                 --transform-object-keys true
                 --unicode-escape-sequence true)
          ;;
      esac

      if npx --yes javascript-obfuscator "$src" --output "$out" "${args[@]}" >/dev/null 2>&1; then
        write_sidecar "$out" "(generated from tier1/${base}.js via javascript-obfuscator preset=$preset)" \
          "MIT (derived)" "Matched-pair obfuscation for ground-truth eval; preset=$preset"
        echo "  ok   $base.$preset"
      else
        rm -f "$out"
        echo "  miss $base.$preset"
      fi
    done
  done
}

# ---------------------------------------------------------------------------
# Driver
# ---------------------------------------------------------------------------

want tier1 && fetch_tier1
want tier2 && { fetch_tier2_vendor; fetch_tier2_local; }
want tier3 && fetch_tier3

echo
echo "Done. Corpus counts:"
for t in tier1 tier2 tier3; do
  printf "  %s: %d files\n" "$t" "$(find "$CORPUS/$t" -name '*.js' | wc -l)"
done
