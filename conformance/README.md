# Conformance

This directory holds **golden vectors** generated from Go celestia-app, plus the Go program that generates them. Every Rust crate that has byte-level Go compatibility requirements asserts against fixtures here.

## Layout

```
conformance/
├── fixtures/        # committed golden vectors (JSON, JSON.zst, or binary.zst)
│   └── <crate>/<scenario>/<file>.json
│   └── <crate>/<scenario>/<file>.meta.json   # source chain, height, app version, generating commit
└── fixturegen/      # Go program that emits fixtures; the only Go in the repo
```

## Regenerating fixtures

```bash
cargo xtask refresh-fixtures                       # all scenarios
cargo xtask refresh-fixtures --scenario bech32-addresses
cargo xtask refresh-fixtures --crate-name hash
```

The Rust runner shells out to `go run ./conformance/fixturegen ...`. Go 1.24.11 is required for regeneration only; consumers of fixtures (Rust crates) need no Go toolchain.

Tx decoding vectors (`fixtures/txdecode/`, celestia-app v10.4.0 `TxDecoder`)
come from their own module, which also regenerates the `txdecode` crate's
static interface-registry table:

```bash
cd conformance/txdecode
GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run . -out ../fixtures/txdecode
```

## Query corpus (M11)

`querycorpus/<scenario>.json.gz` holds every query of a devnet scenario
asked of Go celestia-appd and Go's answer: ABCI `Query` (gRPC methods of
every registered service with empty, valid and malformed requests,
pagination variants, historical heights and proofs, `/store`, `/app`,
`/p2p`, `/custom` inclusion proofs) and the REST (grpc-gateway) GET form
of each. `crates/devnet-conformance/tests/query_corpus.rs` replays the
scenario through the Rust app and requires byte-identical ABCI responses
and equal REST status and JSON bodies (Go map-order lists compared
sorted; two documented divergences).

```bash
# re-drive a recorded scenario into a fresh Go app and record (no consensus)
DEVNET_BIN=<dir> DEVNET_PORT_OFFSET=10000 conformance/querycorpus/record.sh empty gov ...
# the celestia-core-backed services (cmtservice, tx index, tx status, gas
# estimation): a live Go node, its RPC traffic recorded into
# fixtures/devnet/comet-queries/rpc.json.gz and replayed by a mock
DEVNET_BIN=<dir> DEVNET_PORT_OFFSET=10000 conformance/querycorpus/record-comet.sh
```

`crates/devnet-conformance/examples/query_diff.rs` and `rest_diff.rs` print
the mismatches of one scenario grouped by method.

## Expanding cached mainnet blocks

The `square-mainnet-blocks` fixture is generated from cached CometBFT block
JSON files under `conformance/fixturegen/cmd/testdata/`. To add more source
blocks, run the fetch utility from the fixturegen module, then regenerate the
derived golden fixture:

The generated `mainnet-blocks.json` stores source transactions and Go-derived
row/column roots plus DAH hashes, but not the derived ODS share dump. This keeps
the 100-block corpus below GitHub's recommended 50 MB single-file threshold
while preserving the DAH conformance signal.

```bash
cd conformance/fixturegen
go run . square-fetch-mainnet-blocks --rpc <cometbft-rpc-url> --start <height> --count <n>
cd ../..
cargo xtask refresh-fixtures --scenario square-mainnet-blocks
```

## Historical mainnet replay

The historical replay test uses the `celestia-rs` binary built by Cargo.
Its test matrix contains ten consecutive blocks for each app version
deployed on mainnet (1, 2, 3, 4, 5, 6, 8, 9 and 10). For each window
that executes, it compares stored block
bytes, recomputed block IDs and data roots against the pinned archive
responses. It also recomputes each post-block app hash from the locally
committed multistore and checks it against the pinned successor header.

To replay the v10 window, download the Go SDK checkpoint at height
15,558,000. The [checkpoint fetcher](mainnet-rpc/fetch-replay-checkpoint.py)
downloads and verifies 684 MB of snapshot chunks from the
[QubeLabs archive](https://snaps.qubelabs.io/celestia/celestia_2026-11-03.tar).

```sh
SDK_ROOT=$(mktemp -d)
python3 conformance/mainnet-rpc/fetch-replay-checkpoint.py "$SDK_ROOT"
cat > "$SDK_ROOT/replay.json" <<EOF
{
  "rpc": [
    "https://rpc.celestia.pops.one",
    "https://public-endpoint.celestia-mainnet.quiknode.pro"
  ],
  "reference_rpc": "https://rpc.celestia.pops.one",
  "checkpoints": {
    "10": {
      "snapshot": "$SDK_ROOT",
      "height": 15558000,
      "trust_hash": "BF85423368904C12CE93F584120C89EF0B2EDFE3B9C9E9931AE918EE61E8F8B1"
    }
  }
}
EOF

CELESTIA_MAINNET_REPLAY_CONFIG="$SDK_ROOT/replay.json" \
  CELESTIA_MAINNET_REPLAY_VERSION=10 \
  cargo test --release --locked -p celestia-rs --test mainnet_replay \
  historical_mainnet_replay -- --ignored --nocapture
```

For the full run, add the path to the canonical mainnet genesis in the
`genesis` field and provide Go SDK checkpoints for the other windows that
need them. Each checkpoint height must precede its window. Set its
`trust_hash` to the independently verified uppercase block hash at that
height. Run the same test with `CELESTIA_MAINNET_REPLAY_VERSION` unset.
The optional `reference_rpc` verifies checkpoint anchors and defaults to
the archive RPC named in the fixtures.
Each replay uses a fresh temporary home. The full run fails if a window
lacks inputs or disagrees with mainnet.

On 2026-11-23, a build of upstream `63d259f6` (binary SHA-256
`5e55de3011c6333e8d0780c3a2885ae36e41c98dd8dac996a8c4a1862b52ea38`)
replayed mainnet heights 15,558,001–15,558,010. All ten block bodies,
block IDs, data roots and locally committed app hashes matched. The final
app hash was `3B6020A36F10AF20E2AE6EA1FFBE706D8E167A1E8B8A5AAC57C4AD84C0172280`.
The full run failed. Mainnet v1 genesis initialization returned
`gov genesis params are nil`. The run lacked complete state checkpoints for
v2–v6, v8 and v9.

On 2026-11-25, `celestia-rs genesis validate` on the canonical mainnet
genesis got past gov: the node now reads the Tendermint v0.34
`app_version` field and stores the three legacy gov parameter objects in
the `x/params` subspace `gov`, as cosmos-sdk v0.46 does. InitChain now
fails at IBC with `invalid ibc channel genesis state parameters: upgrade
timeout timestamp invalid: 0: upgrade timeout is invalid`. The node has no
app version 1 state machine: it mounts the v9 store set instead of v1's
(no `qgb` store), and it runs ibc-go v8 and the v9 ante handler. The v1
window cannot match until that exists.

The offline block reconstruction check uses retained archive RPC responses.
It rebuilds the block IDs and data roots for all 90 fixture blocks and
compares the app hashes carried by the headers. Application execution
requires the replay command above.

```sh
cargo test --locked -p celestia-rs --test mainnet_replay \
  historical_mainnet_block_reconstruction
```

[`blocks.jsonl.gz`](fixtures/mainnet/blocks.jsonl.gz) contains the complete
`/block` result for each sampled height and each window's last successor.
The responses were captured on 2026-11-23 from Polkachu's archive RPC.
P-OPS independently agreed on the block IDs, app hashes and data roots
for the sampled heights and on the successor commitments. The fixture's
SHA-256 is `ddc99bb0a9a061c2dd6cfa3d7b094b1875026d06ac2cade13a0372a058069c5f`.
Regeneration uses the pinned heights and commitments in
[`replay-windows.json`](fixtures/mainnet/replay-windows.json).

```sh
python3 conformance/mainnet-rpc/fetch-replay-blocks.py
```

## When to regenerate

- After mainnet upgrades to a new app version.
- After adding a new fixture scenario.
- Never as part of normal feature work — regeneration is a deliberate step that may invalidate test expectations.

Each fixture has a sibling `<file>.meta.json` recording: source chain, height (if applicable), app version, celestia-app commit SHA at generation time, timestamp.
