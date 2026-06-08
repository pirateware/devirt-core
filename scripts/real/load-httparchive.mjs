#!/usr/bin/env node
//
// Ingest a BigQuery export of HTTP Archive response bodies into the corpus.
// Run httparchive.sql, export the result as JSONL (one object per line with
// `url` and `body` fields), then:
//
//   node scripts/real/load-httparchive.mjs --in results.jsonl
//
// Each row is re-checked locally with the same obfuscation filter the other
// fetchers use, then deduped and written to samples/real/httparchive/.
import { createReadStream } from "node:fs";
import { createInterface } from "node:readline";
import { shouldKeep, saveSample, loadSeen } from "./obfcheck.mjs";

const OUT = "samples/real/httparchive";
const args = process.argv.slice(2);
const inIdx = args.indexOf("--in");
if (inIdx < 0) {
  console.error("usage: node scripts/real/load-httparchive.mjs --in results.jsonl");
  process.exit(1);
}
const inFile = args[inIdx + 1];

const hostOf = (url) => {
  try {
    return new URL(url).hostname;
  } catch {
    return "site";
  }
};

const seen = loadSeen(OUT);
let kept = 0, rows = 0;

const rl = createInterface({ input: createReadStream(inFile), crlfDelay: Infinity });
for await (const line of rl) {
  const trimmed = line.trim();
  if (!trimmed) continue;
  rows++;
  let row;
  try {
    row = JSON.parse(trimmed);
  } catch {
    continue;
  }
  const code = row.body ?? row.response_body;
  const url = row.url ?? "site";
  if (typeof code !== "string") continue;
  const verdict = shouldKeep(code);
  if (!verdict.keep) continue;
  if (saveSample(OUT, hostOf(url), code, seen)) {
    kept++;
    if (kept % 100 === 0) process.stdout.write(`\rhttparchive: kept ${kept} / ${rows} rows   `);
  }
}

console.log(`\nhttparchive: kept ${kept} samples into ${OUT}/ (from ${rows} rows)`);
