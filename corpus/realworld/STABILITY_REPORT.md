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
| datefns_addMonths.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 18.8% | High PT% |
| lodash_throttle.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 26.5% | High PT% |
| lodash_uniqBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.4% |  |
| lodash_flatten.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 53.5% |  |
| lodash_snakeCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_identity.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 46.4% |  |
| lodash_sortBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_cloneDeep.js | ✓ | ✓ | ✓ | ✓ | ✓ | 74.7% | 29.9% | High PT% |
| datefns_isBefore.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 64.7% | High PT% |
| datefns_format.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.8% | 19.2% | High PT% |
| lodash_without.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_mapValues.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 53.0% |  |
| lodash_set.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 65.5% |  |
| lodash_orderBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 30.4% |  |
| lodash_times.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 42.9% | High PT% |
| lodash_groupBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_formatDistance.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 21.2% | High PT% |
| lodash_chunk.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 31.7% | High PT% |
| lodash_difference.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_union.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_debounce.js | ✓ | ✓ | ✓ | ✓ | ✓ | 87.0% | 15.7% | High PT% |
| lodash_camelCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_compact.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 9.3% | High PT% |
| datefns_parseISO.js | ✓ | ✓ | ✓ | ✓ | ✓ | 59.5% | 14.9% | High PT% |
| lodash_flow.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_isAfter.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 64.6% | High PT% |
| lodash_startCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_escape.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.1% |  |
| lodash_noop.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_keyBy.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_merge.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partial.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_subDays.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 76.0% | High PT% |
| lodash_invert.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_isEqual.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.7% |  |
| lodash_capitalize.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.1% |  |
| lodash_kebabCase.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partition.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_has.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.8% |  |
| lodash_get.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 48.7% |  |
| lodash_intersection.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_range.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_endOfMonth.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 62.6% | High PT% |
| lodash_uniq.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 66.7% |  |
| lodash_curry.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 48.3% |  |
| lodash_omit.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_getDay.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 74.3% | High PT% |
| lodash_zip.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_addDays.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 55.0% | High PT% |
| datefns_startOfDay.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 74.0% | High PT% |
| lodash_memoize.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 19.6% | High PT% |
| lodash_mapKeys.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 52.7% |  |
| lodash_constant.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 30.2% |  |
| datefns_differenceInDays.js | ✓ | ✓ | ✓ | ✓ | ✓ | 79.2% | 34.5% | High PT% |
| lodash_pick.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_isPlainObject.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 53.4% | High PT% |

## Tier 2

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| datefns_addMonths.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 59.5% | High PT% |
| lodash_isPlainObject.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 59.6% | High PT% |
| chart_js.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_memoize.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 29.5% | High PT% |
| datefns_addDays.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 73.4% | High PT% |
| axios.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| sample_bundle_2.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_compact.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 21.5% | High PT% |
| lodash_mapKeys.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 60.3% | High PT% |
| d3.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_orderBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 45.9% | High PT% |
| lodash_constant.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 28.6% |  |
| lodash_zip.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| ramda.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_throttle.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 38.4% | High PT% |
| mobx.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| moment.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_get.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 46.9% |  |
| lodash_camelCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_differenceInDays.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 79.6% | 59.3% | High PT% |
| core_js_bundle.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_groupBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| redux.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_snakeCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| rxjs.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_set.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 63.9% |  |
| datefns_parseISO.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.9% | 21.9% | High PT% |
| lodash_curry.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 44.6% | High PT% |
| lodash_capitalize.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 76.2% |  |
| react.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_pick.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_escape.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 65.7% | High PT% |
| lodash_omit.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_keyBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| zod.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| marked.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partition.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_partial.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_isAfter.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 74.7% | High PT% |
| datefns_isBefore.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 74.7% | High PT% |
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
| datefns_getDay.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 83.0% | High PT% |
| tslib.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_invert.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| preact.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| three.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_noop.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| sample_bundle_1.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_merge.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_range.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| luxon.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_startOfDay.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 79.4% | High PT% |
| lodash_sortBy.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_times.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 53.0% | High PT% |
| datefns_formatDistance.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 36.4% | High PT% |
| lodash_chunk.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 47.7% | High PT% |
| lodash_intersection.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_isEqual.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 68.4% |  |
| datefns_format.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.4% | 34.2% | High PT% |
| vue.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_kebabCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_endOfMonth.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 70.1% | High PT% |
| lodash_mapValues.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 60.6% | High PT% |
| jquery.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_has.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 73.5% |  |
| immer.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| datefns_subDays.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 100.0% | 81.7% | High PT% |
| lodash_startCase.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_cloneDeep.terser.min.js | ✓ | ✓ | ✓ | ✓ | ✓ | 62.5% | 37.3% | High PT% |

## Tier 3

| File | Parse | Validate | Emit | Re-parse | Roundtrip | FB-PT% | TL-PT% | Notes |
|------|-------|----------|------|----------|-----------|--------|--------|-------|
| lodash_without.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 36.5% | High PT% |
| lodash_identity.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 94.8% | 23.7% | High PT% |
| lodash_merge.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 31.8% | High PT% |
| lodash_mapKeys.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.6% | 34.4% | High PT% |
| lodash_escape.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 43.0% | 64.6% | High PT% |
| lodash_constant.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 86.0% | 32.4% | High PT% |
| datefns_endOfMonth.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.9% | 26.6% | High PT% |
| lodash_difference.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 80.9% |  |
| lodash_zip.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.1% | 40.6% | High PT% |
| lodash_identity.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.7% |  |
| datefns_addMonths.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 70.3% | 56.4% | High PT% |
| datefns_isBefore.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 47.7% | 67.7% | High PT% |
| lodash_range.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 36.7% | High PT% |
| lodash_cloneDeep.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 78.7% | 30.8% | High PT% |
| datefns_addMonths.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 93.4% | 37.0% | High PT% |
| lodash_isEqual.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.0% | 36.0% | High PT% |
| lodash_partition.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 28.2% | High PT% |
| lodash_invert.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 47.8% | High PT% |
| datefns_addDays.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.9% | 27.4% | High PT% |
| lodash_flow.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 24.5% | High PT% |
| lodash_debounce.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 74.3% | 29.3% | High PT% |
| lodash_without.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 42.1% | High PT% |
| lodash_debounce.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 84.6% | 24.1% | High PT% |
| datefns_addDays.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 92.3% | 40.9% | High PT% |
| lodash_pick.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.2% |  |
| lodash_merge.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 40.6% | High PT% |
| lodash_intersection.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 46.4% | High PT% |
| lodash_escape.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.3% | 23.2% | High PT% |
| lodash_groupBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 81.3% |  |
| lodash_omit.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 56.1% | High PT% |
| datefns_formatDistance.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 99.3% | 10.5% | High PT% |
| lodash_noop.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.5% |  |
| lodash_orderBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.1% | 19.8% | High PT% |
| lodash_compact.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.8% | 32.2% | High PT% |
| lodash_cloneDeep.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 99.1% | 11.9% | High PT% |
| lodash_keyBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.7% |  |
| lodash_zip.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 75.4% |  |
| lodash_partial.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 81.4% |  |
| lodash_capitalize.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.6% |  |
| lodash_times.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 58.5% | 58.0% | High PT% |
| datefns_getDay.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 42.4% | 70.7% | High PT% |
| lodash_chunk.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 92.7% | 34.7% | High PT% |
| lodash_startCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.1% |  |
| lodash_without.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.0% |  |
| lodash_flatten.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 86.0% | 35.0% | High PT% |
| lodash_omit.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 86.8% |  |
| lodash_memoize.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 73.3% | 47.7% | High PT% |
| lodash_get.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 86.0% | 35.7% | High PT% |
| lodash_difference.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 36.2% | High PT% |
| datefns_formatDistance.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.4% | 24.3% | High PT% |
| lodash_noop.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 23.0% | High PT% |
| datefns_getDay.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.1% | 30.1% | High PT% |
| lodash_chunk.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.6% | 17.6% | High PT% |
| lodash_groupBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 37.0% | High PT% |
| datefns_addDays.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 57.1% | 65.6% | High PT% |
| lodash_sortBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 46.7% | High PT% |
| datefns_differenceInDays.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 94.7% | 31.0% | High PT% |
| datefns_parseISO.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 89.3% | 21.8% | High PT% |
| lodash_union.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.1% | 43.3% | High PT% |
| lodash_set.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.3% |  |
| lodash_compact.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.5% | 19.8% | High PT% |
| lodash_range.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 24.7% | High PT% |
| lodash_has.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 73.2% |  |
| lodash_intersection.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 79.7% |  |
| lodash_isPlainObject.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 62.7% | 62.3% | High PT% |
| lodash_difference.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 45.6% | High PT% |
| datefns_subDays.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.1% | 33.0% | High PT% |
| lodash_debounce.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 99.5% | 6.5% | High PT% |
| lodash_uniq.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.5% |  |
| lodash_capitalize.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.3% | 24.2% | High PT% |
| lodash_groupBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 47.5% | High PT% |
| obfuscated_sample_2.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 100.0% |  |
| lodash_uniq.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.1% | 24.4% | High PT% |
| lodash_orderBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 50.5% | 61.0% | High PT% |
| datefns_endOfMonth.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 55.2% | 64.1% | High PT% |
| lodash_startCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 34.3% | High PT% |
| lodash_memoize.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 93.6% | 28.6% | High PT% |
| lodash_capitalize.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 85.9% | 36.8% | High PT% |
| lodash_keyBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 40.6% | High PT% |
| lodash_mapKeys.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.1% | 21.9% | High PT% |
| lodash_kebabCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.9% |  |
| datefns_isAfter.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.8% | 40.4% | High PT% |
| datefns_parseISO.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 99.6% | 6.5% | High PT% |
| datefns_differenceInDays.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 98.3% | 18.8% | High PT% |
| lodash_has.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.3% | 20.9% | High PT% |
| lodash_times.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 92.3% | 34.5% | High PT% |
| lodash_zip.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 27.6% | High PT% |
| lodash_get.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.5% | 21.0% | High PT% |
| lodash_times.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.3% | 19.5% | High PT% |
| datefns_isAfter.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.4% | 30.8% | High PT% |
| lodash_camelCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 43.9% | High PT% |
| lodash_isPlainObject.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.5% | 22.8% | High PT% |
| datefns_isAfter.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 49.7% | 67.4% | High PT% |
| lodash_isEqual.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.5% |  |
| lodash_merge.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.3% |  |
| datefns_startOfDay.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 43.4% | 69.1% | High PT% |
| lodash_mapValues.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 47.1% | 62.8% | High PT% |
| lodash_throttle.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 61.7% | 54.5% | High PT% |
| lodash_uniqBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 70.5% |  |
| lodash_has.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.0% | 39.6% | High PT% |
| lodash_escape.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.5% | 36.4% | High PT% |
| datefns_getDay.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.5% | 41.8% | High PT% |
| lodash_invert.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 82.4% |  |
| lodash_flatten.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.3% | 24.7% | High PT% |
| lodash_memoize.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.6% | 14.9% | High PT% |
| lodash_range.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 72.7% |  |
| lodash_flatten.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.1% |  |
| datefns_differenceInDays.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 80.9% | 43.5% | High PT% |
| lodash_noop.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 34.5% | High PT% |
| lodash_isEqual.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.1% | 22.7% | High PT% |
| lodash_throttle.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 92.6% | 32.9% | High PT% |
| lodash_camelCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.7% |  |
| datefns_subDays.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 42.9% | 72.0% | High PT% |
| lodash_flow.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 74.2% |  |
| lodash_partition.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 78.2% |  |
| lodash_kebabCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 41.3% | High PT% |
| lodash_isPlainObject.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 92.6% | 40.5% | High PT% |
| datefns_endOfMonth.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 92.1% | 38.6% | High PT% |
| lodash_pick.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 42.2% | High PT% |
| lodash_invert.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 39.7% | High PT% |
| datefns_isBefore.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.5% | 27.5% | High PT% |
| lodash_mapValues.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.6% | 34.7% | High PT% |
| datefns_parseISO.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 93.2% | 17.7% | High PT% |
| datefns_format.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 94.6% | 29.4% | High PT% |
| lodash_set.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.3% | 23.7% | High PT% |
| lodash_orderBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.8% | 34.2% | High PT% |
| lodash_intersection.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 39.3% | High PT% |
| lodash_curry.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.5% | 23.2% | High PT% |
| lodash_flow.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.1% | 36.1% | High PT% |
| lodash_union.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 77.6% |  |
| lodash_uniq.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 86.0% | 35.7% | High PT% |
| lodash_snakeCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 31.7% | High PT% |
| lodash_get.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.2% |  |
| datefns_isBefore.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.8% | 40.4% | High PT% |
| lodash_constant.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 69.0% |  |
| datefns_subDays.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.5% | 42.2% | High PT% |
| lodash_mapKeys.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 47.1% | 63.7% | High PT% |
| lodash_uniqBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 85.9% | 36.9% | High PT% |
| datefns_startOfDay.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.1% | 31.7% | High PT% |
| datefns_addMonths.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.4% | 24.0% | High PT% |
| lodash_constant.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 93.8% | 21.5% | High PT% |
| lodash_sortBy.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 82.5% |  |
| lodash_startCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 41.5% | High PT% |
| datefns_format.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 99.4% | 10.6% | High PT% |
| obfuscated_sample_1.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 90.4% |  |
| lodash_camelCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 34.5% | High PT% |
| lodash_pick.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 32.7% | High PT% |
| lodash_compact.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 51.2% | 57.9% | High PT% |
| lodash_chunk.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 66.0% | 52.8% | High PT% |
| datefns_formatDistance.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 93.1% | 32.6% | High PT% |
| lodash_cloneDeep.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 84.1% | 27.2% | High PT% |
| lodash_snakeCase.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 39.3% | High PT% |
| lodash_uniqBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 96.3% | 22.1% | High PT% |
| lodash_partial.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 40.8% | High PT% |
| lodash_keyBy.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 32.3% | High PT% |
| datefns_format.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.9% | 24.5% | High PT% |
| lodash_omit.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 57.4% | High PT% |
| lodash_set.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.0% | 35.2% | High PT% |
| lodash_identity.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 88.0% | 34.1% | High PT% |
| datefns_startOfDay.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.5% | 40.8% | High PT% |
| lodash_partition.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 42.5% | High PT% |
| lodash_sortBy.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 51.7% | High PT% |
| lodash_curry.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 50.5% | 60.0% | High PT% |
| lodash_throttle.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.2% | 18.6% | High PT% |
| lodash_curry.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 91.8% | 33.2% | High PT% |
| lodash_mapValues.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 97.2% | 18.7% | High PT% |
| lodash_kebabCase.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 34.1% | High PT% |
| lodash_partial.medium.js | ✓ | ✓ | ✓ | ✓ | ✓ | 90.2% | 44.9% | High PT% |
| lodash_snakeCase.low.js | ✓ | ✓ | ✓ | ✓ | ✓ | 0.0% | 76.6% |  |
| lodash_union.high.js | ✓ | ✓ | ✓ | ✓ | ✓ | 95.8% | 35.3% | High PT% |

## Decision Rules Summary

Based on the test results:

- ℹ️ **High passthrough**: 182 file(s) have >20% function-body passthrough. These should be investigated to identify dominant unlowered constructs.


## Blocker Analysis (363 passthrough function(s))

blocker_kind                       occurrences counterfactual +unlocked  cumulative
----------------------------------------------------------------------------------
MultipleDeclarators                       304             60        60          60
SequenceExpr                              247             22        52         112
ThisExpr                                   11              5        10         122
ForInStmt                                  10              4        10         132
UnsupportedBinOp                            6              4         4         136
UnsupportedExpr                           214              2       176         312
RegExpLiteral                               5              1         4         316
ThrowStmt                                  31              1        29         345
BreakStmt                                  18              0         0         345
SwitchStmt                                 18              0        18         363
----------------------------------------------------------------------------------
Fixing all listed blockers would unlock 363/363 passthrough function(s).
