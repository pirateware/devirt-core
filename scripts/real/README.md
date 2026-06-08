# Real-world corpus fetchers

Three independent sources of in-the-wild obfuscated/minified JavaScript. All
write into `samples/real/<source>/` and funnel candidates through one shared
filter (`obfcheck.mjs`): a structural obfuscation heuristic + content-hash dedupe
+ size band (600 B – 512 KB). Run any subset, then score with `report` to compare
what each source yields.

```sh
scripts/pull-corpus.sh            # get ./samples first (writes land in it)

node scripts/real/fetch-npm.mjs                 # jsDelivr min.js of popular packages
node scripts/real/fetch-tranco.mjs              # crawl top sites' <script src>
node scripts/real/load-httparchive.mjs --in results.jsonl   # BigQuery export

cargo run --release --bin report  # per-source scoreboard (generated vs real/*)
```

## Sources

| Script | Yields | Needs | Notes |
|--------|--------|-------|-------|
| `fetch-npm.mjs` | minified bundles | network | Easiest, deterministic. Mostly *minified*, not heavily obfuscated. `--list pkgs.txt`, `--limit N`. |
| `fetch-tranco.mjs` | real site scripts | network | Flakiest (bot-blocking, lazy loading). Pass the full list with `--list top-1m.csv` from <https://tranco-list.eu/>. `--limit N`, `--per-site K`. |
| `load-httparchive.mjs` | crawled response bodies | GCP/BigQuery | Best quality/volume. Run `httparchive.sql` in BigQuery, export JSONL, ingest here. |

## HTTP Archive path

1. Open the BigQuery console under your own (free-tier) GCP project.
2. Run `scripts/real/httparchive.sql` — adjust the `date` to an existing crawl
   and keep the `LIMIT` (the response-bodies table is huge; the date/client
   filter bounds the scan cost).
3. Export results as JSONL (rows with `url` + `body`).
4. `node scripts/real/load-httparchive.mjs --in results.jsonl`.

## Filter

`obfcheck.mjs` `analyze()` scores each script on obfuscator fingerprints —
`_0x…` identifiers, the rotate-by-shift string-array decoder, `["x"]` bracket
access density, hex-literal density — and labels it `obfuscated` / `minified` /
`plain`. Only non-plain scripts in the size band are kept. Reruns skip anything
already on disk (dedupe by content hash), so fetchers are resumable.

## Hitting ~5k

Roughly: `make-seeds.mjs --count 200` × ~21 profiles ≈ 4k generated, plus a few
hundred to ~1k real across the three sources. Tune seed count / `--profiles` and
the real `--limit`s to taste, then `report` to see the per-source split.
