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
