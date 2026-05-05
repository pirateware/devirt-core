# Real-World Stability Testing

This directory contains infrastructure for testing Reify against real-world JavaScript code.

## Directory Structure

```
corpus/realworld/
├── tier1/          # Small unminified libraries
│   ├── lodash_debounce.js
│   ├── lodash_cloneDeep.js
│   └── *.source.txt (provenance files)
├── tier2/          # Minified production bundles
│   ├── sample_bundle_1.js
│   ├── sample_bundle_2.js
│   └── *.source.txt
└── tier3/          # Obfuscated samples
    ├── obfuscated_sample_1.js
    ├── obfuscated_sample_2.js
    └── *.source.txt

tests/realworld_regressions/  # Regression tests for failures
└── unary_neg_panic.rs
```

## Running the Tests

From the `crates/mvp` directory:

```bash
cargo run --example realworld_runner -- ../../corpus/realworld
```

This will:
1. Parse each file to IR
2. Validate the IR
3. Emit back to JS
4. Re-parse the emitted code
5. Compare IR versions (roundtrip check)
6. Generate coverage statistics
7. Write results to `docs/REALWORLD_STABILITY.md`

## Adding New Test Cases

1. Add JS files to the appropriate tier directory
2. Create a `.source.txt` file with provenance information:
   - Source URL
   - License
   - Date fetched
   - Description

## Test Results

See [docs/REALWORLD_STABILITY.md](../../docs/REALWORLD_STABILITY.md) for the latest test results.

## Decision Rules

Based on the test results:

- **Panics/crashes**: Always bugs, must be fixed immediately
- **Tier 1 parse failures**: Likely real bugs in the parser
- **Tier 2/3 parse failures**: May be expected if they use unsupported constructs (async, classes, modules)
- **Roundtrip failures**: Always bugs in emit or re-parsing
- **High passthrough (>20%)**: Investigate dominant unlowered constructs
