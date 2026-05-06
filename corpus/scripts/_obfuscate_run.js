#!/usr/bin/env node
// Helper invoked by fetch_obfuscated.sh.
// Runs all three tier4 obfuscations via the programmatic APIs (not CLI),
// because javascript-obfuscator's CLI rejects paths containing hidden
// directory components — and worktrees commonly live under .claude/.
//
// Usage: _obfuscate_run.js <src> <out_dir> <obfuscatorio|jsconfuser_high|jsconfuser_vm>
//
// JsConfuser 2.0.1 does NOT support a seed parameter, so the two
// jsconfuser_* outputs are snapshots — re-running produces different
// bytes. This is documented in tier4/GENERATION.md.

"use strict";

const fs = require("fs");
const path = require("path");

const [, , srcPath, outDir, mode] = process.argv;
if (!srcPath || !outDir || !mode) {
    console.error("usage: _obfuscate_run.js <src> <out_dir> <mode>");
    process.exit(2);
}

const src = fs.readFileSync(srcPath, "utf8");

function runObfuscatorIo() {
    const JO = require("javascript-obfuscator");
    const result = JO.obfuscate(src, {
        target: "browser",
        optionsPreset: "high-obfuscation",
        seed: 1
    });
    const outPath = path.join(outDir, "baseline.obfuscatorio_high.js");
    fs.writeFileSync(outPath, result.getObfuscatedCode());
    console.log("ok obfuscatorio_high (" + result.getObfuscatedCode().length + " bytes)");
}

async function runJsConfuser(vm) {
    const J = require("js-confuser");
    const opts = Object.assign({}, J.presets.high, { target: "node" });
    if (vm) opts.rgf = true;
    const result = await J.obfuscate(src, opts);
    const outPath = path.join(outDir, vm ? "baseline.jsconfuser_vm.js" : "baseline.jsconfuser_high.js");
    fs.writeFileSync(outPath, result.code);
    console.log("ok " + (vm ? "jsconfuser_vm" : "jsconfuser_high") + " (" + result.code.length + " bytes)");
}

(async () => {
    try {
        if (mode === "obfuscatorio") {
            runObfuscatorIo();
        } else if (mode === "jsconfuser_high") {
            await runJsConfuser(false);
        } else if (mode === "jsconfuser_vm") {
            await runJsConfuser(true);
        } else {
            console.error("unknown mode: " + mode);
            process.exit(2);
        }
    } catch (err) {
        console.error("FAIL " + mode + ": " + (err.stack || err.message));
        process.exit(1);
    }
})();
