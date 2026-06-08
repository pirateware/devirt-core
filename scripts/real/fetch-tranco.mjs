#!/usr/bin/env node
//
// Crawl top domains, extract their <script src> URLs, download the JS, and keep
// the ones that pass the obfuscation filter. Writes samples/real/tranco/.
//
//   node scripts/real/fetch-tranco.mjs                  # built-in ~60-domain demo list
//   node scripts/real/fetch-tranco.mjs --list top1m.csv # Tranco CSV ("rank,domain" or "domain")
//   node scripts/real/fetch-tranco.mjs --limit 2000     # crawl first N domains
//   node scripts/real/fetch-tranco.mjs --per-site 6     # max scripts kept per domain
//
// Get a full list from https://tranco-list.eu/ (download the CSV, unzip, pass
// with --list). This is the flakiest source — many sites block bots, lazy-load
// scripts, or serve no obfuscated JS — but it yields genuine in-the-wild samples.
import { readFileSync } from "node:fs";
import { fetchText, shouldKeep, saveSample, loadSeen, mapLimit } from "./obfcheck.mjs";

const OUT = "samples/real/tranco";
const args = process.argv.slice(2);
const opt = (k, d) => {
  const i = args.indexOf(k);
  return i >= 0 ? args[i + 1] : d;
};

const DEMO = [
  "cloudflare.com", "shopify.com", "stripe.com", "paypal.com", "booking.com",
  "airbnb.com", "linkedin.com", "twitch.tv", "reddit.com", "spotify.com",
  "netflix.com", "ebay.com", "walmart.com", "target.com", "bestbuy.com",
  "nytimes.com", "cnn.com", "bbc.com", "theguardian.com", "medium.com",
  "github.com", "gitlab.com", "stackoverflow.com", "npmjs.com", "vercel.com",
  "cloudinary.com", "datadoghq.com", "segment.com", "mixpanel.com", "hotjar.com",
  "zendesk.com", "intercom.com", "hubspot.com", "salesforce.com", "atlassian.com",
  "notion.so", "figma.com", "canva.com", "dropbox.com", "wetransfer.com",
  "godaddy.com", "namecheap.com", "wordpress.com", "wix.com", "squarespace.com",
  "etsy.com", "aliexpress.com", "wish.com", "expedia.com", "tripadvisor.com",
  "uber.com", "lyft.com", "doordash.com", "instacart.com", "coinbase.com",
  "binance.com", "kraken.com", "robinhood.com", "discord.com", "slack.com",
];

const listFile = opt("--list");
const limit = parseInt(opt("--limit", "0"), 10);
const perSite = parseInt(opt("--per-site", "8"), 10);

let domains = DEMO;
if (listFile) {
  domains = readFileSync(listFile, "utf8")
    .split("\n")
    .map((l) => l.trim())
    .filter(Boolean)
    .map((l) => (l.includes(",") ? l.split(",").pop() : l)); // strip "rank," prefix
}
if (limit > 0) domains = domains.slice(0, limit);

const seen = loadSeen(OUT);
let kept = 0, crawled = 0;

const absolutize = (src, base) => {
  try {
    return new URL(src, base).href;
  } catch {
    return null;
  }
};

await mapLimit(domains, 10, async (domain) => {
  const home = `https://${domain}/`;
  let html;
  try {
    html = await fetchText(home, { timeoutMs: 10000 });
  } catch {
    return;
  }
  crawled++;
  // Pull script srcs, prefer same-origin/CDN bundles, dedupe within the page.
  const srcs = [...html.matchAll(/<script\b[^>]*\bsrc=["']([^"']+)["']/gi)]
    .map((m) => absolutize(m[1], home))
    .filter(Boolean);
  const unique = [...new Set(srcs)];

  // Bound per-site work: examine at most 16 candidate scripts, 4 at a time, so
  // one slow domain can't stall the crawl.
  const candidates = unique.filter((u) => /\.js(\?|$)/i.test(u)).slice(0, 16);
  let perSiteKept = 0;
  await mapLimit(candidates, 4, async (url) => {
    if (perSiteKept >= perSite) return;
    const code = await fetchText(url, { timeoutMs: 6000 });
    const verdict = shouldKeep(code);
    if (!verdict.keep) return;
    if (saveSample(OUT, domain, code, seen)) {
      kept++;
      perSiteKept++;
      process.stdout.write(`\rtranco: kept ${kept} / crawled ${crawled} sites   `);
    }
  });
});

console.log(`\ntranco: kept ${kept} samples into ${OUT}/ (crawled ${crawled}/${domains.length} domains)`);
