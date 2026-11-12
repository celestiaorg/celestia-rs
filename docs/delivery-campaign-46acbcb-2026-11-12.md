# Exact-candidate delivery campaign — 2026-11-12

Candidate **46acbcb516c3dab26cb5706c908dfe0207f27349** passed the complete
`nightly.sh campaign-0of1` matrix on the Mac Mini: **86 runs, zero skipped,
zero consensus mismatches**. The immutable release harness and pinned
v10.6.0/v9.0.8 Go oracles are the same artifacts used for the
[exact-candidate determinism checks](determinism-46acbcb-2026-11-12.md).

The [qualification receipt](evidence/delivery-campaign-46acbcb-2026-11-12/qualification/evidence.json)
binds the clean source/tree, executable hashes, toolchains and unchanged inputs.
The [full wrapper transcript](evidence/delivery-campaign-46acbcb-2026-11-12/qualification/check.log)
and [summary](evidence/delivery-campaign-46acbcb-2026-11-12/summary.json) retain
all run outcomes. The campaign took 1,227 seconds and produced:

| Measurement | Count |
|---|---:|
| Generated blocks | 25,218 |
| Finalized transactions | 126,358 |
| Finalized transactions with code zero | 49,583 |
| Consensus mismatches | 0 |
| Go/Rust log-only differences | 2,400 |

Every selected v10/v9 scenario ran the wire-mutation/gas-sweep/recheck pass at
seed 20261112 and the governance-pass/recheck pass at seed 20261113. Hyperlane
cases expanded across verifier modes and rejection/registry checks. Additional
governance abstention cases ran at both versions. Twelve v9→v10 upgrade runs
used seeds 2026111201–2026111212 with IBC, signed Hyperlane, wire mutations,
gas sweeps and recheck enabled.

Exactly two current-v10 `upgrade` runs stopped at the expected future-v11
signal: 8 blocks at seed 20261112 and 10 at seed 20261113. All other 84 runs
completed their requested 300 generated blocks. These are intentional upgrade
halts, not omitted runs. Log-only differences are recorded separately and do
not constitute full Go/Rust response parity.

[Compressed per-run records](evidence/delivery-campaign-46acbcb-2026-11-12/runs.jsonl.gz)
retain each environment, exit code, complete `DELIVER_STATS`, final summary and
SHA-256 of raw stdout/stderr/environment/exit files. The raw per-run output is
retained locally under the campaign findings directory; the repository retains
its hashes and structured counters, rather than all 64 MB of raw output.
[Digests](evidence/delivery-campaign-46acbcb-2026-11-12/digests.json) bind every
retained file, including separate Go oracle provenance and build information.
The compressed records can be inspected with Python's `gzip.open` and `json`.

This completes C1.4's fresh bounded campaign for the installed candidate. It
does not complete the multi-night, volume/type saturation, historical replay,
independent review or three-day live observation gates. Later source changes,
including the new proposal cache and genesis follower, need their own candidate
qualification. No live validator deployment was performed for this campaign.
