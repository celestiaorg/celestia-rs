# Persistent Comet RPC campaign

`conformance/devnet/rpc-persistence.sh` starts isolated persistent Go and Rust
nodes on the same chain. A Go validator supplies blocks to one Go and one Rust
full node. The validator stops during each capture so both RPC providers have
the same tip. The capture refuses a different chain, tip or committed app root.

The four profiles combine `nothing` or custom app pruning (ten recent versions,
interval ten, the Go SDK's minimum supported custom interval) with explicit `kv`
or `null` transaction/block indexes. Block
retention is unlimited and FinalizeBlock responses are retained. Three bank
transfers populate transaction and event indexes. Inclusion is verified by
scanning blocks, so disabled indexing cannot make workload inclusion silently
disappear. Public genesis, configuration, broadcast and inclusion inputs are
retained; validator private keys and node databases stay outside the capture.

Each profile captures forty paired requests before restart, after an explicit
validator-first restart on the existing homes, and after another twenty-five heights cross
the app-pruning boundary. Cases include latest, historical, future and missing
block/commit/results/validator queries; indexed transaction lookup and proofs;
search ordering and pagination; protobuf bank queries; and store proofs.
Decoded JSON comparisons retain array order, scalar types, null/empty values,
proof fields and exact errors. JSON key ordering and whitespace are immaterial.
HTTP status and content type are compared. Date, X-Server-Time and derived
Content-Length are excluded. Raw bodies and their hashes remain in the output.

Build the pinned oracle with `conformance/devnet/build-bins.sh` and the candidate
with `cargo build -p celestia-rs --locked`. Then run:

```sh
python3 conformance/mainnet-rpc/observe-mainnet.py \
  --endpoint https://rpc.celestia.pops.one --out /tmp/mainnet-observation.json
MAINNET_OBSERVATION=/tmp/mainnet-observation.json \
RPC_PERSISTENCE_OUT=/tmp/rpc-persistence \
  bash conformance/devnet/rpc-persistence.sh
```

The output directory must not exist. Oracle selection is checked against raw
status/ABCI response hashes and must be less than twenty-four hours old. Source,
binary and compiler receipts accompany the output. Set `DEVNET_BIN`, `RS_BIN`
and `DEVNET_PORT_OFFSET` for alternate build locations and unused local ports.
`RPC_PERSISTENCE_PROFILES=retain-kv` selects one profile for diagnosis; it does
not represent the complete matrix. Failed captures preserve partial responses,
mismatches and node logs, and return a failure status.

Run the offline capture-controller regressions with:

```sh
python3 -m unittest discover -s conformance/mainnet-rpc/persistence -v
```

This campaign is a bounded part of #189 and #181. A passing run does not cover
Cosmos gRPC/REST persistence, WebSocket reconnects, snapshot state sync,
block-store pruning, disabled response retention, or pre-v10 queries through the
Go historical-binary multiplexer. Search pagination covers page/per-page, not
Cosmos key/offset pagination. The captured proof bytes are compared but this
controller does not independently verify ICS23 validity. Outputs must be
reviewed and admitted through the evidence ledger before counting as current
release evidence. No capture is implied merely by this source being present.

## Recorded restart blocker

The committed `restart-reproduction` is an **incomplete** run of the retained
`kv` profile on 2026-11-02. Forty paired cases matched before restart. After all
nodes restarted, the Go validator advanced to height 78 while both the Go full
node and Rust full node remained at height 16. Both reported consensus state
`17/0/8` without a proposal block. The campaign failed waiting for the Rust
follower to reach height 19; it did not complete the restart/pruning matrix or
run the remaining profiles. This is an observed shared catch-up failure; its
cause has not been isolated to either implementation.

The reproduction retains public configuration/genesis, transaction inputs and
inclusion results, all sixteen retained blocks, forty pre-restart response pairs,
raw diagnostic RPC bodies, log excerpts and the validator's complete stopped
block archive through height 88. Build receipts identify the Go source/compiler
and the exact Rust binary/commit/toolchain. Archived capture/helper source files
match the original receipt. The verifier checks hashes, signed transaction
presence, chain/heights and the retained app root against the validator archive.
It distinguishes Comet status's latest-header root from the application's
post-height-16 root recorded by the restart handshake.

```sh
python3 conformance/mainnet-rpc/persistence/verify_reproduction.py
```

The verifier explicitly reports an incomplete campaign. Its passing integrity
check and the offline controller tests do not establish restart readiness. The
archived simultaneous-restart source preserves that failing experiment. The
current driver uses the explicit ordered restart below; it does not resolve the
shared simultaneous-restart failure. #189 and the readiness tracker remain open.

## Ordered-restart evidence

`ordered-evidence` retains 480 matching paired cases across the four profiles
and all three phases. The validator starts first and advances three heights
beyond the persisted tip before either follower restarts. Both followers then
resume progress before the validator stops for a matched-tip query capture.
This startup order is a condition of the result.

Three profiles come from the first batch. Its `prune-kv` after-restart sample
was rejected before any comparison because provider tips differed; that original
failed sample remains in the package. The full `prune-kv` profile was separately
recaptured after the driver began waiting for both providers at the higher
observed tip. Its 120 matching cases are bound to that separate source/build
receipt. No failed response was rewritten into a success.

Every profile retains its genesis, configuration, transaction inclusion records
and complete stopped validator block archive. Exact source snapshots, binary
hashes, compiler/build receipts and fresh mainnet observations identify both
capture batches and the archive generator. The verifier checks every request
branch, response pair, configured pruning/index mode, chronological follower
progress, historical pruning result and committed root against the archive.

```sh
python3 conformance/mainnet-rpc/persistence/verify_ordered.py
```

These comparisons cover JSON values, HTTP status and Content-Type. Other HTTP
headers are not captured or compared. ICS23 proof bytes are compared, without
independent proof verification by this controller. Cosmos gRPC/REST persistence,
WebSocket reconnects, snapshots, block-store pruning, disabled FinalizeBlock
retention and pre-v10 multiplexer requests remain outside the matrix. The
simultaneous-restart reproduction remains a separate negative result. Neither
this matrix nor its passing offline integrity check establishes provider readiness.
