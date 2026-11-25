# F1.8 replay coverage inventory — updated 2026-11-24

[Pinned replay results](evidence/replay-43ba527/README.md) cover 1,000 blocks
each on Mocha v9, mainnet v10 and Mocha v11, plus 1,000 synthetic v12 blocks
on the November 24 candidate `43ba527cce4425c2aea63f1165eb5d3c086f78d3`.
The historical mainnet v9 era and Mocha heights 1,001–1,082,620 still need
execution receipts on that candidate. All ranges are inclusive.

## Earlier runs

| Chain | Documented run | Source snapshot | Recorded comparison | Retained run artifact and candidate identity |
|---|---|---|---|---|
| `mocha-5` | 1,624,501–at least 1,711,100, app v10 | Go SDK snapshot at 1,624,500 inside Polkachu's `celestia_1702915.tar.lz4` | App hashes, transaction results and validator sets matched, per [completed milestones](milestones-completed.md) | No per-height receipt or exact Rust commit identified in the repository; Go version for this run is not recorded there |
| `celestia` | 14,894,001–at least 14,914,200, app v9 | Go SDK snapshot from a Polkachu pruned tarball; its exact archive and snapshot height are not recorded in the completed milestone | App hashes, transaction results and validator sets matched, per [completed milestones](milestones-completed.md) | No per-height receipt or exact Rust commit identified in the repository; the milestone identifies the v9 reference as celestia-app v9.0.8 |
| `celestia` | 15,105,001–15,137,961, app v9 to v10 at 15,116,658 | Go SDK snapshot at 15,105,000 inside Polkachu's `celestia_15123013` tarball; restored hash matched mainnet | Every app hash, transaction result and validator set matched, 32,961 heights and 151,765 transactions, per [M13](../MILESTONES.md) | Historical cloud x86_64 release run dated 2026-10-20; no per-height receipt or exact Rust commit identified in the repository. The Go v9/v10 executable patch versions for this run are not recorded in M13 |
| `mocha-5` and `celestia` | Selected live heights on 2026-11-05 | Running Mac validator state, not a replay snapshot | Periodic app-hash checks only; [dated evidence](live-validator-evidence-2026-11-05.md) lists heights and hashes | Running Rust commit `b459a6e041b2675582019cde5c75ecf2969318ba` was observed; no every-height or transaction-result comparison receipt |
| `celestia` | 15,530,001–15,530,101, app v10 | SDK snapshot 15,530,000 from Polkachu's `celestia_15609249.tar.lz4`; outer archive streamed without a retained digest | 101 per-height transaction-result comparisons, 454 transactions; final app hash matched public header 15,530,102 | [Retained JSONL and provenance](tx-result-isolated-replay-2026-11-06.md), candidate `b459a6e041b2675582019cde5c75ecf2969318ba`; Go patch version unknown. This does not cover v9 or the historical upgrade |

The repository also retains a [40-block synthetic v9-to-v10
devnet fixture](../conformance/fixtures/devnet/v9-upgrade-replay/meta.json)
with a Go build record. It tests the upgrade implementation but is not
historical Mocha or mainnet traffic and does not close F1.8.

## Required coverage still without exact-candidate receipts

- **Mainnet v9 era:** The [network upgrade history](https://docs.celestia.org/operate/maintenance/network-upgrades/)
  lists v9 at height 11,771,699. An independent archive RPC returned app
  version 8 in [header 11,771,698](https://celestia-archive-rpc.polkachu.com/block?height=11771698)
  and app version 9 in [header 11,771,699](https://celestia-archive-rpc.polkachu.com/block?height=11771699).
  There is no identified exact-candidate receipt for 11,771,699–14,894,000.
  Between the two documented mainnet replays,
  14,914,201–15,105,000 has no identified receipt. The first run may have
  continued beyond 14,914,200, but its final inclusive height is unknown.
- **Mocha v9 and upgrade:** Mocha-5 began at app v9. The
  [upgrade history](https://docs.celestia.org/operate/maintenance/network-upgrades/)
  lists v10's upgrade height as 1,082,619. Public Mocha RPC headers report
  app version 9 at [1,082,619](https://rpc.celestia-mocha.com/block?height=1082619)
  and app version 10 at [1,082,620](https://rpc.celestia-mocha.com/block?height=1082620).
  No exact-candidate replay receipt is identified for the v9 era through
  1,082,619 or for 1,082,620–1,624,500, including the transition.
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

On 2026-11-06, HTTP HEAD requests to the two named Polkachu snapshot
archives (`celestia_15123013.tar.lz4` and `celestia_1702915.tar.lz4`)
returned 404. Public archive RPC headers establish the upgrade boundaries,
but they do not supply a runnable pre-upgrade state snapshot. The required
isolated replay still needs a historical checkpoint and non-signing compute.

## Provider response and reproducible receipt review

The [SnapsVault provider response](https://github.com/denodesxyz/snapsvault/issues/17#issuecomment-6685525809)
reports that its catalog starts on 2026-10-01 and does not retain the requested
mainnet 11,771,698 or Mocha 1,082,619 checkpoints. It identifies QubeLabs
genesis archive snapshots as an alternative. That is a provider lead, not a
downloaded, checksummed, restored or replayed state artifact. Archive access
alone does not establish that an application database retains a restorable
historical version. No new v9 coverage follows from this response.

For exact-candidate evidence, the entire mainnet v9 interval
**11,771,699–15,116,657** still lacks a retained per-height receipt ledger in
this inventory, including the historically documented subranges above.
Likewise, Mocha's v9 era and historical transition remain unqualified.

The offline [receipt audit](receipt-coverage.md) reproduces the 101-height
transaction-result coverage from the retained JSONL, with its SHA-256 and
inclusive ranges. It can report gaps when supplied the privately collected
live receipts, but does not reconstruct those receipts or certify the
three-day observation. The [#299 observation update](https://github.com/celestiaorg/celestia-rs/issues/299#issuecomment-6678041269)
records an interval beginning 2026-11-06 05:43:12 UTC, with earliest possible
completion 2026-11-09 05:43:12 UTC. Elapsed calendar time does not establish
that the interval completed. The live receipt files and final monitoring,
candidate, restart and signing evidence are not retained in this inventory.

## Receipt review on 2026-11-24

The [Mocha genesis replay manifest](evidence/follow-receipts-2026-11-12/manifest.json)
binds candidate `1cb60aa695f9878871a01d431969c08b732dc1db` to the
executable, pinned public genesis and compressed ledger. The offline
[follower audit](follow-receipts.md) passed for `mocha-5` heights **1–100**
inclusive, with header 101 checked and zero transactions. The reference
was historical network headers. The manifest identifies no Go executable.
For that binary the remaining required inclusive interval is
**101–1,082,620**, including the first v10 block. For a different candidate
the full **1–1,082,620** interval needs its own receipts. A longer run
recorded height 112,904 but lacked a terminal receipt at
[closeout](validator-qualification-closeout-2026-11-13.md#unfinished-work-remains-unqualified).

The [mainnet archive catalog](evidence/validator-qualification-closeout-2026-11-13/snapshot-catalog/catalog.jsonl)
was subsequently inspected. Neither cataloged snapshot supplies state at
or before 11,771,698. Older chunk directories seen in the tar still lack
a verified restore. The historical mainnet v9 gap remains **11,771,699–15,116,657**.

For the [live observation](https://github.com/celestiaorg/celestia-rs/issues/299),
the [first-hour record](evidence/validator-qualification-closeout-2026-11-13/first-hour/window.json)
covers mainnet heights **15,815,626–15,816,886** on November 12. Later
collection failures are documented in the closeout. The continuous
M13-to-M14 record and three-day candidate observation remain unqualified.

## Per-version mainnet replay record on 2026-11-25

The [historical replay test](../conformance/README.md#historical-mainnet-replay)
has one ten-block window per deployed app version. Only the v10 window,
15,558,001–15,558,010, executes. Its checkpoint is the complete Go
snapshot at 15,558,000 from the QubeLabs archive. A script that does not use
celestia-rs rebuilt its 27 IAVL roots and reproduced the signed app hash
`75CAADBC12D83CC7D5031FFC1BE8D79FDBA0A733B9D57649A8D71AA238126D47`.
All ten blocks then matched.

No complete public checkpoint was found for windows 2 to 9. The other 191
snapshot directories in that archive are incomplete. Public archive RPCs
serve state proofs only from height 3,156,484, and proofs cannot rebuild a
full IAVL state. The test therefore pins the store roots before windows 4
to 10, which those proofs do give, and checks them against the signed
headers. These anchors show that celestia-rs cannot restore the states
before windows 4 to 8, and has no handler for the upgrade plans due at
their first heights. The state before the v9 window has the same 25 stores
that celestia-rs mounts and no pending plan. It is the one window from 2 to
9 where a checkpoint would test execution parity.

Consequences for operator-facing claims:

- **Replay from genesis is not supported.** celestia-rs has no state
  machine for app versions 1 to 8 and cannot cross their upgrades.
- **Parity is tested for app v10 only** in this repository's retained
  record. v9 parity rests on the historical M13 run above, which has no
  retained receipts.
- A node must start from a snapshot at or after 11,771,699, the first v9
  height. The `v9` upgrade handler (v8 to v9) is registered but not
  implemented, so celestia-rs cannot cross that boundary either.
