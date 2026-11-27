# Sampled mainnet data roots

Block sync accepts a committed block of app versions 1 to 8 only if the data
square rebuilt from its txs reproduces the header's data hash
(`app::proposal::verify_committed_block_data`, through `verified_block_eds`
with the historical go-square builders). From v9 the block goes through
ProcessProposal, which checks the same root. This campaign checks that
rebuild on real mainnet blocks of every era.

`campaign.py` picks, per app version, 300 evenly spaced heights, the first
and last 50 blocks of the era, and hard cases from 150 random
`/blockchain` windows of 20 block metas each: the largest blocks, the
blocks with the most txs, and empty blocks. It fetches each block over
public RPC (rpc.celestia.pops.one and celestia-archive-rpc.polkachu.com,
with backoff on 429 and 5xx) and pipes it into the `data_root_check`
example. That example rebuilds the square, requires the header's data
hash and the block's `square_size`, and records the block's shape. On v9
and later blocks it also runs the stateless half of ProcessProposal: tx
size, the BlobTx envelope, the SDK decoding, no `MsgPayForBlobs` outside a
BlobTx, and `ValidateBlobTx` with share commitments.

```sh
cargo build --release -p app --example data_root_check
python3 conformance/mainnet-rpc/data-root/campaign.py plan /tmp/plan.json
python3 conformance/mainnet-rpc/data-root/campaign.py run /tmp/plan.json /tmp/results.jsonl
python3 conformance/mainnet-rpc/data-root/campaign.py report /tmp/plan.json /tmp/results.jsonl
```

## Result (2026-11-27, tip 16,265,074)

Every sampled block rebuilds its committed data root and square size.

| App version | Heights | Blocks checked | Mismatches | Stateless ProcessProposal rejects | Empty | PFB-only | Share v1 | ≥50 blobs | Max square | Max blobs |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| v1 | 1–2,371,494 | 428 | 0 | n/a | 20 | 5 | 0 | 53 | 64 | 103 |
| v2 | 2,371,495–2,993,219 | 428 | 0 | n/a | 6 | 24 | 0 | 235 | 64 | 66 |
| v3 | 2,993,220–6,680,339 | 428 | 0 | n/a | 7 | 162 | 0 | 11 | 128 | 304 |
| v4 | 6,680,340–6,748,821 | 424 | 0 | n/a | 7 | 274 | 0 | 5 | 128 | 4182 |
| v5 | 6,748,822–8,662,012 | 428 | 0 | n/a | 19 | 249 | 0 | 143 | 128 | 525 |
| v6 | 8,662,013–10,960,599 | 428 | 0 | n/a | 19 | 328 | 302 | 167 | 256 | 829 |
| v8 | 10,960,600–11,771,698 | 426 | 0 | n/a | 15 | 350 | 398 | 219 | 128 | 3369 |
| v9 | 11,771,699–15,116,658 | 424 | 0 | 0 | 6 | 373 | 416 | 137 | 64 | 764 |
| v10 | 15,116,659–16,265,074 | 422 | 0 | 0 | 6 | 381 | 416 | 98 | 64 | 830 |

The campaign found no blob with share version 1 in the blocks it sampled
from v3 to v5. Such blobs appear from v6.

## Regression set

`campaign.py harvest` keeps each era's hard cases in
[`hard-cases.json`](hard-cases.json): the era's first and last block, its
largest square, the blocks with the most blobs and the most txs, a block
with share version 1 blobs, a PFB-only block, an empty block, the block
with the longest tx, and a small blob block. That is 61 blocks, with their
committed data hash and square size. The 34 blocks whose txs total at most
256 KiB are in
[`../../fixtures/mainnet/data-root-hard-cases.jsonl.gz`](../../fixtures/mainnet/data-root-hard-cases.jsonl.gz).
`cargo test -p app --test mainnet_data_roots` rebuilds them offline and
requires each era's coverage. The ignored test
`fetched_hard_cases_rebuild_their_data_root` checks all 61 from a fresh
fetch (see its doc comment), including the largest v6 block (8,992,760,
14.7 MB of txs).
