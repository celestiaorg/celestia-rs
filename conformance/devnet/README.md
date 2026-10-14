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
ProcessProposal / CheckTx on blob blocks proposed by Go.

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
