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

### Coverage per app version

Windows 2 to 8 start at the first block of their app version. The v9
window starts mid-era, at 15,105,001. A window can execute only from a
checkpoint: the complete Go state at the height before it.

| App version | Window | Pre-state anchor | Replay result |
|---|---|---|---|
| 1 | 1–10 | Genesis | Fails: genesis initialization passes gov and stops at IBC with `invalid ibc channel genesis state parameters`; there is no app version 1 state machine |
| 2 | 2,371,495–2,371,504 | None | Not executed: no checkpoint |
| 3 | 2,993,220–2,993,229 | None | Not executed: no checkpoint |
| 4 | 6,680,340–6,680,349 | 6,680,339, verified | Not executed: no checkpoint. Expected to fail at restore and at 6,680,340 |
| 5 | 6,748,822–6,748,831 | 6,748,821, verified | Not executed: no checkpoint. Expected to fail at restore and at 6,748,822 |
| 6 | 8,662,013–8,662,022 | 8,662,012, verified | Not executed: no checkpoint. Expected to fail at restore and at 8,662,013 |
| 8 | 10,960,600–10,960,609 | 10,960,599, verified | Not executed: no checkpoint. Expected to fail at restore and at 10,960,600 |
| 9 | 15,105,001–15,105,010 | 15,105,000, verified | Not executed: no checkpoint. No structural blocker |
| 10 | 15,558,001–15,558,010 | 15,558,000, verified | All ten blocks match |

celestia-rs runs every app version below 10 with celestia-app v9.0.8
semantics ([ADR 0004](../docs/adr/0004-app-version-9.md)). It mounts the
same 25 stores for all of them and registers no upgrade handler for
`v2` to `v8`. So a faithful checkpoint cannot replay windows 4 to 8:

- **Store set.** A restore must fill every store that celestia-rs mounts.
  The v3 state at 6,680,339 is Cosmos SDK v0.46 state. Its commit info
  includes the memory store `mem_capability` with an empty hash. It lacks
  `circuit`, `consensus`, `hyperlane`, `minfee`, `warp` and `zkism`. The
  states before the v5, v6 and v8 windows lack `zkism`.
- **Upgrade plan.** The state before each of windows 4 to 8 holds the plan
  `v4`, `v5`, `v6` or `v8`, due at the window's first height. Its
  `PreBlocker` halts without a handler.

Windows 2 and 3 need the v1 to v2 and v2 to v3 store and module migrations,
which celestia-rs also lacks. Their pre-states are not public, so the test
cannot pin their store sets.

The offline test `historical_replay_blockers_match_record` checks this
record. Each `pre_state` in [`replay-windows.json`](fixtures/mainnet/replay-windows.json)
holds the store roots from `abci_query` multistore proofs. The test hashes
them and compares the result with the window's first header app hash. It
then derives each blocker from the celestia-rs store set and upgrade
handlers. It fails if a derived blocker differs from the recorded one.
Regenerate the anchors with
[`fetch-replay-anchors.py`](mainnet-rpc/fetch-replay-anchors.py).

```sh
cargo test --locked -p celestia-rs --test mainnet_replay \
  historical_replay_blockers_match_record
```

#### Where checkpoints come from

The QubeLabs archive (`celestia_2026-11-03.tar`, ETag
`"6aea4a03-95f79f8e800"`) holds 193 snapshot directories. Only
15,556,500 and 15,558,000 are complete. Every other directory ends on a
full 10 MB chunk with no metadata entry. The largest v9-era one,
14,736,000 (45 chunks), has a truncated zlib stream. The P-OPS archive RPC
answers state proofs from height 3,156,484. That is enough for the anchors
but not for a full state: the IAVL tree shape is part of the app hash, and
the v10 checkpoint alone has 30,936,015 nodes.

To populate window N, an operator needs a Go node that has committed the
height before it, H−1:

1. Sync mainnet with celestia-app, with `halt-height = H−1` in `app.toml`.
2. Run `celestia-appd snapshots export --height H−1` with the celestia-app
   release that committed H−1.
3. Run `python3 conformance/mainnet-rpc/verify-replay-checkpoint.py
   <snapshot root>/H−1/<format>`. It rebuilds every IAVL root, compares
   each with the pinned anchor and the commit hash with the signed header.
4. Add the snapshot root, H−1 and the block hash at H−1 under key `N` in
   `checkpoints`.

##### Go checkpoint campaign

The `checkpoints` job of
[`operator-benchmark.yml`](../.github/workflows/operator-benchmark.yml)
does these steps unattended on one `n2-standard-8` VM in `us-central1-a`.
Dispatch it from `main`. [`checkpoints-vm.sh`](../scripts/bench/checkpoints-vm.sh)
builds celestia-app `v11.0.0` with the multiplexer and block-syncs mainnet
from the verified genesis. Its peers are two archive nodes, 20 peers of
Polkachu's archive RPC node, the `peers` input and the seeds. For each
planned height H it does these steps:

1. It sets `halt-height = H+1`. The multiplexer refuses to finalize a block
   at or above `halt-height`, so the node commits H and then stops.
2. It runs `celestia-appd passthrough vN snapshots export --height H`, where
   N is the app version of H. N is 3 for app versions 1 to 3, because
   v3.13.0 runs those versions.
3. It runs `verify-replay-checkpoint.py` on the snapshot. If a window starts
   at H+1, the script uses the pinned fixture. If not, it uses `--app-hash`
   with the app hash of header H+1. Both archive RPCs must agree on that
   hash.
4. It writes `checkpoint.json` with the height, app version, block hash,
   snapshot format, the size and SHA-256 of each chunk, and the binary
   versions and checksums. Then it resumes the sync.

A failed verification stops the campaign. The metadata is then kept as
`failed.json`, not `checkpoint.json`, so a resumed run exports that height
again and does not go past it. The default plan is the state
before each window after v1, plus 2,371,489 (five v1 blocks before the
v1 to v2 upgrade).

| Action | Inputs | Result |
| --- | --- | --- |
| `preflight` | `checkpoints=preflight` | Lists the required and optional Compute permissions |
| `provision` | `checkpoints=provision checkpoint_name=mainnet-1`, optional `checkpoint_heights`, `checkpoint_disk_gb` (default 1000), `checkpoint_hours` (default 168), `peers` | Creates the VM. If the campaign's disks already exist, the VM resumes from them |
| `status` | `checkpoints=status checkpoint_name=mainnet-1` | Writes the progress line (height, next checkpoint, blocks/s over the last hour, ETA) and one row per checkpoint to the job summary |
| `collect` | `checkpoints=collect checkpoint_name=mainnet-1` | Does the same as `status`, and uploads the metadata, polls and log excerpt as an artifact |
| `delete` | `checkpoints=delete checkpoint_name=mainnet-1 checkpoint_delete_disks=none\|data\|all` | Deletes the VM and, if requested, its disks |

The node stops 30 minutes before the VM deletes itself. The VM and its
guest attributes are then deleted. Two disks remain:

- `ckpt-<name>-data` (pd-balanced, default 1000 GB) holds the node home and
  the multiplexer binary. A later `provision` with the same name resumes
  from this disk, not from genesis.
- `ckpt-<name>-out` (pd-balanced, 50 GB) holds `<height>/<format>/<chunk>`
  and `<height>/checkpoint.json`. This is the snapshot root for `replay.json`.
  Later VMs, such as a replay VM for app versions 4 to 8 or the end-to-end
  genesis follow, attach this disk read-only.

The retained disks cost $0.10 per GB-month: $105 a month for both at the
defaults, and $5 a month for the checkpoint disk alone. Delete the data disk
when the plan is done. The service account is not known to have
`compute.disks.delete`. If `delete` cannot remove a disk, it prints the
`gcloud compute disks delete` command for an owner. You cannot download
chunks without a VM. Copying them to a Cloud Storage bucket needs an owner
to create the bucket and a VM service account that can write to it.

Sampled mainnet blocks hold about 80 KB of transactions in v1, 270 KB in
v2, 0.9 MB in v3, 0.2 to 0.3 MB in v4 to v8 and 0.1 MB in v9. That is
about 3.8 TB of block data to 6,680,339 and 5.7 TB to 15,105,000. Plan
the first 168-hour run for the v1 to v2 and v2 to v3 checkpoints. It
continues toward 6,680,339 until its deadline. Later runs resume.

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

On 2026-11-25, a build of `8aec12b` with this record's test-only changes (SHA-256
`1f3558f06806bb0c1ade58c4f4bb2a4f6c94acfdbe712ed548af0743c31aeafc`)
replayed the v10 window again from the same checkpoint, and all ten blocks
matched. Before that run, `verify-replay-checkpoint.py` reproduced the
checkpoint's app hash from its 27 IAVL roots. A run with no checkpoints
failed each of windows 2 to 9 with the recorded blockers above.

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
