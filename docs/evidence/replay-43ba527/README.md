# Pinned replay samples for app versions 9–12

On November 24, 2026 (Pacific time), three isolated RPC followers executed
1,000 consecutive historical blocks each. All used the release binary built
from clean `origin/main` at `43ba527cce4425c2aea63f1165eb5d3c086f78d3`.
A separate Go-recorded test chain supplied 1,000 app-v12 blocks. All height
ranges are inclusive.

| App version | Chain | Executed heights | Transactions | Verification |
|---|---|---:|---:|---|
| 9 | `mocha-5` | 1–1,000 | 0 | [Genesis replay audit](audit.json) |
| 10 | `celestia` | 15,530,001–15,531,000 | 4,518 | [Mainnet replay audit](audit-v10.json) |
| 11 | `mocha-5` | 2,929,501–2,930,500 | 2,876 | [Mocha replay audit](audit-v11.json) |
| 12 | `test` (synthetic) | 6–1,005 | 0 | [Continuous and reopened replay results](v12-extended.log) |

## Historical replay

The [manifest](manifest.json) identifies the binary, Rust toolchain, inputs,
RPCs and SHA-256 digests. A [subsequent qualified rebuild](build-qualification/evidence.json)
recorded the pinned Git revision, tree and clean worktree before and after
compilation into an empty target directory. The [build transcript](build-qualification/check.log)
records the rebuilt executable's SHA-256. A byte-for-byte comparison with
the retained replay executable passed, and all three historical ledgers
record that digest in their opening record.

The comparison reference for each executed height H
was the signed network header at H+1. The follower checked each app hash,
validator set and transaction-result commitment. The retained ledgers include
the consensus result fields (code, data and gas wanted/used). The offline
audit recomputed their commitments. The [public edge headers](edge-headers.json)
also matched the first executed block ID and final signed header ID, app hash
and results hash for each range. Network headers do not identify the Go binary
patch that produced each block.

- **v9.** The fresh home started from the public genesis pinned in the
  candidate source. The run used a 10,000-hour historical trusting period.
  A primary RPC status timeout appears in the [transcript](replay.log).
- **v10.** The 69 [snapshot chunks](snapshot-15530000.json) are from
  Polkachu's `celestia_15609249.tar.lz4`, retained from the
  [November 6 replay](../../tx-result-isolated-replay-2026-11-06.md).
  The snapshot metadata hash and restored application hash matched that record.
  The initial 336-hour trust period rejected the expired anchor before
  execution. The [failed startup](v10-expired-anchor.log) and its
  [partial ledger](v10-expired-anchor.jsonl) are retained. The completed
  run used the same documented block hash, a fresh home and a 720-hour
  historical trusting period. [Replay transcript](v10-historical.log)
- **v11.** Only the 18 numeric [SDK snapshot chunks](snapshot-2929500.json)
  at height 2,929,500 were copied from the local Mocha node. Two public RPC
  endpoints [agreed on the anchor](anchor-2929500.jsonl) before restore.
  The follower used the default 336-hour trusting period.
  [Replay transcript](v11-replay.log)

To reproduce a ledger audit from the candidate checkout, set `EVIDENCE` to
this directory and supply the corresponding chain, range and compressed file.
For example, the mainnet run can be audited with:

```sh
python3 scripts/quality/follow_receipt_audit.py \
  "$EVIDENCE/heights-15530001-15531000.jsonl.gz" \
  --chain celestia --first 15530001 --last 15531000 \
  --candidate 43ba527cce4425c2aea63f1165eb5d3c086f78d3 \
  --binary-sha256 41c6d0677a446bacaab2ce625d13f3e0ace2eae771759d7aed081419c59c64d3
```

## Historical upgrade headers

Two additional light-client runs verified signed headers sequentially across
activation. On mainnet the trusted height was 11,771,697 and the verified
range was 11,771,698–11,771,700. On Mocha the trusted height was 1,082,618
and the verified range was 1,082,619–1,082,621. The trust hashes came from
Polkachu and Pops respectively. Both runs used a 10,000-hour historical
trusting period. These checks provide header coverage only. The subsequent
skipping verification to the tip adds no intermediate execution heights.
[Mainnet headers](mainnet-boundary.jsonl) · [Mainnet transcript](mainnet-light.log) ·
[Mocha headers](mocha-boundary.jsonl) · [Mocha transcript](mocha-light.log)

## Go-recorded v12 replay

The [upstream capture](../../../conformance/fixtures/devnet/upgrade/v12-main-upgrade/README.md)
bootstrapped a synthetic chain through height 5 with Go v11.0.0. Upstream
celestia-app `bb3a9e52d09620323b35b8305922f764b2df6a3e` executed the
1,000 v12 blocks from height 6. Each v12 block had zero transactions.
The comparison used the pinned Rust application code and checked the full
recorded FinalizeBlock responses and commits through 1,005, once continuously
and once reopening the application after height 5.

The [capture and test patch](v12-1000.patch) extends the fixture loop and
points the test at the longer recording. To repeat the comparison, apply
the patch to a scratch checkout of the candidate, create a directory named
by the absolute path `CELESTIA_V12_CAPTURE`, decompress
[v12-abci.rec.gz](v12-abci.rec.gz) to `abci.rec` there, export the variable
and run `cargo test --release --locked -p devnet-conformance --test v12_upgrade`.
The unchanged repository fixtures also passed all eight selected tests for
the [v9→v10](v9-v10.txt) and [v11/v12](v11-v12.txt) upgrades.

## Remaining F1.8 coverage

For this candidate the mainnet v9 execution interval **11,771,699–15,116,657**
and Mocha execution interval **1,001–1,082,620**, including the first v10
block, remain outstanding. A restorable mainnet checkpoint at or before
11,771,698 is still needed. The three historical samples and synthetic
fixture do not establish the continuous mainnet P2P record.
[Replay issue #302](https://github.com/celestiaorg/celestia-rs/issues/302) ·
[Observation issue #299](https://github.com/celestiaorg/celestia-rs/issues/299)
