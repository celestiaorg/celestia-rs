# Validator qualification work closeout — 2026-11-13

The timeboxed work produced merged PRs #350–#372. This record retains the
remaining completed results and distinguishes unfinished shell-managed work.
It does not establish overall production qualification or satisfy the pending
three-day and historical-traffic requirements.

## Completed signed upgrade campaign

Installed candidate `46acbcb516c3dab26cb5706c908dfe0207f27349` passed 50
independent signed devnet-shaped Go v9.0.8 → v10.6.0 upgrades: 25 message-ID
and 25 Merkle-root ISM cases, seeds 1–50, 240 generated blocks each. Every
case required both signed v9 incoming paths, one actual Go binary switch,
and successful finalized v10 phase traffic while Rust continued in one process.

| Result | Count |
|---|---:|
| Generated blocks | 12,000 |
| Finalized transactions | 73,587 |
| Go code-zero transactions | 52,633 |
| Consensus mismatches | 0 |
| Log-only differences | 1,422 |
| Runs with both signed v10 incoming paths | 45 of 50 |

[Qualification evidence](evidence/validator-qualification-closeout-2026-11-13/upgrades/qualification/evidence.json)
records clean, unchanged source and unchanged Rust/Go/driver input hashes.
All retained per-run raw artifact hashes were checked again at closeout.
[Compressed run records](evidence/validator-qualification-closeout-2026-11-13/upgrades/runs.json.gz)
retain complete statistics, phase counts, switch heights and environments;
the executed driver is included. This supplies the bounded 50-run evidence,
with mainnet-shaped state, coverage and the other acceptance gates still open.
The Go executables used their distinct pinned compilers: v9 Go 1.26.9 and
v10 Go 1.26.10. Ambient wrapper tool versions do not identify those compilers.

## Completed branch-instrumented coverage

The same candidate source completed the full coverage driver with nightly
`2026-11-02` (rustc 1.101.0-nightly, LLVM 23.1.5) and cargo-llvm-cov 0.9.1.
Its report measured **5,669 / 13,576 branch arms in handwritten tier source**.
The report's scope includes inline tests; exported regions and LLVM summary
counters have different denominators. These counts are not a percentage of
all consensus behavior or proof that every error path was reached.

The [qualification receipt](evidence/validator-qualification-closeout-2026-11-13/coverage/qualification/evidence.json),
complete run transcript, summary and [compressed branch inventory](evidence/validator-qualification-closeout-2026-11-13/coverage/branches.json.gz)
are retained. Uncovered arms still need review and reasons. Instrumentation
used a separate compiler/profile and did not replace either live executable.
The newer macOS-compatible controller came from signed source `449ffa2`;
its sole local adaptation selected the unchanged candidate source root.
A separate pre/post controller identity check is retained, while the original
qualifier receipt remains unchanged. Full LLVM JSON, LCOV, merged profile and
instrumented binary remain local with [paths, sizes and hashes](evidence/validator-qualification-closeout-2026-11-13/coverage/raw-artifacts.json).

## First pooled hour and ongoing observation

The strict **2026-11-12 05:25–06:25 UTC** audit passed on both networks:

| Network | Inclusive heights | Valid samples | Matched result commitments | Signed | Missed |
|---|---|---:|---:|---:|---:|
| Mocha | 2,543,075–2,544,330 | 700 | 1,256 | 1,248 | 8 |
| Mainnet | 15,815,626–15,816,886 | 701 | 1,261 | 1,254 | 7 |

Both first-hour reports have no sample failure, gap, collector interruption
or missing participation range. Required heights came from independent
execution logs with both interval edges retained. This is a bounded receipt
and sample audit; separate monitoring and the full acceptance duration are
not certified by it. Reports and frozen compressed inputs are retained under
[first-hour evidence](evidence/validator-qualification-closeout-2026-11-13/first-hour/window.json).

At the **November 13, 23:43 UTC** spot check, both validators were caught up.
A separate closeout health query also reported neither jailed nor tombstoned.
The durable collectors continue writing. Their aggregate records now contain
three unavailable Mocha status samples, 27 unavailable mainnet status samples
and 32 unavailable mainnet height attempts. Those failures remain explicit
unknowns under the strict rules, even where retries later matched. Elapsed
time alone cannot qualify this window. The [current snapshot](evidence/validator-qualification-closeout-2026-11-13/current-observation.json)
records the counts and deployment hold, which expires November 15 at
06:28:57 UTC. No node restart or replacement was performed by this work.

## Unfinished work remains unqualified

The historical Mocha ledger ends at a retained height record **112,904**,
timestamped November 12 at 06:57:27 UTC, without a terminal receipt. The
recorded replay child/relay PIDs were absent at closeout, and the original
execution handles were missing. The newer-candidate full campaign and two
consensus fuzz jobs likewise lack final receipts. Their precise interruption
cause and time are not established.

Their original manifests still say `running`; they were preserved unchanged.
The separate [incomplete-check inventory](evidence/validator-qualification-closeout-2026-11-13/incomplete-checks.json)
records the verified missing handles and the absence of completed evidence.
These runs provide no full-era replay, completed-matrix or consensus-volume
qualification credit. Frozen candidate checkouts and local raw artifacts
remain available for source/profile inspection and a future explicit run.

The public archive's copied snapshot catalog was inspected read-only during
closeout. Its two retained records are mainnet heights **15,556,500** and
**15,558,000**, format 3, 69 chunks each. Both are after the required v9 era;
neither supplies the missing near-activation checkpoint. The partial tar
inventory also encountered older chunk directories, so the catalog does not
prove those directories absent or unusable. No older donor was restored or
qualified. [Catalog output, range provenance and the exact reader source](evidence/validator-qualification-closeout-2026-11-13/snapshot-catalog/catalog.jsonl)
are retained; application-state and signing files were not extracted.

## Tracking and signing correction

GitHub interpreted the phrase “does not close #302” in PR #361 as a closing
reference despite its negation. The PR's broader replay acceptance remained
unmet. The description was corrected and #302 reopened. Its open state now
matches the evidence; #299 also remains open.

A closeout [signature audit](evidence/validator-qualification-closeout-2026-11-13/signature-audit.json)
verified 31 original PR commits and the additional session-created merge
locally and through GitHub's verification fields. This report's own commit is
verified separately before and after pushing. [File digests](evidence/validator-qualification-closeout-2026-11-13/digests.json)
identify every retained artifact; they do not authenticate external RPC claims.
