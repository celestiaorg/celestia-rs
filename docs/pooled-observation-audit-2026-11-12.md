# Checked pooled observation interval — 2026-11-12

The separately started [pooled observation](https://github.com/celestiaorg/celestia-rs/pull/370)
passed the unchanged bounded audit for **05:25:00–05:35:00 UTC** on both
networks. Both validators remained on candidate
`46acbcb516c3dab26cb5706c908dfe0207f27349`, executable SHA-256
`fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192`.

| Network | Required executed heights | Valid samples | Matched heights and result commitments | Signed | Missed |
|---|---|---:|---:|---:|---:|
| Mocha `mocha-5` | 2,543,075–2,543,285 | 116 | 211 | 209 | 2 |
| Mainnet `celestia` | 15,815,626–15,815,836 | 117 | 211 | 208 | 3 |

Both reports retain zero sample failures, gaps, collector interruptions,
missing participation heights or contradictory participation records. App-hash
and transaction-result coverage is complete for the explicitly requested
ranges. All 422 transaction result lists also pass commitment recomputation
against the recorded H+1 roots. Missed signatures remain recorded observations;
this report does not impose or claim a separate missed-block acceptance policy.

The audit still requires a maximum 20-second sample gap, three heights of
absolute reference lag and a 60-second block age. Required heights came from
independent local execution logs with both interval-edge records retained.
The snapshot contains every complete receipt line through its captured prefix,
including failures; successful receipts did not define the requested bounds.
No threshold or failure-accounting rule was relaxed.

[Retained inputs and reports](evidence/pooled-observation-audit-2026-11-12/window.json)
include compressed complete-prefix event/sample/height files, the independent
execution-log edges, [Mocha report](evidence/pooled-observation-audit-2026-11-12/mocha-audit.json),
[mainnet report](evidence/pooled-observation-audit-2026-11-12/mainnet-audit.json),
auditor source hashes and [file digests](evidence/pooled-observation-audit-2026-11-12/digests.json).
Decompress each network's files into a fresh directory to reproduce the audit
with its retained explicit UTC and height bounds. This checks ten minutes;
three-day duration, independent monitoring, reference-signature authentication
and the other acceptance requirements remain separate. Both reports retain
`qualification_established: false`; #299 and #302 remain open.

## Active application metadata and source tags

A separate read-only `abci_info` check at **05:45:13 UTC** found:

| Network | Local reported app version | Reference reported app version | Common observed height |
|---|---|---|---:|
| Mocha | `10.6.0` | `10.6.0-mocha` | 2,543,498 |
| Mainnet | `10.6.0` | `10.6.0` | 15,816,052 |

Each local/reference pair also reported the same app hash at that observed
height. The [raw responses](evidence/pooled-observation-audit-2026-11-12/abci-info.json)
are point observations, not proof of a uniform network rollout or a deployed
Go executable's hash, compiler or build flags.

GitHub's [tag comparison](https://github.com/celestiaorg/celestia-app/compare/v10.6.0...v10.6.0-mocha)
reports `identical`, zero commits and zero changed files: both tags identify
commit [`25f09cab3f8912bbe78c62909e8d39d8fa8b3fc0`](https://github.com/celestiaorg/celestia-app/commit/25f09cab3f8912bbe78c62909e8d39d8fa8b3fc0).
The [retained comparison](evidence/pooled-observation-audit-2026-11-12/app-tag-compare.json)
therefore shows no additional source delta to port solely because of the
Mocha version suffix. Identical source does not establish identical deployed
binaries or binary-dependent Go error formatting. The F1.10/F1.11 checks and
remaining exact-candidate qualification requirements stay open.
