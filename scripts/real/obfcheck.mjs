//
// Shared helpers for the real-world corpus fetchers: an obfuscation heuristic,
// content-hash dedupe + save, and small fetch/concurrency utilities. All three
// fetchers (npm, tranco, httparchive) funnel candidate scripts through
// `classify` + `saveSample` so the corpus is filtered and deduped the same way
// regardless of source.
//
import { writeFileSync, existsSync, mkdirSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { createHash } from "node:crypto";

const COUNT = (re, s) => (s.match(re) || []).length;

// Cheap structural signals that distinguish obfuscated / minified / plain JS.
// Densities are per-KB so thresholds don't depend on file size.
export function analyze(code) {
  const kb = Math.max(code.length / 1024, 0.001);
  const lines = code.split("\n");
  const avgLine = code.length / lines.length;

  const hexIds = COUNT(/_0x[0-9a-fA-F]{3,}/g, code); // _0x21ab decoder names
  const hexNums = COUNT(/\b0x[0-9a-fA-F]+\b/g, code); // 0x1f literals
  const bracketStr = COUNT(/\[(['"])(?:\\.|(?!\1).)*\1\]/g, code); // ["push"] access
  // String-array decoder fingerprints (rotate-by-shift IIFE that obfuscator.io emits).
  const saDecoder =
    /while\s*\(\s*!!\[\]\s*\)/.test(code) ||
    /\['push'\]\(\s*\w+\['shift'\]/.test(code) ||
    /\["push"\]\(\s*\w+\["shift"\]/.test(code);
  const evalish = COUNT(/\b(eval|Function)\s*\(/g, code);

  const signals = {
    bytes: code.length,
    avgLine: Math.round(avgLine),
    hexIdsPerKb: +(hexIds / kb).toFixed(2),
    hexNumsPerKb: +(hexNums / kb).toFixed(2),
    bracketPerKb: +(bracketStr / kb).toFixed(2),
    saDecoder,
    evalish,
  };

  // Weighted score, capped at 100. Heavy weight on the decoder fingerprint and
  // hex identifiers — those are near-certain obfuscator output.
  let score = 0;
  if (saDecoder) score += 45;
  score += Math.min(signals.hexIdsPerKb * 8, 30);
  score += Math.min(signals.bracketPerKb * 3, 15);
  score += Math.min(signals.hexNumsPerKb * 2, 10);
  score = Math.min(Math.round(score), 100);

  let label;
  if (saDecoder || signals.hexIdsPerKb >= 1 || score >= 35) label = "obfuscated";
  else if (avgLine >= 150) label = "minified";
  else label = "plain";

  return { score, label, signals };
}

// Keep decision: only obfuscated/minified, within a sane size band.
export function shouldKeep(code, { minBytes = 600, maxBytes = 512 * 1024 } = {}) {
  if (code.length < minBytes) return { keep: false, reason: "too-small" };
  if (code.length > maxBytes) return { keep: false, reason: "too-large" };
  const a = analyze(code);
  if (a.label === "plain") return { keep: false, reason: "not-obfuscated", ...a };
  return { keep: true, ...a };
}

const sha8 = (s) => createHash("sha256").update(s).digest("hex").slice(0, 8);

// Write <base>__<sha8>.js into dir, deduped by content hash. Returns the path,
// or null if an identical sample is already present (any source).
export function saveSample(dir, base, code, seen) {
  mkdirSync(dir, { recursive: true });
  const h = sha8(code);
  if (seen) {
    if (seen.has(h)) return null;
    seen.add(h);
  }
  const safe = base.replace(/[^a-zA-Z0-9._-]+/g, "_").slice(0, 60);
  const path = join(dir, `${safe}__${h}.js`);
  if (existsSync(path)) return null;
  writeFileSync(path, code);
  return path;
}

// Hashes already on disk under a dir tree, so reruns don't re-add duplicates.
export function loadSeen(dir) {
  const seen = new Set();
  const walk = (d) => {
    let ents;
    try {
      ents = readdirSync(d, { withFileTypes: true });
    } catch {
      return;
    }
    for (const e of ents) {
      if (e.isDirectory()) walk(join(d, e.name));
      else {
        const m = e.name.match(/__([0-9a-f]{8})\.js$/);
        if (m) seen.add(m[1]);
      }
    }
  };
  walk(dir);
  return seen;
}

export async function fetchText(url, { timeoutMs = 12000, maxBytes = 2 * 1024 * 1024 } = {}) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const res = await fetch(url, {
      signal: ctrl.signal,
      redirect: "follow",
      headers: { "user-agent": "devirt-corpus/0.1 (+https://github.com/devirt-dev)" },
    });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const buf = Buffer.from(await res.arrayBuffer());
    if (buf.length > maxBytes) throw new Error("body too large");
    return buf.toString("utf8");
  } finally {
    clearTimeout(t);
  }
}

// Run async fn over items with bounded concurrency; failures resolve to null.
export async function mapLimit(items, limit, fn) {
  const out = new Array(items.length);
  let i = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (i < items.length) {
      const idx = i++;
      try {
        out[idx] = await fn(items[idx], idx);
      } catch {
        out[idx] = null;
      }
    }
  });
  await Promise.all(workers);
  return out;
}
