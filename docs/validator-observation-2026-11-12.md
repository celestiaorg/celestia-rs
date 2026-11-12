# Validator observation restarted with process provenance — 2026-11-12

With owner authorization to inspect and make necessary changes on both local
validators, new read-only collectors started at **00:29:36 UTC** on candidate
`46acbcb516c3dab26cb5706c908dfe0207f27349`. Both validators remained running;
neither was restarted or redeployed. This advances
[#298](https://github.com/celestiaorg/celestia-rs/issues/298) and
[#299](https://github.com/celestiaorg/celestia-rs/issues/299), without closing
either issue or establishing a completed qualifying window.

## Why collection changed

Both validators were caught up and retained FinalizeBlock results. Loaded
executable inodes matched installed files; embedded versions identified
candidate `46acbcb`, release profile, Rust 1.99.1 / macOS aarch64. Their
SHA-256 was `fdaf29302ce215334663a851ad4b16295da99b03d57bb4779249ebfb04a7c192`.
Local `/abci_info` reported app 10.6.0; independent reference responses
reported Go celestia-app 10.6.0 on mainnet and 10.6.0-mocha on Mocha.
These are dated endpoint assertions, not proof of reference binary bytes.

The old mainnet per-height collectors still hard-coded `b459a6e` despite
subsequent deployments. Their later candidate assertions cannot authenticate
the running process. Both jobs were stopped and disabled, their launch files
archived, and existing receipts preserved. Their timestamps and comparisons
remain evidence inputs with that provenance limitation. Mocha had no matching
continuous per-height collector. No older interval is relabeled as this run.

## New collection and initial evidence

The [observer](validator-observation.md) runs from a fixed installed copy,
one finite user LaunchAgent per network. It verifies PID/home, loaded inode,
embedded candidate and binary bytes, then repeats process checks and SHA-256
measurements. It retains independent tip samples, actual execution-log hashes,
transaction-result comparisons and reference commit participation at each
processed height. The collector source SHA-256 is
`7045e78702bd41772e457236d381b446ae9ce24d67f5af61c53f29c403f5d6d7`.

The [start summary](evidence/validator-observation-2026-11-12/start.json)
records baselines, process provenance, sample counts, snapshot hashes and
bounded coverage. Frozen height receipts are published for
[Mocha](evidence/validator-observation-2026-11-12/mocha-heights.jsonl) and
[mainnet](evidence/validator-observation-2026-11-12/mainnet-heights.jsonl):

| Chain | Initial inclusive range | Evidence |
|---|---|---|
| `mocha-5` | 2,536,888–2,536,931 | 44 matching heights; 43 signed commits and one missed commit |
| `celestia` | 15,809,397–15,809,440 | 44 matching heights; all 44 signed; one additional unavailable attempt at 15,809,400 |

The mainnet reference header request at 15,809,400 timed out. The failed
attempt is retained alongside the later successful retry. The conservative
receipt auditor therefore reports that height as missing/rejected even
though a later comparison matched; no failure is erased. Separate app-hash
and transaction-result audit JSON files accompany each chain's receipts.
The initial Mocha receipt range passes both audits; the initial mainnet
range fails both. No concrete parity mismatch was observed in this snapshot.
Reference status sampling and per-height comparisons remain discrete
observations, not proof of uninterrupted measurement.

## Finite hold and remaining acceptance

The existing updater hold expires at **2026-11-15T01:29:36Z**. Its normal
entry point was checked and reported deployments paused. Watchdog and
read-only health checks remain enabled. Each collector is scheduled for
72 hours plus five minutes; no automatic restart into the same output
directory is configured. Process changes, executable changes, log loss or
parity discrepancies stop observation and retain a terminal event.

The baseline's earliest possible 72-hour completion is approximately
**2026-11-15T00:29:37Z**, contingent on all acceptance evidence. The retained
reference timeout already limits uncontested initial coverage. Elapsed time,
eventual successful retries, and a deployment hold alone cannot establish
qualification. Review complete receipts, failures, exact required ranges,
independent tip/signing observations and the separate Prometheus monitoring
record before deciding whether any interval satisfies #299. F1.8's historical
era coverage in #302 is not supplied by this live observation.
