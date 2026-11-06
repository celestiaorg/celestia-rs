# Isolated mainnet transaction-result comparison — 2026-11-06

An isolated, unbonded Rust node replayed **101 consecutive mainnet blocks,
15,530,001–15,530,101 inclusive**, on candidate
`b459a6e041b2675582019cde5c75ecf2969318ba` (binary SHA-256
`fb66fba07c468595cc2801c9355054d8518149f4a62bb0ce78caac4d1d76efde`).
It used a fresh home and key, not a copy of either live validator's home or
signing key. The run stopped at the stated height and the temporary observer
was stopped after result collection. The live mainnet validator remained
caught up during the run.

The state source was the SDK snapshot at height **15,530,000** inside
Polkachu's public [15,609,249 pruned archive](https://snapshots.polkachu.com/snapshots/celestia/celestia_15609249.tar.lz4),
streamed on 2026-11-06. The snapshot metadata hash was
`8FC8E56A96E2668D6609ED253D033BDD5CDD983B219980F7EF305FB0CE5EB4DE`
(69 chunks). [Pops](https://rpc.celestia.pops.one/block?height=15530000)
and [Polkachu](https://celestia-archive-rpc.polkachu.com/block?height=15530000)
agreed on trusted block hash
`C5C8DCAE1B28BF293C552ADF51E2F0C0634363D241AFE6C455A53B8E78B983AE`
at height 15,530,000. The restored app hash
`7440199094FB434297C505E7EC7199D0E00920FE17308D1CBB32539EDA0FDFC3`
matched the network. The outer compressed archive was streamed and not
retained or hashed; its Go binary patch version is not established here.

The follower ran from 05:14:36 to 05:18:41 UTC and executed **454
transactions**. Its final app hash after 15,530,101 was
`6C9472B2D19985146A8103BFD11F2D05E6DADF898A556805A0018B9A795E2463`,
equal to the app hash in the public header at 15,530,102. It also checked
intermediate headers as it followed the chain.

From 05:19:48 to 05:22:12 UTC, the bounded
[`compare-block-results.sh`](../packaging/compare-block-results.sh) read
the observer's retained `/block_results` and the independent
[`rpc.celestia.pops.one`](https://rpc.celestia.pops.one) results at every
height. All **101 per-height comparisons matched**: the local and
reference result counts and SHA-256 digests agreed for the 454
transactions. The checked fields were `code`, `data`, `gas_wanted` and
`gas_used`; logs, events and other ABCI fields were outside this
comparison. The [JSONL receipts](../conformance/mainnet-rpc/tx-result-receipts/2026-11-06-isolated-replay-15530001-15530101.jsonl)
have no missing or unavailable height. Their SHA-256 is
`f6acdade007a288bf6c8de4308f624e3e7589abd6cc9f4001220a162e66eb6f8`.
The published copy redacts the observer's loopback RPC address; the raw
receipts are retained locally.

This is **every-height coverage of the stated historical range**, not
an at-tip or continuous live record. It advances [#298](https://github.com/celestiaorg/celestia-rs/issues/298)
but does not replace the three-day exact-candidate observation or close
C1.1 and F1.8. The live signing validator still does not persist its own
historical FinalizeBlock responses.
