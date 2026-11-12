# Installed-candidate delivery volume — 2026-11-12

Candidate **46acbcb516c3dab26cb5706c908dfe0207f27349** reached **1,620,567
finalized transactions across 825 complete clean runs**, with zero consensus
mismatches. This satisfies F1.2's numerical volume threshold on this candidate.
The three consecutive clean nights, script/type/branch coverage and other
qualification requirements remain open.

The total combines the [earlier full campaign](delivery-campaign-46acbcb-2026-11-12.md)
with a fresh two-hour `nightly.sh deliver-0of1` rotation:

| Evidence | Complete block runs | Counted finalized transactions |
|---|---:|---:|
| Earlier full campaign, seed 20261112 | 84 | 126,276 |
| Two-hour rotation, seed 20261113 | 741 | 1,494,291 |
| Combined, deduplicated | 825 | 1,620,567 |

The fresh rotation finished 768 runs in 7,061 seconds, all exit code zero,
with 16 budget skips. It generated 222,799 blocks and finalized 1,500,494
transactions overall. Its 27 intentionally short v10 future-upgrade runs
contributed 6,203 transactions excluded from the volume count. Together with
the earlier campaign's two short runs, the combined report excludes 29
incomplete block runs. The skipped cases remain recorded in the transcript;
the earlier complete matrix supplies the separately documented scenario pass.

The fresh rotation includes 741,716 Go/Rust log-only differences. Those remain
separate from the zero consensus mismatch count. It also records 239 runs
where scripted inputs were dropped, reordered or replaced during proposal
mutation. The volume count includes only transactions that actually reached
FinalizeBlock; it does not claim every scripted transaction executed. Required
coverage gates still controlled process success, and the retained structured
records preserve each run's script state, type/error counters and flags.
Coverage and script reachability therefore require their own review.

The [execution receipt](evidence/delivery-volume-46acbcb-2026-11-12/qualification/evidence.json)
binds the clean candidate tree and unchanged release harness and pinned Go
app v10.6.0/v9.0.8 executable hashes. The release Rust harness is the same
`0ba760fd0d6b857b9b06beb0aedd664f7943c59779e02e7d9aa3084b7a919f07`
artifact used for the earlier campaign. Both validators continued on the same
candidate while these isolated checks ran.

The [combined saturation report](evidence/delivery-volume-46acbcb-2026-11-12/combined-report/check.log)
was generated separately from clean report-tool revision
`aa97ac3b8f8de06ef5b4fe5a63ee5d931f6ddc74`, which reads the correct distinct
Go compiler pins. Its [receipt](evidence/delivery-volume-46acbcb-2026-11-12/combined-report/evidence.json)
identifies that reporting tool, rather than changing the execution candidate.
It rejects dirty, failed, mismatched, incomplete-block and conflicting duplicate
runs, and deduplicates scenario/seed/size/flag identities per candidate.

[Compressed per-run records](evidence/delivery-volume-46acbcb-2026-11-12/runs.jsonl.gz)
retain all `DELIVER_STATS`, environments, exits, log-difference totals and
hashes of raw output files. The raw local artifacts remain under
`/private/tmp/celestia-rc-46acbcb/deliver-2h-20261113/`.
[File digests](evidence/delivery-volume-46acbcb-2026-11-12/digests.json)
bind the repository evidence and [summary](evidence/delivery-volume-46acbcb-2026-11-12/summary.json).
Counts do not transfer to later source revisions. F1.2 remains unchecked until
its separate nightly requirement is satisfied; this report does not close
#299, #302 or the overall qualification gate.
