# Live transaction-result availability — 2026-11-06

This read-only probe advances
[#298](https://github.com/celestiaorg/celestia-rs/issues/298); it does not
claim transaction-result parity. At 00:39:28 UTC, the mainnet validator's
running executable was identified as Rust commit
`b459a6e041b2675582019cde5c75ecf2969318ba`. The loaded text inode
reported by `lsof` (24168243) matched the installed executable's
`stat` inode, and `version --long` reported that commit.

`packaging/compare-block-results.sh` requested the inclusive range
15,627,609–15,627,629 from local RPC
`http://127.0.0.1:26757` and independent reference
`https://rpc.celestia.pops.one`. Both `/status` responses reported
chain ID `celestia`. The 21
[per-height receipts](../conformance/mainnet-rpc/tx-result-receipts/2026-11-06-mainnet.jsonl)
record 21 unavailable comparisons and zero matches or mismatches.
The local `/block_results` response was HTTP 500 with
`node is not persisting finalize block responses`. A read-only
`/tx_search` request also returned HTTP 500:
`transaction indexing is disabled`. Local `/block` returned the
requested block, but it does not contain this node's execution results.

The comparison tool compares the consensus-relevant result fields
`code`, `data`, `gas_wanted` and `gas_used` at each requested
height when both RPCs expose `/block_results`. It records result
counts, canonical SHA-256 digests, first differing index and explicit
unavailable errors. It is bounded to 10,000 heights per invocation and
does not read signing keys or modify validator state.

The live validator RPC cannot provide historical local transaction
results through either tested endpoint. The 21 unavailable receipts
must not be interpreted as parity. A later
[isolated replay](tx-result-isolated-replay-2026-11-06.md) established
every-height result parity for a separate 101-block historical range,
without changing the live validator. At-tip continuous result evidence
is still missing, so C1.1, F1.8 and #298 remain open.
