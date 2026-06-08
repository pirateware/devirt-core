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

const PROFILES = {
  minimal: {},
  strings_b64: { stringArray: true, stringArrayEncoding: ["base64"], stringArrayThreshold: 1 },
  strings_rc4: { stringArray: true, stringArrayEncoding: ["rc4"], stringArrayThreshold: 1 },
  controlflow: { controlFlowFlattening: true, controlFlowFlatteningThreshold: 1 },
  deadcode: { deadCodeInjection: true, deadCodeInjectionThreshold: 1 },
  strong: {
    stringArray: true,
    stringArrayEncoding: ["rc4"],
    stringArrayThreshold: 1,
    controlFlowFlattening: true,
    controlFlowFlatteningThreshold: 1,
    deadCodeInjection: true,
    numbersToExpressions: true,
    simplify: true,
  },
};

mkdirSync(OUT, { recursive: true });
const seeds = readdirSync(SEEDS).filter((f) => f.endsWith(".js"));
if (seeds.length === 0) {
  console.error(`no seeds in ${SEEDS}/ — pull the corpus first (scripts/pull-corpus.sh)`);
  process.exit(1);
}

let n = 0;
for (const f of seeds) {
  const code = readFileSync(join(SEEDS, f), "utf8");
  const stem = basename(f, ".js");
  for (const [name, opts] of Object.entries(PROFILES)) {
    const out = JsObf.obfuscate(code, { seed: 1337, ...opts }).getObfuscatedCode();
    writeFileSync(join(OUT, `${stem}__${name}.js`), out);
    n++;
  }
}
console.log(`Generated ${n} files (${seeds.length} seeds x ${Object.keys(PROFILES).length} profiles) into ${OUT}/`);
