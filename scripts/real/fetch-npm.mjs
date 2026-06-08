#!/usr/bin/env node
//
// Fetch minified bundles of popular npm packages via jsDelivr and keep the ones
// that pass the obfuscation/minification filter. Writes samples/real/npm/.
//
//   node scripts/real/fetch-npm.mjs                 # built-in popular-package list
//   node scripts/real/fetch-npm.mjs --list pkgs.txt # one "name" or "name@version" per line
//   node scripts/real/fetch-npm.mjs --limit 800     # cap number of packages tried
//
// jsDelivr's flat API lists a package's files; we grab every *.min.js (often
// several entry points / locales per package), so a few hundred packages yields
// a few thousand candidate bundles. These are mostly minified rather than
// heavily obfuscated — `report` will show that in the npm row.
import { readFileSync } from "node:fs";
import { fetchText, shouldKeep, saveSample, loadSeen, mapLimit } from "./obfcheck.mjs";

const OUT = "samples/real/npm";
const args = process.argv.slice(2);
const opt = (k, d) => {
  const i = args.indexOf(k);
  return i >= 0 ? args[i + 1] : d;
};

const POPULAR = [
  "react", "react-dom", "lodash", "jquery", "moment", "axios", "vue", "d3",
  "three", "chart.js", "bootstrap", "rxjs", "core-js", "underscore", "anime.js",
  "gsap", "pixi.js", "babylonjs", "monaco-editor", "codemirror", "ace-builds",
  "video.js", "hls.js", "dashjs", "socket.io-client", "firebase", "stripe",
  "@sentry/browser", "html2canvas", "jspdf", "pdfjs-dist", "fabric", "konva",
  "leaflet", "mapbox-gl", "echarts", "highcharts", "plotly.js-dist", "katex",
  "mathjs", "crypto-js", "jsencrypt", "forge", "uuid", "dayjs", "luxon",
  "marked", "dompurify", "handlebars", "mustache", "ejs", "swiper", "flatpickr",
  "tippy.js", "popper.js", "@popperjs/core", "sortablejs", "dragula", "hammerjs",
  "lottie-web", "particles.js", "tsparticles", "typed.js", "aos", "wow.js",
];

const listFile = opt("--list");
const limit = parseInt(opt("--limit", "0"), 10);
let pkgs = listFile
  ? readFileSync(listFile, "utf8").split("\n").map((l) => l.trim()).filter(Boolean)
  : POPULAR;
if (limit > 0) pkgs = pkgs.slice(0, limit);

const seen = loadSeen(OUT);
let kept = 0, tried = 0;

// List the *.min.js files jsDelivr has for a package (resolves "latest").
async function minFiles(pkg) {
  const at = pkg.lastIndexOf("@");
  const name = at > 0 ? pkg.slice(0, at) : pkg; // index 0 = scope @, not a version
  let ver = at > 0 ? pkg.slice(at + 1) : null;
  if (!ver) {
    // The data API rejects "latest"/tags — resolve the concrete version first.
    const top = JSON.parse(await fetchText(`https://data.jsdelivr.com/v1/packages/npm/${name}`));
    ver = top.tags?.latest;
    if (!ver) return [];
  }
  const meta = JSON.parse(await fetchText(`https://data.jsdelivr.com/v1/packages/npm/${name}@${ver}`));
  const files = [];
  const walk = (node, prefix) => {
    for (const f of node.files || []) {
      if (f.type === "directory") walk(f, `${prefix}/${f.name}`);
      else if (f.name.endsWith(".min.js")) files.push(`${prefix}/${f.name}`);
    }
  };
  walk(meta, "");
  return files.map((p) => ({
    url: `https://cdn.jsdelivr.net/npm/${name}@${meta.version}${p}`,
    base: `${name}${p.replace(/\//g, "_")}`.replace(/\.min\.js$/, ""),
  }));
}

// Phase 1: resolve each package's min.js files concurrently (cap per package so
// a mega-package can't dominate the corpus).
const lists = await mapLimit(pkgs, 6, async (pkg) => {
  try {
    return (await minFiles(pkg)).slice(0, 8);
  } catch {
    return [];
  }
});
const targets = lists.flat();

// Phase 2: fetch + filter the flat file list concurrently, so one slow/large
// bundle never blocks the others.
await mapLimit(targets, 8, async ({ url, base }) => {
  tried++;
  try {
    const code = await fetchText(url, { timeoutMs: 8000 });
    const verdict = shouldKeep(code);
    if (!verdict.keep) return;
    if (saveSample(OUT, base, code, seen)) {
      kept++;
      process.stdout.write(`\rnpm: kept ${kept} / tried ${tried}   `);
    }
  } catch {
    /* skip unreachable/oversized */
  }
});

console.log(`\nnpm: kept ${kept} samples into ${OUT}/ (from ${pkgs.length} packages, ${tried} files tried)`);
