# Real-World Stability Test Report

This report tests the parse → IR → emit pipeline against real-world JavaScript.

## Summary

- Total files tested: 308
- Parse success: 308/308 (100.0%)
- Roundtrip success: 308/308 (100.0%)
- Panics/crashes: 0

## Tier 1

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| datefns_addMonths.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 49.4% |  |
| lodash_throttle.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 26.5% | High PT% |
| lodash_uniqBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.4% |  |
| lodash_flatten.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 53.5% |  |
| lodash_snakeCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_identity.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 46.4% |  |
| lodash_sortBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_cloneDeep.js | ✓ | ✓ | ✓ | ✓ | ✓ | 6.3% | 33.2% |  |
| datefns_isBefore.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.2% |  |
| datefns_format.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.8% | 19.2% | High PT% |
| lodash_without.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_mapValues.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 53.0% |  |
| lodash_set.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 65.5% |  |
| lodash_orderBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 30.4% |  |
| lodash_times.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 43.0% |  |
| lodash_groupBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_formatDistance.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 21.2% | High PT% |
| lodash_chunk.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 31.7% | High PT% |
| lodash_difference.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_union.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_debounce.js | ✓ | ✓ | ✓ | ✓ | ✓ | 87.0% | 15.7% | High PT% |
| lodash_camelCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_compact.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 8.5% |  |
| datefns_parseISO.js | ✓ | ✓ | ✓ | ✓ | ✓ | 59.5% | 14.9% | High PT% |
| lodash_flow.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_isAfter.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.1% |  |
| lodash_startCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_escape.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.1% |  |
| lodash_noop.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_keyBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_merge.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partial.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_subDays.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 83.0% |  |
| lodash_invert.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_isEqual.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.7% |  |
| lodash_capitalize.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.1% |  |
| lodash_kebabCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partition.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_has.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.8% |  |
| lodash_get.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 48.7% |  |
| lodash_intersection.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_range.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_endOfMonth.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 67.0% |  |
| lodash_uniq.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 66.7% |  |
| lodash_curry.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 48.3% |  |
| lodash_omit.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_getDay.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.1% |  |
| lodash_zip.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_addDays.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 61.9% |  |
| datefns_startOfDay.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.3% |  |
| lodash_memoize.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 19.6% | High PT% |
| lodash_mapKeys.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 52.7% |  |
| lodash_constant.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 30.2% |  |
| datefns_differenceInDays.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 50.4% |  |
| lodash_pick.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_isPlainObject.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 53.4% | High PT% |

## Tier 2

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| datefns_addMonths.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 57.8% |  |
| lodash_isPlainObject.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 59.6% | High PT% |
| chart_js.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_memoize.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 29.5% | High PT% |
| datefns_addDays.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.3% |  |
| axios.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| sample_bundle_2.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_compact.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 21.5% | High PT% |
| lodash_mapKeys.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.3% |  |
| d3.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_orderBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.8% |  |
| lodash_constant.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 28.6% |  |
| lodash_zip.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| ramda.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_throttle.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 38.4% | High PT% |
| mobx.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| moment.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_get.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 46.9% |  |
| lodash_camelCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_differenceInDays.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 58.8% |  |
| core_js_bundle.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_groupBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| redux.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_snakeCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| rxjs.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_set.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 63.9% |  |
| datefns_parseISO.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 43.7% | 21.7% | High PT% |
| lodash_curry.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 62.5% |  |
| lodash_capitalize.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 76.2% |  |
| react.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_pick.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_escape.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 85.6% |  |
| lodash_omit.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_keyBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| zod.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| marked.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partition.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partial.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_isAfter.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 73.9% |  |
| datefns_isBefore.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 73.9% |  |
| lodash_union.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_identity.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 44.4% |  |
| lodash_uniq.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 65.1% |  |
| lodash.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| dayjs.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| uuid.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_uniqBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 76.6% |  |
| lodash_debounce.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 64.5% | 29.2% | High PT% |
| lodash_without.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_flatten.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 68.4% |  |
| lodash_difference.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| underscore.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| react_dom.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_flow.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_getDay.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 91.8% |  |
| tslib.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_invert.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| preact.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| three.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_noop.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| sample_bundle_1.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_merge.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_range.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| luxon.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_startOfDay.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 79.2% |  |
| lodash_sortBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_times.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 53.0% | High PT% |
| datefns_formatDistance.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 36.4% | High PT% |
| lodash_chunk.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 47.7% | High PT% |
| lodash_intersection.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_isEqual.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 68.4% |  |
| datefns_format.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.4% | 34.2% | High PT% |
| vue.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_kebabCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_endOfMonth.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 74.0% |  |
| lodash_mapValues.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.6% |  |
| jquery.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_has.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 73.5% |  |
| immer.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_subDays.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 82.0% |  |
| lodash_startCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_cloneDeep.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 11.4% | 35.7% |  |

## Tier 3

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| lodash_without.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 40.2% | High PT% |
| lodash_identity.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 80.6% | 26.6% | High PT% |
| lodash_merge.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 35.3% | High PT% |
| lodash_mapKeys.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 40.3% | High PT% |
| lodash_escape.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.9% |  |
| lodash_constant.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 35.6% | High PT% |
| datefns_endOfMonth.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 86.8% | 28.9% | High PT% |
| lodash_difference.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 80.9% |  |
| lodash_zip.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.0% | 44.2% | High PT% |
| lodash_identity.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.7% |  |
| datefns_addMonths.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 62.4% |  |
| datefns_isBefore.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.2% |  |
| lodash_range.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 40.2% | High PT% |
| lodash_cloneDeep.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 6.7% | 36.3% |  |
| datefns_addMonths.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 43.9% | 42.4% | High PT% |
| lodash_isEqual.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 58.4% | 39.4% | High PT% |
| lodash_partition.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 31.5% | High PT% |
| lodash_invert.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 51.6% | High PT% |
| datefns_addDays.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 87.2% | 29.7% | High PT% |
| lodash_flow.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 27.4% | High PT% |
| lodash_debounce.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 74.3% | 29.3% | High PT% |
| lodash_without.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 45.7% | High PT% |
| lodash_debounce.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 73.7% | 25.1% | High PT% |
| datefns_addDays.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 47.8% | 45.0% | High PT% |
| lodash_pick.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.2% |  |
| lodash_merge.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 44.2% | High PT% |
| lodash_intersection.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 50.0% | High PT% |
| lodash_escape.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 77.7% | 27.6% | High PT% |
| lodash_groupBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 81.3% |  |
| lodash_omit.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 59.8% | High PT% |
| datefns_formatDistance.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.5% | 10.7% | High PT% |
| lodash_noop.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.5% |  |
| lodash_orderBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 87.8% | 21.5% | High PT% |
| lodash_compact.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 47.8% | 34.5% | High PT% |
| lodash_cloneDeep.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 63.8% | 19.4% | High PT% |
| lodash_keyBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.7% |  |
| lodash_zip.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.4% |  |
| lodash_partial.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 81.4% |  |
| lodash_capitalize.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.6% |  |
| lodash_times.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 58.5% | 58.0% | High PT% |
| datefns_getDay.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.1% |  |
| lodash_chunk.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 71.7% | 37.1% | High PT% |
| lodash_startCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.1% |  |
| lodash_without.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.0% |  |
| lodash_flatten.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 38.3% | High PT% |
| lodash_omit.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 86.8% |  |
| lodash_memoize.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 73.3% | 47.7% | High PT% |
| lodash_get.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 39.0% | High PT% |
| lodash_difference.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 39.9% | High PT% |
| datefns_formatDistance.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.5% | 25.0% | High PT% |
| lodash_noop.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.5% | 25.9% | High PT% |
| datefns_getDay.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 76.9% | 34.0% | High PT% |
| lodash_chunk.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.3% | 18.9% | High PT% |
| lodash_groupBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 40.8% | High PT% |
| datefns_addDays.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 67.6% |  |
| lodash_sortBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 50.7% | High PT% |
| datefns_differenceInDays.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 42.1% | 40.4% | High PT% |
| datefns_parseISO.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 49.5% | 24.1% | High PT% |
| lodash_union.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 59.8% | 47.0% | High PT% |
| lodash_set.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.3% |  |
| lodash_compact.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 71.1% | 23.4% | High PT% |
| lodash_range.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 27.7% | High PT% |
| lodash_has.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 73.2% |  |
| lodash_intersection.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 79.7% |  |
| lodash_isPlainObject.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 62.7% | 62.3% | High PT% |
| lodash_difference.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 49.3% | High PT% |
| datefns_subDays.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 77.8% | 37.4% | High PT% |
| lodash_debounce.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.8% | 7.9% | High PT% |
| lodash_uniq.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.5% |  |
| lodash_capitalize.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 76.9% | 28.5% | High PT% |
| lodash_groupBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 51.2% | High PT% |
| obfuscated_sample_2.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_uniq.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 77.8% | 28.2% | High PT% |
| lodash_orderBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.7% |  |
| datefns_endOfMonth.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.0% |  |
| lodash_startCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 38.0% | High PT% |
| lodash_memoize.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 75.3% | 30.5% | High PT% |
| lodash_capitalize.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 40.1% | High PT% |
| lodash_keyBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 44.3% | High PT% |
| lodash_mapKeys.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 87.7% | 23.8% | High PT% |
| lodash_kebabCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.9% |  |
| datefns_isAfter.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 52.6% | 45.1% | High PT% |
| datefns_parseISO.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.4% | 8.0% | High PT% |
| datefns_differenceInDays.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 85.6% | 24.9% | High PT% |
| lodash_has.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 76.9% | 24.9% | High PT% |
| lodash_times.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 69.8% | 37.1% | High PT% |
| lodash_zip.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.5% | 30.9% | High PT% |
| lodash_get.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 75.1% | 25.6% | High PT% |
| lodash_times.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 89.1% | 21.0% | High PT% |
| datefns_isAfter.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 73.3% | 35.8% | High PT% |
| lodash_camelCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 47.6% | High PT% |
| lodash_isPlainObject.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 89.7% | 24.5% | High PT% |
| datefns_isAfter.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.7% |  |
| lodash_isEqual.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.5% |  |
| lodash_merge.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.3% |  |
| datefns_startOfDay.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.5% |  |
| lodash_mapValues.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 71.8% |  |
| lodash_throttle.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 61.7% | 54.5% | High PT% |
| lodash_uniqBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.5% |  |
| lodash_has.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 58.4% | 43.1% | High PT% |
| lodash_escape.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 42.0% | High PT% |
| datefns_getDay.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 52.6% | 45.6% | High PT% |
| lodash_invert.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 82.4% |  |
| lodash_flatten.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 75.8% | 29.2% | High PT% |
| lodash_memoize.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.1% | 16.0% | High PT% |
| lodash_range.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.7% |  |
| lodash_flatten.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.1% |  |
| datefns_differenceInDays.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 58.4% |  |
| lodash_noop.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 38.0% | High PT% |
| lodash_isEqual.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 77.8% | 26.1% | High PT% |
| lodash_throttle.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 71.0% | 35.4% | High PT% |
| lodash_camelCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.7% |  |
| datefns_subDays.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.0% |  |
| lodash_flow.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 74.2% |  |
| lodash_partition.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.2% |  |
| lodash_kebabCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 45.0% | High PT% |
| lodash_isPlainObject.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 71.0% | 43.2% | High PT% |
| datefns_endOfMonth.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 52.6% | 44.3% | High PT% |
| lodash_pick.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 45.9% | High PT% |
| lodash_invert.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 43.5% | High PT% |
| datefns_isBefore.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 73.5% | 32.1% | High PT% |
| lodash_mapValues.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 40.7% | High PT% |
| datefns_parseISO.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 55.0% | 20.0% | High PT% |
| datefns_format.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.1% | 29.5% | High PT% |
| lodash_set.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 76.9% | 28.2% | High PT% |
| lodash_orderBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 55.4% | 40.0% | High PT% |
| lodash_intersection.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 43.1% | High PT% |
| lodash_curry.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 76.0% | 28.3% | High PT% |
| lodash_flow.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 39.4% | High PT% |
| lodash_union.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.6% |  |
| lodash_uniq.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 39.0% | High PT% |
| lodash_snakeCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 35.2% | High PT% |
| lodash_get.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.2% |  |
| datefns_isBefore.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 52.6% | 45.1% | High PT% |
| lodash_constant.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.0% |  |
| datefns_subDays.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 54.0% | 46.7% | High PT% |
| lodash_mapKeys.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.6% |  |
| lodash_uniqBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 56.9% | 40.3% | High PT% |
| datefns_startOfDay.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 77.8% | 35.9% | High PT% |
| datefns_addMonths.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 89.3% | 25.7% | High PT% |
| lodash_constant.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 79.6% | 24.3% | High PT% |
| lodash_sortBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 82.5% |  |
| lodash_startCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 45.1% | High PT% |
| datefns_format.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.7% | 10.8% | High PT% |
| obfuscated_sample_1.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 90.4% |  |
| lodash_camelCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 38.1% | High PT% |
| lodash_pick.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 36.3% | High PT% |
| lodash_compact.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 57.0% |  |
| lodash_chunk.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 66.0% | 52.8% | High PT% |
| datefns_formatDistance.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 93.1% | 32.6% | High PT% |
| lodash_cloneDeep.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 18.4% | 32.3% |  |
| lodash_snakeCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 42.9% | High PT% |
| lodash_uniqBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 77.8% | 26.2% | High PT% |
| lodash_partial.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.5% | 44.6% | High PT% |
| lodash_keyBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 35.8% | High PT% |
| datefns_format.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 89.5% | 25.1% | High PT% |
| lodash_omit.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 61.2% | High PT% |
| lodash_set.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 58.4% | 38.5% | High PT% |
| lodash_identity.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 58.4% | 37.4% | High PT% |
| datefns_startOfDay.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 54.0% | 45.3% | High PT% |
| lodash_partition.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 46.2% | High PT% |
| lodash_sortBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 55.4% | High PT% |
| lodash_curry.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 66.9% |  |
| lodash_throttle.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.3% | 20.2% | High PT% |
| lodash_curry.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 54.0% | 38.4% | High PT% |
| lodash_mapValues.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.3% | 20.3% | High PT% |
| lodash_kebabCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 37.7% | High PT% |
| lodash_partial.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 60.1% | 48.6% | High PT% |
| lodash_snakeCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 76.6% |  |
| lodash_union.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 81.6% | 39.0% | High PT% |

## Decision Rules Summary

Based on the test results:

- ℹ️ **High passthrough**: 138 file(s) have >20% function-body passthrough. These should be investigated to identify dominant unlowered constructs.


## Blocker Analysis (246 passthrough function(s))

blocker_kind                       occurrences counterfactual +unlocked  cumulative
----------------------------------------------------------------------------------
CompoundAssignmentExpr                    176             62        62          62
ThrowStmt                                  31             21        21          83
ThisExpr                                   11             11        11          94
UnsupportedBinOp                           13              5        13         107
MultipleDeclarators                       117              4       117         224
RegExpLiteral                               5              4         4         228
BreakStmt                                  18              0         0         228
SwitchStmt                                 18              0        18         246
----------------------------------------------------------------------------------
Fixing all listed blockers would unlock 246/246 passthrough function(s).
