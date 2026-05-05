# Real-World Stability Test Report

Generated: 2025-01-05

This report tests the Reify parse → IR → emit pipeline against real-world JavaScript.

## Summary

| Metric | Count | Percentage |
|--------|-------|------------|
| Total files tested | 6 | 100% |
| Parse success | 5/6 | 83.3% |
| Validation success | 5/6 | 83.3% |
| Emit success | 5/6 | 83.3% |
| Re-parse success | 5/6 | 83.3% |
| Roundtrip success | 5/6 | 83.3% |
| Panics/crashes | 1 | 16.7% |

## Tier 1 - Small Libraries (Unminified)

Tier 1 files are small, unminified library modules suitable for behavioral equivalence testing.

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| lodash_cloneDeep.js | ✗ | ✗ | ✗ | ✗ | ✗ | 0.0% | 0.0% | PANIC: unary negation not implemented |
| lodash_debounce.js | ✓ | ✓ | ✓ | ✓ | ✓ | 87.0% | 686.0% | High passthrough - top-level driver statements |

**Analysis:**
- `lodash_cloneDeep.js`: Panics during SSA lowering due to unimplemented unary negation operator (`-1`, `-x`)
- `lodash_debounce.js`: Successfully processes but has 87% function-body passthrough due to unsupported constructs (try/catch, spread, etc.)

## Tier 2 - Minified Production Bundles

Tier 2 files are synthetic minified bundles representative of production code.

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| sample_bundle_1.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 1050.0% | Full roundtrip success |
| sample_bundle_2.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 1021.0% | Full roundtrip success |

**Analysis:**
- Both minified bundles process successfully
- High top-level passthrough is expected (IIFE wrappers, module boilerplate)
- No behavioral testing performed (Tier 2 only tests structural roundtrip)

## Tier 3 - Obfuscated Samples

Tier 3 files contain obfuscated code patterns from tools like javascript-obfuscator.

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| obfuscated_sample_1.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 564.0% | Full roundtrip success |
| obfuscated_sample_2.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 309.0% | Full roundtrip success |

**Analysis:**
- Both obfuscated samples process successfully
- String array obfuscation and hex identifiers are preserved
- No behavioral testing performed (Tier 3 only tests structural roundtrip)
- **Warning**: Do not execute these files - they are for analysis only

## Issues Found

### Critical: Panic in SSA Lowering

**File:** `Tier 1 - lodash_cloneDeep.js`

**Error:** `not implemented: unary neg not yet lowered`

**Location:** `src/ir/lower_ssa.rs:2388`

**Description:** The SSA lowering panics when encountering unary negation operators (e.g., `-1`, `-x`). This is a hard crash that prevents processing any code containing negation.

**Minimal Reproducer:**
```javascript
function test() {
    var x = -1;  // Panics here
    return x;
}
```

**Regression Test:** `tests/realworld_regressions/unary_neg_panic.rs`

**Recommendation:** Implement unary negation lowering in SSA module.

## Coverage Analysis

### Function-Body Passthrough (FB-PT%)

High passthrough indicates code paths the lowering doesn't yet support:

- **87% for lodash_debounce.js**: Top functions passthrough due to:
  - Destructuring parameters
  - Spread operators
  - Try/catch blocks
  - Complex property access patterns

This is expected for complex library code. Focus optimization efforts on:
1. Unary operators (negation causing panics)
2. Destructuring patterns
3. Try/catch exception handling

### Top-Level Passthrough (TL-PT%)

High top-level passthrough is generally acceptable:
- Module wrapper IIFEs
- Export/require statements
- Top-level assignments

These don't affect the core optimization pipeline.

## Decision Rules Applied

Based on the test results:

1. ✅ **Panic detection**: 1 panic found (unary negation)
   - **Action Required**: Fix before any further work
   - **Assigned to**: SSA lowering module

2. ✅ **Tier 1 parse failure classification**: 
   - `lodash_cloneDeep.js` panics on supported construct (unary negation)
   - **Classification**: BUG - this construct should be supported

3. ✅ **Roundtrip failures**: None (5/6 files roundtrip successfully)

4. ⚠️ **High passthrough**: 
   - `lodash_debounce.js` has 87% FB-PT
   - **Action**: Report dominant unlowered constructs in this document
   - **Recommendation**: Address unary negation first, then destructuring

## Recommendations

### Immediate Actions (Before Next Work Session)

1. **Fix unary negation panic** (Critical)
   - Implement `UnOp::Neg` lowering in `lower_ssa.rs`
   - Add test case from regression file
   - Verify fix with `lodash_cloneDeep.js`

2. **Add more Tier 1 library samples**
   - Current: 2 files (target: 10 files)
   - Missing: date-fns modules, minimatch, more lodash utilities
   - Priority: Get realistic coverage of common library patterns

### Follow-up Work

3. **Implement destructuring support**
   - High impact for library code
   - Would significantly reduce FB-PT% for lodash modules

4. **Expand Tier 2/3 corpus**
   - Current: 2 files each
   - Target: 3-5 files each
   - Source: Real production bundles from CDNs

5. **Add behavioral testing for Tier 1**
   - Currently only structural testing
   - Verify emitted code produces same output as original

## Test Harness

To reproduce these results:

```bash
cd crates/mvp
cargo run --example realworld_runner -- ../../corpus/realworld
```

This generates:
- Console output with per-file status
- `../../docs/REALWORLD_STABILITY.md` (this file)

## Files Tested

### Tier 1
- `lodash_debounce.js` - MIT License, from unpkg.com
- `lodash_cloneDeep.js` - MIT License, simplified from unpkg.com

### Tier 2
- `sample_bundle_1.js` - Synthetic webpack-style bundle
- `sample_bundle_2.js` - Synthetic bundle with arrow functions

### Tier 3
- `obfuscated_sample_1.js` - Synthetic javascript-obfuscator style
- `obfuscated_sample_2.js` - Synthetic control flow flattening

See individual `.source.txt` files in corpus directory for full provenance.

## Appendix: Passthrough Calculation

**Function-Body Passthrough % (FB-PT%)**:
- Numerator: Bytes in PassThrough function bodies
- Denominator: (Lowered statements × 30B proxy) + PassThrough bytes
- Threshold for concern: >20%

**Top-Level Passthrough % (TL-PT%)**:
- Raw byte count of top-level PassThrough statements
- Informational only (driver code doesn't affect optimization)
