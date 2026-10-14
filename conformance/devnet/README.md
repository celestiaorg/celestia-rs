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
