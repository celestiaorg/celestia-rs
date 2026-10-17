# Devnet differential oracle

`devnet.sh` runs Go `celestia-appd` (as a standalone ABCI app) behind Go
celestia-core, with the Rust `abci-recorder` proxy between them, and writes
a fixture under `conformance/fixtures/devnet/<scenario>/`:

- `genesis.json` — the SDK app genesis.
- `abci.rec` — every ABCI request/response frame, readable with
  `abci_server::record` (`cargo run -p abci-server --bin abci-dump -- <file>`).
- `state/<h>.json` — full committed multistore at heights 1 and the last
  height, per store (hash + hex KV pairs), from `../devnet-oracle`.

The Rust app is correct for a scenario when replaying the recorded requests
produces byte-identical consensus-relevant responses (app hashes, tx results,
events, validator updates) and, on mismatch, the state dumps pinpoint the
diverging store and key.

```bash
conformance/devnet/devnet.sh empty 20                       # no txs
conformance/devnet/devnet.sh <name> <blocks> workloads/x.sh  # with txs
```

Only Go (module proxy) and cargo are needed; no public RPC access.
Genesis keys and times are random per run, so regenerating a scenario
replaces its fixture wholesale.

Workloads may ship `<name>.setup.sh` (genesis accounts, run before gentx)
and `<name>.genesis.py` (edits the final genesis, e.g. short gov voting
periods); `CHAIN_ID` overrides the chain id and `ALLOW_HALT=1` accepts a
workload that halts the chain (the replay then requires Rust to refuse the
same block). Shared helpers (`tx`, `rawtx` for messages the CLI cannot
build, `q`) live in `workloads/lib.sh`.

Misbehavior cannot be produced by a single honest validator, so
`inject-evidence.sh <src> <dst> <height>...` re-drives a recorded scenario
through a fresh Go `celestia-appd` with duplicate-vote evidence injected
into those FinalizeBlock requests and records Go's responses as `<dst>`.

```bash
CHAIN_ID=test ALLOW_HALT=1 conformance/devnet/devnet.sh upgrade 10 conformance/devnet/workloads/upgrade.sh
conformance/devnet/inject-evidence.sh staking evidence 20 30
```

The `ibc` workload starts a second Go chain at port offset
`DEVNET_PORT_OFFSET + 500` and links the two with cosmos/relayer v2.6.0
(`build-rly.sh` builds it into `$DEVNET_BIN`, patched to use a fixed gas
limit since celestia-app does not serve `Simulate`); the relayer needs tx
indexing on the recorded chain (`DEVNET_TX_INDEX=kv`). IBC edge cases a
relayer does not produce come from `../ibcvectors` (ibc-go testing
package; fixture `ibc-edge`).

```bash
conformance/devnet/build-rly.sh
DEVNET_TX_INDEX=kv conformance/devnet/devnet.sh ibc 10 conformance/devnet/workloads/ibc.sh
```

`workloads/cli.sh` (fixture `cli`, needs `cargo build -p celestia-rs`)
drives the Rust CLI against the Go node: every command's `--generate-only`
tx must equal `celestia-appd`'s (fees, `--gas auto` estimates, create /
edit-validator, unbond, redelegate, the generic `tx <module> <msg> --json`),
and the Rust-signed txs must commit with code 0.

```bash
cargo build -p celestia-rs
conformance/devnet/devnet.sh cli 40 conformance/devnet/workloads/cli.sh
```

## App version 9 (ADR 0004)

`APP_V9=1` records a scenario from Go celestia-app **v9.0.8** (mainnet's
version; `$DEVNET_BIN/v9/celestia-appd`, also used by the workload's CLI)
behind celestia-core v0.42.3, as the v10 multiplexer runs its embedded v9
binary. `UPGRADE_TO_V10=1` additionally reproduces the multiplexer's switch:
`abci-recorder <listen> <app> <out> <switch-cmd>` watches FinalizeBlock
responses; after the one that raises `consensus_param_updates.version.app`
it holds the following Commit response, lets every connection drain, runs
the switch command (stop v9, start v10 on the same home and address) and
reconnects all four connections before releasing the Commit. The v10 app
then opens with the v10 store loader (fibre and valaddr added at `H+1`)
and runs the `v10` handler in the next PreBlocker.

```bash
APP_V9=1 conformance/devnet/devnet.sh v9-staking 40 conformance/devnet/workloads/staking.sh
APP_V9=1 conformance/devnet/devnet.sh v9-exec 30 conformance/devnet/workloads/v9-exec.sh
APP_V9=1 conformance/devnet/devnet.sh v9-utf8 30 conformance/devnet/workloads/utf8.sh
APP_V9=1 UPGRADE_TO_V10=1 CHAIN_ID=test conformance/devnet/devnet.sh v9-upgrade 40 conformance/devnet/workloads/v9-upgrade.sh
APP_V9=1 UPGRADE_TO_V10=1 CHAIN_ID=test conformance/devnet/devnet.sh v9-upgrade-boundary 90 conformance/devnet/workloads/v9-upgrade-boundary.sh
```

Right after the switch the v10 app's check state has an empty header
(height 0) until its first commit, so CheckTx rejects signed txs (account
number 0): upgrade workloads wait two blocks before sending under v10.
`crates/fuzz-diff` `DELIVER_UPGRADE` fuzzes the same boundary in process
(`deliver-diff.sh`, `GO_APPD_NEXT`).

A v9 CheckTx that returns a Go error (e.g. a non-canonical BlobTx or a
duplicated TxRaw field) makes celestia-core stop its socket client, so
v9 workloads do not send those (`blob.sh` skips its non-canonical BlobTx
under `APP_V9`).

`cargo run -p devnet-conformance --example real_dah -- 9 <block.json>...`
rebuilds the square of real blocks (CometBFT `/block` responses from a
mainnet or Mocha RPC) with the app version's rules and checks every tx
decodes and the square size / data root match the header.

`genesis_replay` runs a public network from its genesis on the Rust app
and checks every app hash (header `h+1`) and every tx result's code and
gas against the network's `/block_results` (blocks are trusted, not light
verified). Mocha (`mocha-5`) starts at app version 9:

```bash
conformance/devnet/fetch-chain.sh https://rpc.celestia-mocha.com /tmp/mocha5 200   # genesis + first blocks
python3 conformance/devnet/fetch-chain.py https://rpc.celestia-mocha.com /tmp/mocha5 201 40000
cargo run -p devnet-conformance --example genesis_replay -- /tmp/mocha5
```

## Live mixed network

`mixed.sh <blocks> [workload.sh]` runs a Go `celestia-appd` validator and a
follower made of Go celestia-core on top of the Rust state machine
(`celestia-rs abci`). celestia-core verifies every block's app hash against
the follower's state, so reaching `<blocks>` proves live consensus
compatibility for that workload:

```bash
cargo build -p celestia-rs
DEVNET_BIN=<dir with celestia-appd, cometbft> conformance/devnet/mixed.sh 60 conformance/devnet/workloads/staking.sh
```

`mixed.sh` with `workloads/blob.sh` also exercises the Rust
ProcessProposal / CheckTx on blob blocks proposed by Go. `mixed.sh` and
`rust-proposer.sh` apply the workload's `.setup.sh` / `.genesis.py` hooks
like `devnet.sh`.

`workloads/fibre.sh` builds signed payment promises (escrow owner's
secp256k1 signature, validator ed25519 signatures from
`priv_validator_key.json`) with `conformance/fibre-client`, a small Go
program independent of celestia-app; `workloads/zkism-forwarding.sh`
submits celestia-app's own SP1 Groth16 test vectors.

## Rust proposer

`rust-proposer.sh <blocks> [workload.sh]` inverts the roles: the only
validator is celestia-core on top of `celestia-rs abci` (so every block is
built by the Rust PrepareProposal) and a Go `celestia-appd` full node
follows it. Txs are submitted to the Go node and reach the Rust mempool by
gossip. The run fails if the Go node logs a rejected proposal or an app
hash mismatch:

```bash
DEVNET_BIN=... conformance/devnet/rust-proposer.sh 60 conformance/devnet/workloads/blob.sh
```

The replay (`devnet-conformance::replay`) compares, besides FinalizeBlock,
every PrepareProposal response (txs, square size, data root),
ProcessProposal status and CheckTx response (code, codespace, gas,
priority, signer address/sequence, or exception).

## State sync and restarts (ADR 0002)

`statesync.sh [blocks] [workload.sh]` checks persistent storage and
snapshots against Go, in three phases on one chain:

1. **Go → Rust:** a fresh follower (celestia-core + `celestia-rs abci
   --home`) state-syncs from the Go validator's snapshots, then follows with
   celestia-core verifying every app hash.
2. **Restart:** the follower's app and celestia-core are killed and
   restarted; the ABCI handshake resumes it from its redb state.
3. **Rust → Go:** a fresh Go `celestia-appd` whose only peer is the Rust
   follower state-syncs from the Rust-made snapshots and follows the chain.

```bash
cargo build -p celestia-rs
DEVNET_BIN=<dir with celestia-appd, cometbft> DEVNET_PORT_OFFSET=6000 \
  conformance/devnet/statesync.sh 10 conformance/devnet/workloads/staking.sh
```

## P2P interop (M10)

`p2p-interop.sh [duration]` checks the Rust `p2p` crate against a Go
celestia-core v0.42.3 validator (`celestia-appd`): one Rust node
(`cargo run -p p2p --example p2p_interop`) dials the Go node, and the Go
node dials a second Rust node configured as its persistent peer. Both must
stay connected for `duration` seconds (default 75, covering Go's 60s ping)
in the right direction (checked on both sides, incl. Go's `/net_info`),
decode every consensus / propagation / block sync message, answer and
receive PEX requests, and never be dropped by the Go node:

```bash
DEVNET_BIN=<dir with celestia-appd> DEVNET_PORT_OFFSET=11000 conformance/devnet/p2p-interop.sh
```

Byte-level vectors (SecretConnection handshakes with fixed ephemeral keys,
MConnection wire bytes, NodeInfo, NetAddress, PEX, `node_key.json`,
`addrbook.json`) come from `conformance/p2pvectors` (fixture
`conformance/fixtures/p2p/go-vectors.json`):

```bash
cd conformance/p2pvectors
GOPATH=... GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run . -out ../fixtures/p2p/go-vectors.json
```

## Mempool interop (M10)

`mempool-interop.sh` checks the Rust CAT mempool and reactor (crate
`mempool`) against a Go celestia-core v0.42.3 validator. The Rust node
(`cargo run -p mempool --example mempool_interop`) runs the Rust state
machine in process with the Rust mempool calling it (CheckTx New/Recheck,
QuerySequence), dials the Go validator with the CAT reactor, advertises its
height with a consensus stub, and serves a small RPC (`broadcast_tx_sync`;
other requests are proxied to the Go node). A Go celestia-core follower only
drives block execution of the Rust app (the Rust node has no consensus yet).

- Phase A: the blob workload (`workloads/blob.sh`) and a burst of
  consecutive-sequence bank sends go to the Go node; every tx Go includes in
  a block must have reached the Rust mempool by gossip and passed the Rust
  CheckTx first.
- Phase B: a bank-send burst (JSON-RPC) and PFBs (`celestia-appd` CLI with
  `--node` at the Rust node) go to the Rust node only; each must be included
  in a Go-produced block.

Both mempools must drain, and the Go node must keep the Rust peer:

```bash
DEVNET_BIN=<dir with celestia-appd, cometbft> DEVNET_PORT_OFFSET=13000 conformance/devnet/mempool-interop.sh
```

## Rust state-sync node (ADR 0003: `light`, `statesync`)

`statesync-rust.sh [workload.sh]` runs a Rust state-syncing node
(`cargo run -p statesync --example statesync_node`: p2p `Switch` +
statesync reactor on 0x60/0x61 + light client against the validator's RPC +
the in-process Rust app) against a Go validator taking snapshots:

1. **Go → Rust:** the Rust node discovers the Go snapshots, light-verifies
   headers H, H+1, H+2 and the consensus params, fetches the chunks from the
   Go peer, restores them into the Rust app, verifies the app hash against
   the light-client-verified header and bootstraps State + Commit; the
   script also compares the restored app hash with the validator's block
   H+1 header.
2. **Rust → Go:** the Rust node snapshots the restored state and serves it
   through its statesync reactor; a fresh Go `celestia-appd` whose only peer
   is the Rust node state-syncs from it.

```bash
DEVNET_BIN=<dir with celestia-appd> DEVNET_PORT_OFFSET=14000 \
  conformance/devnet/statesync-rust.sh conformance/devnet/workloads/staking.sh
```

Light client outcomes are checked against celestia-core's `light` package
with vectors from `conformance/lightvectors` (fixtures
`conformance/fixtures/light/{verifier,client}.json.zst`):

```bash
cd conformance/lightvectors
GOPATH=... GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run . -out ../fixtures/light
```

## Block sync, state sync and the node (ADR 0003: `blocksync`, `evidence`)

`blocksync-interop.sh` runs `celestia-rs start` with P2P against a Go
`celestia-appd` validator producing `$BLOCKS` (default 200) blocks with the
blob workload and taking snapshots every `$INTERVAL` (50) blocks:

- **A. Go → Rust block sync:** a fresh Rust full node (only peer: the
  validator) block-syncs every block over 0x40 — each block verified with
  the next block's commit, `ProcessProposal` and executed by the in-process
  Rust app — then keeps following the chain. The script compares the
  node's app hash and block ID after every height with the validator's.
- **B. Rust → Go block sync:** a fresh Go node whose only peer is the Rust
  node block-syncs the whole chain from the Rust block store and must hold
  the validator's block hashes.
- **C. State sync → block sync → follow:** a fresh Rust node with
  `--statesync-rpc-servers` restores the validator's snapshot (light client
  against its RPC), bootstraps its stores, block-syncs from the snapshot
  height and follows; app hashes are checked as in A.

```bash
DEVNET_BIN=<dir with celestia-appd> DEVNET_PORT_OFFSET=16000 conformance/devnet/blocksync-interop.sh
```

Evidence verification outcomes (duplicate votes against historical
validator sets, lunatic / forward-lunatic / equivocation / amnesia light
client attacks, expiry, timestamps) are checked against celestia-core's
`evidence` package with vectors from `conformance/evidencevectors`
(fixture `conformance/fixtures/evidence/vectors.json`):

```bash
cd conformance/evidencevectors
GOPATH=... GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run . -out ../fixtures/evidence/vectors.json
```

## Mixed validator set (M9)

`mixed-validators.sh [heights] [heights-B]` runs Go `celestia-appd`
validators next to Rust `celestia-rs start` validators on one P2P network
(consensus reactor, `RECOVERY` block propagation, CAT mempool, block sync,
evidence):

- network A: 2 Go + 2 Rust validators (equal power), a Go and a Rust full
  node; bank / blob (up to 64x64 squares) / staking workload; one Rust
  validator is SIGKILLed and restarted mid-run (WAL replay, catch-up);
- network B: the Rust validators hold 80% of the power; both Go validators
  are stopped halfway and a Go full node keeps following.

Every node must agree on every block hash and app hash, Rust validators
must propose and sign their share, and no block may carry evidence.

```bash
DEVNET_BIN=<dir with celestia-appd> DEVNET_PORT_OFFSET=15000 conformance/devnet/mixed-validators.sh 100 30
```

The blob workload of network A is submitted through the Rust validator's
RPC, and the Go CLI queries it (`status`, `query block`, bank queries,
`net_info`, `catching_up`) as an extra check.

`double-sign-evidence.sh` runs 2 Go + 2 Rust validators and a byzantine
peer (`cargo run -p celestia-rs --example double_sign`) that sends two
conflicting prevotes of a Go validator to a Rust validator only. The Rust
node detects them, verifies the `DuplicateVoteEvidence` and gossips it;
the network commits it, the double signer is slashed out of the set, and
all block / app hashes still agree.

## Soak and chaos (`soak.sh`)

`soak.sh` runs a mixed network for `$SOAK_SECS` (default 7200) and at
least `$SOAK_MIN_HEIGHTS` (1500) heights: Go validators V0, V1, Rust
validators V2, V3, V4 (equal power), a Go full node F5 and a Rust full node
F6. Every P2P link goes through `soak/netproxy.py` (one listener per
directed node pair), so the orchestrator (`soak/soak.py`) can cut or slow
links without iptables.

- **Workload:** bank sends at volume (`soak/cosmostx.py` signs `MsgSend`s
  itself and broadcasts them to every node's RPC, `$SOAK_TX_RATE` tx/s),
  PFB rounds (medium blobs; every `$SOAK_BIG_EVERY` s a near-max round of
  8 x 1.9 MB or 3 x 7.3 MB blobs: 128x128 to 256x256 squares), staking
  churn (delegate / redelegate / unbond / withdraw on all validators), gov
  proposals (`MsgUpdateBlobParams`, passing and rejected).
- **Chaos** (one disruptive event at a time): SIGKILL + restart of Rust
  validators (WAL recovery; with and without block sync) and of the Rust
  full node, isolating any node, link latency, a 3/4 split that halts the
  chain until healed, a Rust node wiped (except
  `priv_validator_state.json`) and restarted from state sync off the Go
  snapshots, and V4 unbonding its whole self-delegation (leaves the set)
  then re-bonding + unjailing.
- **Checks:** block and app hash of every height on every node against
  the Go reference (`/commit`), no evidence in any block, no double-sign /
  conflicting-vote / consensus-failure / rejected-proposal log line, no
  stall over 60 s outside the planned split, every node converges after
  each event; RSS of every node every 15 s (`rss.csv`, per-lifetime growth
  slope in the report) and disk usage every 60 s (`disk.csv`).

```bash
DEVNET_BIN=<dir with celestia-appd> DEVNET_PORT_OFFSET=19000 conformance/devnet/soak.sh
```

`SOAK_PROFILE=memory` runs the same workload without chaos on 2 Go + 2
Rust validators and a Rust full node (every node keeps one lifetime) and
samples every node every `$SOAK_MEM_SAMPLE` s (default 30) into
`mem.jsonl`: RSS, threads, open files, disk, and the Prometheus memory
gauges (Rust `memstats_*`: RocksDB caches per database, IAVL node cache,
jemalloc heap with `--features memstats`; Go `go_memstats_*`).
`SOAK_RS_N<i>` / `SOAK_ENV_N<i>` run node i on another binary / with extra
environment (e.g. `MALLOC_CONF` heap profiling); `soak/memreport.py <dir>
--svg rss.svg` summarizes the run (`docs/benchmarks/memory.md`):

```bash
cargo build --release -p celestia-rs --features memstats   # optional: heap gauges
SOAK_PROFILE=memory SOAK_SECS=21600 DEVNET_BIN=<dir> DEVNET_PORT_OFFSET=21000 conformance/devnet/soak.sh
```

`SOAK_PROFILE=long` is a restart-tolerant multi-hour soak: V0, V1 Go and
V2, V3 Rust validators and a Rust full node F4; V2 rotates its WAL every
few blocks (`CELESTIA_RS_TEST_WAL_*` group limits); only the Rust nodes
take (background) snapshots, so every state sync, of a Rust or a Go
node, restores a Rust snapshot. On top of the chaos workload: authz /
feegrant, invalid-UTF-8 txs (`fuzz-diff craft`, CheckTx compared on a Go
and a Rust node), hyperlane + warp; chaos adds SIGTERM / SIGKILL of Go
validators, SIGTERM inside the delayed-precommit wait (sign state read
afterwards), 5-10 min downtimes (block sync) and Go state sync from Rust
snapshots; checks add WAL group sizes, privval refusals, total disk
(`$SOAK_DISK_SOFT_GB`: the largest node is state-synced afresh) and RSS
(`$SOAK_RSS_BUDGET_MB`). Counters live in `state.json` and every
reference hash in `hashes.jsonl`; after the machine restarts (every
process killed), `SOAK_RESUME=1` with the same `SOAK_DIR` restarts every
node from its home (a whole-network crash event) and continues until
`$SOAK_SECS` of cumulative run time:

```bash
SOAK_PROFILE=long SOAK_SECS=25200 SOAK_DIR=<dir> DEVNET_BIN=<dir> DEVNET_PORT_OFFSET=30000 conformance/devnet/soak.sh
SOAK_RESUME=1 SOAK_PROFILE=long SOAK_SECS=25200 SOAK_DIR=<dir> ... conformance/devnet/soak.sh   # after a restart
```

`crash-restart.sh` SIGKILLs every validator inside the delayed-precommit
window (all prevotes cast, no precommit) and restarts them: an all-Go
network (`CRASH_KINDS="go go go go"`, the default) stalls for good, as
does a mixed one. Peer-gossiped proposals and parts stay out of the WAL
(`only_internal_wal = true`), so only the proposer has the block after the
restart, and its propagation cache, which serves block parts, is not
rebuilt from the WAL; the others receive a polka for that block before
they reach the prevote step, their own nil prevote is refused by the
privval, and nothing schedules their prevote-wait timeout. The long soak
records such a stall after a machine restart and recovers by restarting
two stuck validators so that their peers' prevotes arrive after their
propose timeout.

`latency-stall.sh` puts 1.5 s of latency on one validator's links (all
through the soak proxy) and then removes it: an all-Go network leaves
that validator in the commit step of a height without the block's parts
for good (until a peer reconnects), as the long soak saw on a Rust
validator after 700 ms of latency. Wants that reach peers after they have
committed are not answered (normal wants only serve the current height),
and the propagation reactor never re-requests parts from a peer it has
already asked (`retryWants` subtracts outstanding requests, which only a
commit or a peer removal clears). The long soak records such a stall and
reconnects the node.

`statesync-traffic.sh` state-syncs fresh Go nodes (from Go and from Rust
snapshots) and a fresh Rust node while txs are broadcast to them: both Go
nodes panic on the first block after the snapshot (`collections: not
found ... ConsensusParams`): the multistore's inter-block cache keeps
what CheckTx read from the empty pre-restore stores, and the restore
reloads the stores without resetting it (cosmos-sdk store; a restart
recovers). The Rust node syncs and follows; the long soak sends no
traffic to a node while it state-syncs.

The report (`report.md` / `report.json`) lists heights, txs, block sizes,
proposers, restarts, max RSS and disk per node, stalls, chaos events and
the most frequent Rust warnings. `build-bins.sh` builds the Go binaries
(with download retries); the nightly workflow runs the interop scripts and
a 30-minute soak.

`workloads/utf8.sh` (scenarios `utf8` and, with `APP_V9=1`, `v9-utf8`)
sends txs whose strings carry invalid UTF-8 (ADR 0005). The CLI cannot
build them (it round-trips strings through JSON), so `fuzz-diff craft`
signs them with the devnet's test keyring (`SIGN_MODE_DIRECT` and
`SIGN_MODE_LEGACY_AMINO_JSON`); the workload broadcasts them over RPC:

```bash
cargo build --release -p fuzz-diff
conformance/devnet/devnet.sh utf8 30 conformance/devnet/workloads/utf8.sh
```
