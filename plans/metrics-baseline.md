# P0 baseline metrics (Stage 0)

Captured on branch `feat/cfg-recovery` at commit `162b936`, suite = **174 green**.

## Per-sample scoreboard

| sample | switch | distinct varNNN | bytes | converged |
|--------|--------|-----------------|-------|-----------|
| s1  | 38 | 1569  | 894310  | true |
| s3  | 6  | 740   | 103040  | true |
| s4  | 5  | 948   | 130101  | true |
| s5  | 4  | 267   | 24133   | true |
| s6  | 5  | 296   | 24849   | true |
| s7  | 26 | 4607  | 636549  | true |
| s8  | 40 | 2794  | 250183  | true |
| s9  | 0  | 606   | 157990  | true |
| s10 | 86 | 16055 | 1675367 | true |
| s11 | 37 | 1112  | 195461  | true |

## `[gci]` ratio (final round) — P1 reach baseline

- s1:  inlined=0 of candidates=1083 (final-round; peaks 820/1903 mid-pipeline)
- s4:  inlined=0 of candidates=422 (final-round; peaks 385/807 mid-pipeline)
- s11: inlined=0 of candidates=36

The mid-pipeline gci fires (s1 820, s4 385) but the *case-label tables* of the
dispatchers stay symbolic at the round the unflatteners run — P1 targets exactly
this. s11 never inlines (init shapes the narrow 4-condition core rejects).

## Targets (from P0 §3)

- S3 → s1/s4 switch begins dropping.
- S7 → s1/s4/s11 switch ≈ 0; varNNN collapses.
- bytes never regress > 2% on any sample without a documented reason.
