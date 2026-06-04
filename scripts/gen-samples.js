#!/usr/bin/env node
//
// Generate the synthetic obfuscated sample corpus.
//
//   node scripts/gen-samples.js
//
// For every seed in samples/seeds/*.js, obfuscate it under each profile below
// with javascript-obfuscator (a FIXED `seed` per profile, so regeneration is
// reproducible) and write the result to samples/generated/<seed>__<profile>.js.
//
// The seed's own output (run once in Node) is the ground truth. Each obfuscated
// file is re-run and kept ONLY if it still prints exactly that output — a broken
// obfuscation (or a non-deterministic seed) is skipped with a warning, so every
// committed sample is a valid, behavior-preserving obfuscation. The mapping
// sample -> expected output lines is written to samples/generated/manifest.json,
// which the Rust corpus test asserts the *deobfuscated* form reproduces.
//
// Profiles deliberately exclude self-defending / debug-protection / console
// suppression: those add anti-tamper that breaks plain execution and would make
// the fixtures untestable. They DO cover the transforms a deobfuscator must see
// through: string arrays (base64/rc4, rotate, shuffle), control-flow flattening,
// dead-code injection, number-to-expression, string splitting, key transforms.

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");
const JsObf = require("javascript-obfuscator");

const ROOT = path.join(__dirname, "..");
const SEEDS = path.join(ROOT, "samples", "seeds");
const OUT = path.join(ROOT, "samples", "generated");

// Each profile: a fixed `seed` (reproducible output) + the technique it exercises.
const PROFILES = {
  minimal: {
    seed: 1001,
    compact: true,
    identifierNamesGenerator: "mangled",
    simplify: true,
  },
  strarr_base64: {
    seed: 1002,
    compact: true,
    stringArray: true,
    stringArrayEncoding: ["base64"],
    stringArrayThreshold: 1,
    rotateStringArray: true,
    shuffleStringArray: true,
    identifierNamesGenerator: "hexadecimal",
  },
  strarr_rc4: {
    seed: 1003,
    compact: true,
    stringArray: true,
    stringArrayEncoding: ["rc4"],
    stringArrayThreshold: 1,
    rotateStringArray: true,
    shuffleStringArray: true,
    splitStrings: true,
    splitStringsChunkLength: 6,
  },
  controlflow: {
    seed: 1004,
    compact: true,
    controlFlowFlattening: true,
    controlFlowFlatteningThreshold: 1,
    stringArray: true,
    stringArrayThreshold: 1,
  },
  deadcode: {
    seed: 1005,
    compact: true,
    deadCodeInjection: true,
    deadCodeInjectionThreshold: 1,
    stringArray: true,
    stringArrayThreshold: 1,
  },
  numbers_keys: {
    seed: 1006,
    compact: true,
    numbersToExpressions: true,
    transformObjectKeys: true,
    stringArray: true,
    stringArrayEncoding: ["base64"],
    stringArrayThreshold: 1,
  },
  strong: {
    seed: 1007,
    compact: true,
    controlFlowFlattening: true,
    controlFlowFlatteningThreshold: 0.75,
    deadCodeInjection: true,
    deadCodeInjectionThreshold: 0.4,
    numbersToExpressions: true,
    splitStrings: true,
    splitStringsChunkLength: 5,
    transformObjectKeys: true,
    stringArray: true,
    stringArrayEncoding: ["rc4"],
    stringArrayThreshold: 1,
    rotateStringArray: true,
    shuffleStringArray: true,
  },
};

// Run a JS file in a fresh Node subprocess, capturing console output as lines.
// Uses a tiny preload that overrides console.log so only program output is seen.
function runLines(file) {
  const harness = `
    const __lines = [];
    const __rec = (...a) => __lines.push(a.map(String).join(" "));
    console.log = __rec; console.info = __rec; console.warn = __rec; console.error = __rec;
    try { require(${JSON.stringify(file)}); }
    catch (e) { process.stdout.write("@@THREW@@" + (e && e.name || "Error")); process.exit(0); }
    process.stdout.write(JSON.stringify(__lines));
  `;
  const out = execFileSync(process.execPath, ["-e", harness], {
    timeout: 15000,
    maxBuffer: 64 * 1024 * 1024,
  }).toString();
  if (out.startsWith("@@THREW@@")) return { threw: out.slice(9) };
  return { lines: JSON.parse(out) };
}

function main() {
  fs.mkdirSync(OUT, { recursive: true });
  // Clear stale generated files (but keep the dir).
  for (const f of fs.readdirSync(OUT)) {
    if (f.endsWith(".js") || f === "manifest.json") fs.unlinkSync(path.join(OUT, f));
  }

  const seeds = fs
    .readdirSync(SEEDS)
    .filter((f) => f.endsWith(".js"))
    .sort();

  const manifest = {};
  let written = 0,
    skipped = 0;

  for (const seedFile of seeds) {
    const seedPath = path.join(SEEDS, seedFile);
    const expected = runLines(seedPath);
    if (expected.threw || !expected.lines) {
      console.error(`SKIP seed ${seedFile}: did not run cleanly`);
      continue;
    }
    const base = seedFile.replace(/\.js$/, "");
    const src = fs.readFileSync(seedPath, "utf8");

    for (const [profile, opts] of Object.entries(PROFILES)) {
      let obf;
      try {
        obf = JsObf.obfuscate(src, opts).getObfuscatedCode();
      } catch (e) {
        console.error(`SKIP ${base}__${profile}: obfuscator error ${e.message}`);
        skipped++;
        continue;
      }
      const outName = `${base}__${profile}.js`;
      const outPath = path.join(OUT, outName);
      fs.writeFileSync(outPath, obf);

      // Sanity: the obfuscated form must still print exactly the seed output.
      const got = runLines(outPath);
      if (got.threw || JSON.stringify(got.lines) !== JSON.stringify(expected.lines)) {
        console.error(
          `SKIP ${outName}: obfuscated output != seed output ` +
            `(threw=${got.threw || "no"})`
        );
        fs.unlinkSync(outPath);
        skipped++;
        continue;
      }
      manifest[outName] = { seed: seedFile, profile, expected: expected.lines };
      written++;
    }
  }

  fs.writeFileSync(
    path.join(OUT, "manifest.json"),
    JSON.stringify(manifest, null, 2) + "\n"
  );
  console.log(
    `generated ${written} samples (${skipped} skipped) from ${seeds.length} seeds -> samples/generated/`
  );
}

main();
