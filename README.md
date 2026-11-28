# celestia-rs

`celestia-rs` runs a Celestia consensus node, an optional data availability
(DA) bridge and a standalone DA light node from one Rust binary. Its consensus
node is wire- and consensus-compatible with Go
[celestia-app](https://github.com/celestiaorg/celestia-app) and
[celestia-core](https://github.com/celestiaorg/celestia-core). The DA roles
interoperate with Go [celestia-node](https://github.com/celestiaorg/celestia-node)
over the DA peer and RPC protocols exercised by the
[mixed-node conformance suite](conformance/da/README.md).

[Operator website](https://celestiaorg.github.io/celestia-rs/) · Quickstart, DA bridge setup, performance charts and operator FAQ.

## Install

Install the release your network's channel recommends: `mainnet`, `mocha`
or `corto`.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/celestiaorg/celestia-rs/main/scripts/install.sh | sh -s -- --channel mainnet
```

Run the same command again to update. A channel moves to a newer release
once that release has run on the network before it. `main` is released
once a day; Corto gets each release at once, Mocha after 24 hours on Corto,
Mainnet after 24 hours on Mocha. To update automatically on Linux, install the daily
[auto-update timer](packaging/linux/celestia-rs-auto-update.service). See
[releasing](docs/releasing.md#automatic-releases-and-release-channels).


## Status

**celestia-rs is an early, experimental project under rapid development.**
Expect breaking changes and rough edges, and don't rely on it for production
yet. Feedback, bug reports and questions are very welcome: please open an
issue. For a node problem, attach the archive that `celestia-rs
debug-bundle` writes (see the
[operator guide](docs/operator-guide.md#9-day-to-day-operations)).

celestia-rs runs a validator on both Mocha and Mainnet, signing blocks
alongside the Go validators:

| Network | Validator |
|---|---|
| Mocha | [Mocha validator](https://mocha.celenium.io/validator/122597103?tab=proposed+blocks) |
| Mainnet | [Mainnet validator](https://celenium.io/validator/1499834894?tab=proposed+blocks) |

General mainnet recommendation for non-archival RPC nodes, full consensus
nodes and validators is pending qualification. See [ADR 0008](docs/adr/0008-mainnet-recommendation.md) for scope
and evidence requirements.

See the [milestones](MILESTONES.md) for qualification progress and the
[open questions](OPEN_QUESTIONS.md) for outstanding owner decisions.

## Goals

- **Faster than the Go consensus node:** see
  [Performance vs Go](#performance-vs-go).
- **Minimal dependencies:** every external crate is a potential supply chain
  attack vector, so we keep the dependency tree small. Long term, the goal is
  to keep reducing the number of external dependencies this repo relies on.

## Features

- **Full node and validator in Rust:** ABCI++ app, Tendermint BFT consensus
  (celestia-core semantics, incl. block propagation), P2P (secret
  connection, MConn, PEX), CAT mempool, block sync, state sync, evidence,
  Tendermint light client for state sync.
- **Interoperates with Go nodes:** peers, gossips blocks and txs, and reaches
  consensus in mixed Go + Rust validator sets.
- **App-hash parity with Go**, checked by replaying a corpus of devnet
  scenarios recorded from Go (staking, gov, authz / feegrant, vesting,
  blobs, fibre, IBC, hyperlane, evidence, upgrades), by following Mocha and mainnet, and by nightly
  differential fuzzing against Go.
- **Go-compatible operator surface:** CLI (`init`, `keys`, `genesis`, `tx`,
  `query`, …), Go keyrings, RPC, gRPC and REST responses checked against Go,
  pruning, Prometheus metrics, double-sign protection.
- **Embedded Fibre server (opt-in):** a validator serves Fibre blob shards
  from `celestia-rs start` itself (`[fibre] enable = true`), with TLS
  identity, gRPC health and Prometheus metrics; see the
  [operator guide](docs/operator-guide.md#fibre-server).
- **Built for modest hardware** (pruned node or validator: 8 CPU cores, 16 GB RAM, 500 GB NVMe SSD; archive: 4 TB; [`docs/hardware.md`](docs/hardware.md)):
  release archives for macOS and Linux, launchd and systemd units.

## Architecture

| Role | Command | What it does |
|---|---|---|
| Consensus full node or validator | `celestia-rs start` | Executes blocks and maintains application state. A validator also participates in consensus voting. |
| DA bridge | `celestia-rs start` with a bridge listener | Reads committed blocks from the local blockstore, indexes verified extended headers, and serves DA peer requests or authenticated DA JSON-RPC on the configured listeners. |
| DA light node | `celestia-rs da-light` | Runs in a separate process and home. It follows signed headers from a trust anchor, verifies randomly selected shares and stores sampled headers. |

```mermaid
flowchart TB
    consensusPeers["Rust or Go consensus peers"]
    daPeers["Other DA peers, including Go celestia-node"]
    clients["DA RPC clients"]

    subgraph start["celestia-rs start"]
        consensus["Tendermint consensus and app"]
        blocks[("Consensus blockstore")]
        bridge["Optional DA bridge"]
        consensus -->|Committed blocks| blocks
        blocks -->|Local reads| bridge
    end

    subgraph lightProcess["celestia-rs da-light"]
        light["Header verification and share sampling"]
        lightStore[("Sampled headers and pending coordinates")]
        light --> lightStore
    end

    consensusPeers <-->|Consensus P2P| consensus
    bridge -->|Extended headers, shares and proofs| light
    bridge <-->|DA P2P| daPeers
    daPeers <-->|DA P2P| light
    bridge -->|Authenticated JSON-RPC| clients
    light -->|Authenticated JSON-RPC| clients
```

The bridge-to-light connection uses DA libp2p. The light node can sample via
DA peers independently of the bridge shown here. See the
[operator guide](docs/operator-guide.md#3-quick-starts) for listener
configuration, data retention and trust inputs.

## Performance vs Go

The current operator benchmark target is [celestia-app v11.0.0](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0).
The [operator comparison protocol](docs/benchmarks/operator-sync.md) covers snapshot sync to tip, RAM and disk usage. In the [2026-11-26 Mainnet runs](docs/benchmarks/operator-sync-2026-11-26.md) on 8-vCPU VMs, celestia-rs reached a verified tip 4.9x and 4.1x as fast as Go (307 s and 329 s against 1,493 s and 1,361 s). Over a matched 600-block DA window, with both sides serving the same reads, celestia-rs used 65% less peak memory than celestia-app plus celestia-node (4.06 GB against 11.54 GB). The [1,000-block runs](docs/benchmarks/operator-block-range-2026-11-26.md) on 32-vCPU VMs measured 75% and 78% less peak process memory, 62% to 64% less disk and about 8x faster snapshot restore; there, slow snapshot discovery on Mainnet made time to tip 0.24x and 1.05x Go's. A discovery fix has landed and awaits a Mainnet rerun.

### Mac Mini M4 results

The November 25 run used release builds on the same Mac Mini M4 (16 GB
RAM, 256 GB SSD) with Go celestia-appd v11.0.0 and celestia-rs `9ebb74f`.
The recorded workload covered 511 heights and 386,980 transactions, with
blocks up to 28.2 MiB. Both implementations matched every recorded app
hash. See the [benchmark report](docs/benchmarks/replay-and-node-benchmarks.md#results-mac-mini-m4-2026-11-25-go-v1100)
for the method and caveats.

| | Go | Rust | |
|---|---|---|---|
| Block execution (FinalizeBlock + Commit) | 1,112 tx/s | 4,759 tx/s | 4.3x faster |
| CheckTx, new txs | 6,338 tx/s | 13,288 tx/s | 2.1x faster |
| State sync to first block | 47.7 s | 10.9 s | 4.4x faster |
| Block sync (511 heights) | 192.4 s | 128.7 s | 33% less time |
| Memory (RSS after block sync) | 2,148 MB | 1,260 MB | 41% less |
| App state on disk after replay | 443 MB | 307 MB | 31% smaller |
| PrepareProposal, max-size squares (median) | 221.1 ms | 100.1 ms | 2.2x faster |
| ProcessProposal, max-size squares (median) | 213.5 ms | 56.8 ms | 3.8x faster |

## Getting started

Install a release ([above](#install)), or build from source:

```
cargo build --locked --release -p celestia-rs --features da-light
```

Release builds from v0.1.0-rc.8 include both experimental DA roles. The
`da-light` build feature includes the bridge, and `start` enables each DA
listener only when configured.

- [Run a consensus node](docs/operator-guide.md#3-quick-starts) or follow the
  validator runbooks for [Mocha](docs/operator-guide.md#11-running-a-mocha-validator-on-a-mac-m13)
  and [mainnet](docs/operator-guide.md#12-mainnet-validator-m14).
- [Run an integrated DA bridge](docs/operator-guide.md#experimental-da-bridge-on-a-private-network).
- [Run a standalone DA light node](docs/operator-guide.md#standalone-da-light-node).

## Development

```
cargo test --workspace
cargo xtask repo-health --offline   # local gate
cargo xtask refresh-fixtures        # regenerate Go fixtures (needs Go)
```

The [static analysis workflow](.github/workflows/static-analysis.yml) runs
Rust CodeQL scanning and Kani proofs for the `shares` crate. CodeQL findings
appear under Security → Code scanning. To run the same Kani proofs locally on
Linux or macOS:

```sh
cargo install --locked kani-verifier --version 0.68.0
cargo kani setup
cargo kani -p shares
```

`crates/` holds the workspace, tiered from primitives up to the node
binary; `conformance/` the Go fixture generators and devnet harness;
`docs/` the [design](docs/specs/2026-05-14-celestia-rs-design.md),
[ADRs](docs/adr/), benchmarks and [security notes](docs/security.md).

Reliability metrics, coverage budgets and fuzz evidence:
[`docs/quality.md`](docs/quality.md).

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
