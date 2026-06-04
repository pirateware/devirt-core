# Plan P0 — Sequential execution of P1 then P2

The orchestration layer for [P1](P1-whole-program-const-resolution.md)
(whole-program constant resolution) and
[P2](P2-cfg-control-flow-recovery.md) (CFG control-flow recovery). It defines the
**order**, the **gates** between steps, the **verification protocol**, and the
**rollback discipline** so the whole sequence stays sound and every step lands on
a green, committed checkpoint.

**Why strictly sequential:** P2 keys its recovered graph on *concrete state
values*; P1 is what turns symbolic case labels/entries into literals. P2 cannot
begin to fire until P1/M3 lands. P1/M4 (value-axis) is independent and may be
deferred without blocking P2.

---

## 0. Operating principles (non-negotiable, from this repo's invariants)

1. **Soundness is the hard invariant.** Every value-changing step is gated by the
   differential equivalence harness (`tests/equivalence.rs`, executes
   original-vs-deobfuscated in QuickJS). A step that can't be proven sound by a
   *sufficient condition* declines (no transform) — partial coverage is always OK.
2. **Test-first.** Write the adversarial equivalence pair (should-fire vs
   must-NOT-fire) *before* the implementation. The test defines done.
3. **Measure every step.** Run the metrics dashboard (§3) before and after. A step
   that regresses output (bloat, lost folds) is **reverted**, not patched over —
   exactly as the demultiplex widening was (see `issues/02`).
4. **One milestone = one commit = one green checkpoint.** Never stack two
   unverified changes.
5. **The sample suite is not a semantic oracle.** `tests/samples.rs` only proves
   reparse + idempotency. Real semantic safety comes from the equivalence battery +
   sound-by-construction design. Treat any sample-only "pass" with suspicion.

## 1. Baseline & setup (Stage 0 — do this first)

```bash
git checkout main && git pull
cargo test --release --quiet            # confirm 174 green
git checkout -b feat/cfg-recovery       # long-lived integration branch
cp -r out /tmp/p0_baseline_out          # snapshot for parity/regression diffs
./scripts/run-samples.sh                # regenerate out/ + record kept%/stats
```

Capture the **baseline metrics table** (§3) into `plans/metrics-baseline.md`. This
is the scoreboard the whole sequence is measured against.

## 2. The combined milestone ladder

Sequential; each row's gate must be green before the next starts. `↩` = revert
criteria apply (§4). **Fully autonomous — no human gate.** The differential
equivalence harness + adversarial test pairs + idempotency/reparse are the sole
soundness authority (§5).

| # | Stage | From | Definition of done | Gate / metric |
|---|-------|------|--------------------|---------------|
| S0 | Baseline | — | 174 green, metrics captured | suite green |
| S1 | Refactor: `OrderingOracle` + `ConstResolver` skeleton; `global_const_inline` becomes a client | P1/M1 | **Byte-identical** sample output vs baseline | parity diff empty; suite green ↩ |
| S2 | Generalize ordering proof (IIFE/`.call`/multi-init/idempotency-guard + call graph) | P1/M2 | `[gci]` inlined/candidates ratio ↑ on s1/s4/s11 | adversarial pairs green; no equiv regress ↩ |
| S3 | Wire `resolve_int` into `unflatten` + `unflatten_param` | P1/M3 | **Gate G1:** s1/s4/s11 case labels & param entries are numeric | switch count ↓ on s1/s4; suite green ↩ |
| S4 | Value-axis: `EvalValue::Arr` (size-capped) + `var=pureCall()` preamble + global-obj idiom | P1/M4 | sample_7 Base91 sites → strings; **no bloat** (cap holds) | s7 lift measured; suite green ↩ |
| — | **Merge P1 to main** (auto, when green) | — | full suite + parity + equiv + clippy/fmt green | auto-merge §6 |
| S5 | Recovered-CFG builder (read-only): conditional + back edges; debug dump | P2/M1 | unit tests on synthetic branching/looping graphs | graph-shape tests green |
| S6 | Structuring (`src/ir/structure.rs`): Simple/Loop/Multiple → if/while; splice-back | P2/M2 | branch/loop/nested equiv battery green | equiv battery green ↩ |
| S7 | Pipeline integration after P1; gate on "linear `simulate` declined" | P2/M3 | **Gate G2:** s1/s4/s11 `switch`→~0; idempotent | switch≈0; suite green ↩ |
| S8 | Irreducible handling: detect + decline soundly (default) | P2/M4 | irreducible test → pass declines, value matches | equiv + `log()` skip ↩ |
| S9 | Cascade cleanup: confirm resolved guards fold via const-fold/flatten-else/DCE next round | P2/M5 | recovered code reads like source; varNNN ↓ | metrics dashboard ↩ |
| — | **Merge P2 to main** (auto, when green) | — | full suite + equiv + sample assertions + clippy/fmt green | auto-merge §6 |

## 3. Verification protocol — the metrics dashboard

Run before/after every milestone; commit the table into the milestone's commit
message. The numbers *are* the definition of progress.

```bash
# per-sample scoreboard
for n in 1 3 4 5 6 7 8 9 10 11; do
  ./target/release/deob samples/sample_$n.js > /tmp/s$n.js 2>/tmp/s$n.stats
  printf "s%-2s switch=%-3s varNNN=%-6s bytes=%-8s converged=%s\n" "$n" \
    "$(grep -c switch /tmp/s$n.js)" \
    "$(grep -oE 'var[0-9]+' /tmp/s$n.js | sort -u | wc -l)" \
    "$(wc -c </tmp/s$n.js)" \
    "$(grep -oE 'converged=[a-z]+' /tmp/s$n.stats | tail -1)"
done
# soundness + regression nets
cargo test --release --quiet            # equivalence + samples + unit
git --no-pager diff --stat /tmp/p0_baseline_out out   # output drift (parity refactors)
```

**Headline metrics:** `switch` count (control-flow recovery), distinct `varNNN`
(fold depth), `bytes`/`kept%` (no-bloat), `[gci]` ratio (P1 reach), and the
equivalence pass/fail count (soundness).

**Targets:** S3 → s1/s4 switch begins dropping; S7 → s1/s4/s11 switch ≈ 0 and
varNNN collapses; bytes never regress > 2% on any sample without a deliberate,
documented reason.

## 4. Revert criteria (the `↩` rows)

Revert the milestone (don't patch forward) if **any** holds after it lands:

- Any `tests/equivalence.rs` case fails (semantic change) — **hard stop**.
- Any sample fails reparse or idempotency (`tests/samples.rs`).
- An adversarial must-NOT-fire test fires (unsound resolution / recovery).
- Output bytes regress > 2% on any sample with no folding payoff (the demux lesson:
  a transform that fires but bloats is net-negative — revert, document in `issues/`).
- A parity refactor (S1) changes any sample's output.

Record every revert with its measurement in the relevant `issues/` doc, like
`issues/02` — a measured dead-end is a deliverable, not a failure.

## 5. Autonomous soundness authority (no human in the loop)

There are **no human checkpoints**. The soundness-sensitive milestones (the
generalized happens-before proof in S2/S3; the structuring algorithm in S6) are
governed entirely by automated proof obligations that must be green to proceed:

1. **Adversarial test pairs land first.** Before implementing a widening, write its
   must-NOT-fire sibling (use-before-init, conditional init, two-writes,
   irreducible graph, conditional-transition-mis-linearized). The implementation is
   only allowed to make the should-fire case pass *without* making any
   must-NOT-fire case fire.
2. **The differential equivalence harness is the oracle.** `cargo test
   --release --test equivalence` executes original-vs-deobfuscated in QuickJS;
   any value mismatch = unsound = revert. This replaces human judgment on
   correctness.
3. **The sample net.** `tests/samples.rs` (reparse + idempotency) must stay green
   on all 13 samples.
4. **Sound-by-sufficient-condition design.** Every rule only ever *widens what it
   accepts*; it never assumes a fact it can't prove, and declines (no transform)
   otherwise. A decline is always safe.

If all four hold, the milestone is sound by the project's own definition and
proceeds automatically. If any fails, §4 revert applies — autonomously.

## 6. Branch & merge strategy (autonomous)

- One integration branch `feat/cfg-recovery`; one commit per milestone (Sx).
- **Auto-merge to main at the two phase boundaries** (after S4, after S9),
  fast-forward, **only when fully green**: full `cargo test` suite + the equivalence
  battery + sample idempotency + `cargo clippy --all-targets` + `cargo fmt --check`
  all clean. A red gate blocks the merge — never merge red. Mirrors how
  `feat/unflatten-redesign` was landed and pushed.
- Push `main` to `origin` after each green phase merge.
- Keep `plans/` and `issues/` STATUS blocks updated as part of each milestone's
  commit, so the docs never drift from the code.

## 7. Sequencing & parallelism notes

- **Strictly sequential P1 → P2** for the firing path (S1→S3 unblock S5→S7).
- **S4 (value-axis) is independent** of P2 — defer it if CFF recovery is the
  priority; it can land anytime after S1 or even after P2.
- Within a milestone, the read-only analysis (e.g. P2/M1 graph builder) can be
  built and unit-tested before any AST mutation — always split read-only modeling
  from mutation, as `unflatten_param` does (census → build → rewrite).

## 8. Autonomous handling of dead-ends (no halting — keep making progress)

When a milestone can't be made sound or net-positive, **do not stop and wait** —
revert it, document the measured outcome in `issues/`, and route around it:

- A revert criterion fires and the fix isn't obviously sound → revert, write the
  finding to `issues/`, and **skip to the next independent milestone** (e.g. S4 is
  independent of P2; a stuck S2 widening can be narrowed and S3 attempted on what
  *did* resolve).
- A design assumption proves false on real samples (e.g. the recovered CFG is *not*
  reducible on sample_1) → do **not** force goto-soup; keep the sound decline, record
  it, and move on. The irreducible-decline path (S8) is the designed fallback.
- Diminishing returns: if S3 unblocks far fewer dispatchers than expected (blocker
  was prefix statements, not symbolic labels), re-diagnose from the metrics, adjust
  the gate target, and continue — every green checkpoint is still a net win.

The invariant: the branch is **always green and committed**; progress is monotonic;
nothing is ever left half-applied or red.

## 9. Definition of "done" for the whole sequence

- Samples 1, 4, 11: control-flow dispatchers recovered to structured
  `if/else/while/for`; `switch` ≈ 0; `varNNN` collapsed; output reads like source.
- Samples 5, 6: CFF `split("|")`/`switch` dispatchers recovered (where not VM).
- sample_7: Base91 string table lifted (if S4 done).
- Full suite green; clippy/fmt clean; both plans' STATUS blocks updated;
  metrics-baseline → metrics-final delta recorded.
- Out of scope and explicitly *not* done: bytecode VMs (8/9/10 — Phase 3), Closure
  minified property names (sample_10 floor), host-coupled runtime (sample_7 sensor).
```
