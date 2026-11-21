# Completed collector audit — November 20, 2026

The November 12 pooled collectors completed normally, but their retained
records **do not qualify a continuous three-day interval** for
[#299](https://github.com/celestiaorg/celestia-rs/issues/299). Both offline
observation audits exit 1. C1.1 remains open. No deployment hold, live service
change, restart or signing-secret access was performed for this audit.

The requested interval is **2026-11-12 05:25:00 UTC through November 15
05:25:00 UTC**, exactly 259,200 seconds. It extends the previously retained
[first-hour audit](validator-qualification-closeout-2026-11-13.md) using the
same explicit start time. Required heights come from independent node execution
logs, with the immediately preceding and following lines retained in
[window.json](evidence/validator-observation-audit-2026-11-20/window.json).
Both execution ranges are contiguous.

| Network | Inclusive executed heights | Valid status samples | Invalid/unavailable samples | Sample gaps over 20s |
|---|---|---:|---:|---:|
| Mainnet `celestia` | 15,815,626–15,906,676 | 50,308 | 31 | 2 |
| Mocha `mocha-5` | 2,543,075–2,633,657 | 50,343 | 3 | 1 |

## Candidate, comparisons and participation

Both collectors identify candidate
`46acbcb516c3dab26cb5706c908dfe0207f27349`, release profile, rustc 1.99.1
on macOS ARM64, compatibility target celestia-app v10.6.0 / celestia-core
v0.42.8, executable SHA-256
`fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192`.
The recorded process IDs are mainnet 50523 and Mocha 26124. Every accepted
status sample includes matching process/file identity and a fresh binary-byte
measurement. Both controllers report unchanged tool inputs and child exit 0;
their terminal events are normal deadlines at November 15 05:29:01 UTC.
There is no collector interruption inside the requested interval.

[Mainnet audit](evidence/validator-observation-audit-2026-11-20/mainnet/audit.json)
and [Mocha audit](evidence/validator-observation-audit-2026-11-20/mocha/audit.json)
retain every missing/rejected range and the identity, gap and participation checks.

| Network | Required heights | Uncontested app-hash and result comparisons | Heights excluded by failed attempts | Reference signed / missed |
|---|---:|---:|---:|---:|
| Mainnet | 91,051 | 91,006 | 45 | 90,842 / 209 |
| Mocha | 90,583 | 90,583 | 0 | 90,436 / 147 |

Matching receipts exist at every required height, including mainnet's 45
excluded heights. However, 49 failed attempts at those 45 heights remain
unknown comparisons under the existing conservative rules; later successful
retries do not erase them. The mainnet height audits therefore report
`complete: false`. Mocha's two height audits report `complete: true`.
All uncontested result comparisons recompute the deterministic result
commitment and match reference header H+1. The compared transaction fields
are `code`, `data`, `gas_wanted` and `gas_used`; events and logs are outside
this comparison.

Participation has no missing ranges, conflicting outcomes, nil votes or
out-of-set outcomes. These are reference RPC assertions, not independent
cryptographic verification of signatures. Source configurations and controller
records bind the loopback relays to Pops for mainnet and the Celestia Mocha
RPC for Mocha, as described in the
[installation record](pooled-validator-observation-2026-11-12.md).

## Availability failures and local health

Mainnet's 31 failing status samples comprise 13 HTTP 429 responses, 14 HTTP
502 responses, three timeouts and one reference node catching up. In the last
case, at **November 13 06:59:57.747780 UTC**, the local node reports height
15,847,973 and `catching_up=false`; Pops reports height 15,847,948 and
`catching_up=true`. This is a reference failure, not evidence of local catch-up.
Valid mainnet sample gaps are 22.801858 and 82.019821 seconds. Mocha has one
HTTP 502 and two HTTP 503 samples, plus a 20.386702-second valid-sample gap.
The raw failing samples and failed height attempts are retained beside each
audit. They are not reclassified as healthy time.

The independent [monitoring summary](evidence/validator-observation-audit-2026-11-20/monitoring-summary.json)
and compressed raw Prometheus matrix responses cover the same fixed 72 hours.
Each node target has 17,280 successful scrapes, zero failed scrapes and no
sample spacing over 20 seconds, including interval edges. Each network's
collector series has 17,279 samples, likewise without a gap over 20 seconds.
The collector timestamp's maximum age is 33.029 seconds. Sampled process/RPC
availability stays 1; local catch-up, jailed and tombstoned gauges stay 0.
Maximum sampled block age is 42.467 seconds mainnet and 41.280 seconds Mocha.
These discrete samples cannot repair unavailable independent reference checks.

Periodic app-hash gauges contain unavailable checks (`-1`) on both networks;
none reports a mismatch (`0`). These gauges are repeated scrape observations,
not distinct per-height receipts. Mainnet's `ValidatorMissingBlocks` alert
has 17 firing samples on November 12; alert evaluation is not delivery proof.
An absent firing-alert series is not a missing target scrape.

The mainnet guard-refusal counter stays 0 and Mocha's stays 3. Selected
node-log patterns find no node start/stop, panic, conflicting-data refusal or
app-hash mismatch in the window. Mocha's October 31 refusal at height
2,179,423 remains the documented historical restart/WAL incident; the
[previous audit](observation-window-audit-2026-11-10.md#codesigning-and-historical-guard-refusal)
links its known mechanism and candidate fix. No new unresolved consensus
failure follows from that old refusal.

The [codesigning diagnosis](codesigning-incident-2026-11-05.md) remains a
November 5 deliberate deployment followed by an AMFI failed launch at
23:12:17.433 UTC and a successful spawn at 23:12:47.551 UTC. Its deeper
transient cause is unproven. It is not a Rust panic or a fresh incident in
this window. The original deployment hold expired November 15 at
06:28:57 UTC; this report does not extend it.

## Reproduction and remaining acceptance

Full frozen receipt and execution-log inputs remain machine-local under
`/private/tmp/celestia-299-audit-20261120/`. The
[local input manifest](evidence/validator-observation-audit-2026-11-20/local-inputs.json)
retains their paths, byte counts and SHA-256 values. Git retains reports,
failures, collector events/controller outcomes, interval edges and raw
monitoring responses; it does not retain the full successful receipt ledgers
or execution logs. Reproducing the full audits requires those local inputs.
Digests identify bytes, not authenticity of node or reference assertions.

From the repository root, run the existing auditor against each frozen input:

```sh
python3 scripts/quality/observation_audit.py \
  /private/tmp/celestia-299-audit-20261120/mainnet \
  --chain celestia --candidate 46acbcb516c3dab26cb5706c908dfe0207f27349 \
  --binary-sha256 fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192 \
  --start 2026-11-12T05:25:00Z --end 2026-11-15T05:25:00Z \
  --first 15815626 --last 15906676
# Mocha: directory mocha, chain mocha-5, first 2543075, last 2633657.
# Both commands exit 1; the auditor's ten tests and 13 receipt tests pass.
```

The auditor and receipt module came from checkout `33f3080acdc0eede9aee7e5702a541f462830900`;
their source is unchanged from the earlier closeout checkout. The monitoring
query driver is retained with its raw responses; rerunning it contacts the
local Prometheus service and depends on its retention.
[File digests](evidence/validator-observation-audit-2026-11-20/digests.json)
cover all retained evidence artifacts.

A new qualifying interval requires separate authorization for an exact
candidate and a deployment hold, with durable every-height comparison,
independent tip/participation evidence and monitoring. An interruption or
candidate change starts a new interval; neither these spot checks nor normal
collector deadlines establish acceptance. Historical replay requirements in
[#302](https://github.com/celestiaorg/celestia-rs/issues/302) remain separate.
