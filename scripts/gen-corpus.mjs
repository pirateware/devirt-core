#!/usr/bin/env node
//
// Generate the obfuscated corpus from plain seeds.
//
//   npm i javascript-obfuscator      # local tooling only; never part of the crate
//   node scripts/gen-corpus.mjs      # reads samples/seeds/, writes samples/generated/
//
// For every samples/seeds/*.js, obfuscate it under each profile below and write
// samples/generated/<seed>__<profile>.js. Then push ./samples to the Hugging
// Face dataset to publish. Deterministic: a fixed `seed` per run, so regenerating
// the same seeds yields byte-identical output.
//
// Profiles deliberately exclude self-defending / debug-protection / console
// suppression — those add anti-tamper that breaks plain execution. They cover
// the transforms a deobfuscator must see through.
import { readFileSync, writeFileSync, readdirSync, mkdirSync } from "node:fs";
import { join, basename } from "node:path";
import JsObf from "javascript-obfuscator";

const SEEDS = "samples/seeds";
const OUT = "samples/generated";

// A matrix of named profiles spanning javascript-obfuscator's technique space.
// Each isolates or combines transforms a deobfuscator must see through; the
// names become the `__<profile>` suffix so `report` can aggregate per technique.
const SA = (enc, extra = {}) => ({
  stringArray: true,
  stringArrayEncoding: [enc],
  stringArrayThreshold: 1,
  ...extra,
});
const PROFILES = {
  minimal: {},
  strings_none: SA("none"),
  strings_b64: SA("base64"),
  strings_rc4: SA("rc4"),
  strings_rotate: SA("base64", { stringArrayRotate: true, stringArrayShuffle: true }),
  strings_wrappers: SA("rc4", { stringArrayWrappersCount: 3, stringArrayWrappersType: "function" }),
  strings_indexshift: SA("rc4", { stringArrayIndexShift: true, stringArrayWrappersCount: 2 }),
  splitstrings: { splitStrings: true, splitStringsChunkLength: 4 },
  numbers: { numbersToExpressions: true },
  objectkeys: { transformObjectKeys: true },
  names_hex: { identifierNamesGenerator: "hexadecimal", renameGlobals: true },
  names_mangled: { identifierNamesGenerator: "mangled", renameGlobals: true },
  controlflow: { controlFlowFlattening: true, controlFlowFlatteningThreshold: 1 },
  controlflow_half: { controlFlowFlattening: true, controlFlowFlatteningThreshold: 0.5 },
  deadcode: { deadCodeInjection: true, deadCodeInjectionThreshold: 1 },
  cf_dead: {
    controlFlowFlattening: true,
    controlFlowFlatteningThreshold: 1,
    deadCodeInjection: true,
    deadCodeInjectionThreshold: 1,
  },
  sa_cf: SA("rc4", { controlFlowFlattening: true, controlFlowFlatteningThreshold: 1 }),
  sa_numbers: SA("base64", { numbersToExpressions: true }),
  strong: SA("rc4", {
    stringArrayRotate: true,
    stringArrayShuffle: true,
    stringArrayWrappersCount: 2,
    controlFlowFlattening: true,
    controlFlowFlatteningThreshold: 1,
    deadCodeInjection: true,
    numbersToExpressions: true,
    transformObjectKeys: true,
    splitStrings: true,
    splitStringsChunkLength: 5,
    simplify: true,
  }),
};

// Profiles deliberately exclude self-defending / debug-protection / console
// suppression — anti-tamper that breaks plain execution.
const selected = (() => {
  const i = process.argv.indexOf("--profiles");
  if (i < 0) return Object.keys(PROFILES);
  return process.argv[i + 1].split(",").filter((p) => PROFILES[p]);
})();

mkdirSync(OUT, { recursive: true });
const seeds = readdirSync(SEEDS).filter((f) => f.endsWith(".js"));
if (seeds.length === 0) {
  console.error(`no seeds in ${SEEDS}/ — pull the corpus first (scripts/pull-corpus.sh)`);
  process.exit(1);
}

let n = 0, failed = 0;
for (const f of seeds) {
  const code = readFileSync(join(SEEDS, f), "utf8");
  const stem = basename(f, ".js");
  for (const name of selected) {
    try {
      const out = JsObf.obfuscate(code, { seed: 1337, ...PROFILES[name] }).getObfuscatedCode();
      writeFileSync(join(OUT, `${stem}__${name}.js`), out);
      n++;
    } catch {
      failed++; // a profile/seed combo the obfuscator rejects — skip, don't abort
    }
  }
  if (n % 500 === 0 && n) process.stdout.write(`\rgenerated ${n} files...   `);
}
console.log(
  `\nGenerated ${n} files (${seeds.length} seeds x ${selected.length} profiles) into ${OUT}/` +
    (failed ? ` (${failed} combos skipped)` : "")
);
