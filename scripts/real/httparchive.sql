-- HTTP Archive: pull real obfuscated JavaScript response bodies.
--
-- Run in the BigQuery console (https://console.cloud.google.com/bigquery) under
-- your own free-tier project, then export the result to a JSONL file (Save
-- results -> JSONL, or "EXPORT DATA" to a GCS bucket) and feed it to:
--
--   node scripts/real/load-httparchive.mjs --in results.jsonl
--
-- COST WARNING: response_bodies is a very large table. Always keep the
-- date/client filter and a LIMIT so you scan a bounded slice. Adjust the date
-- to a crawl that exists (first of a month). The pre-filter here uses url +
-- regexp on the body so the heavy `report` filtering happens locally, not in SQL.
SELECT
  url,
  response_body AS body
FROM
  `httparchive.crawl.response_bodies`
WHERE
  date = '2024-06-01'            -- pick an existing crawl date (YYYY-MM-01)
  AND client = 'desktop'
  AND url LIKE '%.js'
  AND LENGTH(response_body) BETWEEN 600 AND 524288
  -- obfuscator.io fingerprints: _0x hex identifiers or the rotate-by-shift decoder
  AND (
    REGEXP_CONTAINS(response_body, r'_0x[0-9a-fA-F]{3,}')
    OR REGEXP_CONTAINS(response_body, r"\['push'\]\(\s*\w+\['shift'\]")
    OR REGEXP_CONTAINS(response_body, r'while\s*\(\s*!!\[\]\s*\)')
  )
LIMIT 5000;
