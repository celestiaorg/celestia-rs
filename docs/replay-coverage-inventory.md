# F1.8 replay coverage inventory — 2026-11-05

This inventory separates documented historical runs from artifacts retained
in this repository. It does not certify a replay of the current candidate.
Height ranges below are inclusive. A gap means there is no auditable
exact-candidate receipt identified here, not that a block was never executed.

| Chain | Documented run | Source snapshot | Recorded comparison | Retained run artifact and candidate identity |
|---|---|---|---|---|
| `mocha-5` | 1,624,501–at least 1,711,100, app v10 | Go SDK snapshot at 1,624,500 inside Polkachu's `celestia_1702915.tar.lz4` | App hashes, transaction results and validator sets matched, per [completed milestones](milestones-completed.md) | No per-height receipt or exact Rust commit identified in the repository; Go version for this run is not recorded there |
| `celestia` | 14,894,001–at least 14,914,200, app v9 | Go SDK snapshot from a Polkachu pruned tarball; its exact archive and snapshot height are not recorded in the completed milestone | App hashes, transaction results and validator sets matched, per [completed milestones](milestones-completed.md) | No per-height receipt or exact Rust commit identified in the repository; the milestone identifies the v9 reference as celestia-app v9.0.8 |
| `celestia` | 15,105,001–15,137,961, app v9 to v10 at 15,116,658 | Go SDK snapshot at 15,105,000 inside Polkachu's `celestia_15123013` tarball; restored hash matched mainnet | Every app hash, transaction result and validator set matched, 32,961 heights and 151,765 transactions, per [M13](../MILESTONES.md) | Historical cloud x86_64 release run dated 2026-10-20; no per-height receipt or exact Rust commit identified in the repository. The Go v9/v10 executable patch versions for this run are not recorded in M13 |
| `mocha-5` and `celestia` | Selected live heights on 2026-11-05 | Running Mac validator state, not a replay snapshot | Periodic app-hash checks only; [dated evidence](live-validator-evidence-2026-11-05.md) lists heights and hashes | Running Rust commit `b459a6e041b2675582019cde5c75ecf2969318ba` was observed; no every-height or transaction-result comparison receipt |

The repository also retains a [40-block synthetic v9-to-v10
devnet fixture](../conformance/fixtures/devnet/v9-upgrade-replay/meta.json)
with a Go build record. It tests the upgrade implementation but is not
historical Mocha or mainnet traffic and does not close F1.8.

## Required coverage still without exact-candidate receipts

- **Mainnet v9 era:** The v9 activation height is not established in the
  cited run records. There is no identified receipt from that activation
  through 14,894,000. Between the two documented mainnet replays,
  14,914,201–15,105,000 has no identified receipt. The first run may have
  continued beyond 14,914,200, but its final inclusive height is unknown.
- **Mocha v9 and upgrade:** Mocha began at app v9 and upgraded at
  1,082,620. No historical Mocha replay receipt is identified for the v9
  era, the upgrade boundary, or 1,082,621–1,624,500. The precise v9/v10
  execution boundary must be checked against a trusted header record.
- **Mainnet live P2P record:** The Mac's historical parity samples do not
  establish a continuous record from M13 to M14 on one candidate. Issue
  [#299](https://github.com/celestiaorg/celestia-rs/issues/299) tracks the
  exact-candidate observation; [#298](https://github.com/celestiaorg/celestia-rs/issues/298)
  tracks retained parity and transaction-result evidence.

Before treating any earlier replay as exact-candidate evidence, retain its
source snapshot archive hash and origin, chain ID, inclusive first/last
executed heights, Rust and Go executable versions and hashes, per-height
app-hash and transaction-result comparison receipts, failures and gaps.
Historical claims above remain useful context but do not replace that
record. F1.8 remains open.
