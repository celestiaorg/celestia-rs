# celestia-rs

A Rust implementation of a Celestia consensus node, wire- and
consensus-compatible with Go [celestia-app](https://github.com/celestiaorg/celestia-app)
and [celestia-core](https://github.com/celestiaorg/celestia-core). One
binary, `celestia-rs`, runs the state machine, Tendermint consensus, P2P,
mempool, RPC and gRPC.

## Install

```sh
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/celestiaorg/celestia-rs/main/scripts/install.sh | sh
```


## Status

**celestia-rs is an early, experimental project under rapid development.**
Expect breaking changes and rough edges, and don't rely on it for production
yet. Feedback, bug reports and questions are very welcome: please open an
issue.

celestia-rs runs a validator on both Mocha and Mainnet, signing blocks
alongside the Go validators:

| Network | Validator |
|---|---|
| Mocha | [Mocha validator](https://www.mintscan.io/celestia-testnet/validators/celestiavaloper13v73gcz3k923ajt23alr5h3wuvsgqxu3jag0sd) |
| Mainnet | [Mainnet validator](https://www.mintscan.io/celestia/validators/celestiavaloper16v6qxgj2w996jt49tngf9fkvxw6j7u0uh6d9ec) |

Roadmap and progress: [`MILESTONES.md`](MILESTONES.md). Open decisions:
[`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md).

## Goals

- **Compatible with Celestia Mainnet:** run alongside Go
  [celestia-app](https://github.com/celestiaorg/celestia-app) and
  [celestia-core](https://github.com/celestiaorg/celestia-core) nodes on the
  existing network, with matching app hashes and wire formats.
- **Faster than the Go consensus node:** see
  [Performance vs Go](#performance-vs-go).
- **Minimal dependencies:** every external crate is a potential supply chain
  attack vector, so we keep the dependency tree small. Long term, the goal is
  to keep reducing the number of external dependencies this repo relies on.

## Features

- **Full node and validator in Rust:** ABCI++ app, Tendermint BFT consensus
  (celestia-core semantics, incl. block propagation), P2P (secret
  connection, MConn, PEX), CAT mempool, block sync, state sync, evidence,
  light client.
- **Interoperates with Go nodes:** peers, gossips blocks and txs, and reaches
  consensus in mixed Go + Rust validator sets.
- **App-hash parity with Go**, checked by replaying a corpus of devnet
  scenarios recorded from Go (staking, gov, authz / feegrant, vesting,
  blobs, fibre, IBC, hyperlane, evidence, upgrades), by following Mocha and mainnet, and by nightly
  differential fuzzing against Go.
- **Go-compatible operator surface:** CLI (`init`, `keys`, `genesis`, `tx`,
  `query`, …), Go keyrings, RPC, gRPC and REST responses checked against Go,
  pruning, Prometheus metrics, double-sign protection.
- **Built for a Mac Mini M4** (16 GB / 512 GB, [`docs/hardware.md`](docs/hardware.md)):
  release archives for macOS and Linux, launchd and systemd units.

## Performance vs Go

Release builds of both on the same machine, same recorded heavy chain (513
heights, 257k txs, blocks up to 27.9 MiB). Linux cloud VM; the reference
run on the Mac Mini is pending. Full method and caveats:
[`docs/benchmarks/`](docs/benchmarks/README.md).

| | Go | Rust | |
|---|---|---|---|
| Block execution (FinalizeBlock + Commit) | 225 tx/s | 802 tx/s | 3.6x faster |
| CheckTx, new txs | 971 tx/s | 2,007 tx/s | 2.1x faster |
| State sync to first block | 60.3 s | 16.3 s | 3.7x faster |
| Block sync | 1.1 blocks/s | 1.3 blocks/s | 1.2x faster |
| Memory (RSS after block sync) | 1,860 MB | 1,264 MB | 32% less |
| App state on disk | 329 MB | 549 MB | 1.7x larger ([ADR 0006](docs/adr/0006-compact-iavl-storage.md)) |

Since that run, parallel SIMD erasure coding, a signature cache and
libsecp256k1 cut PrepareProposal on max-size squares from ~1.1x Go's
latency to ~0.65x and ProcessProposal to ~0.25x.

## Getting started

Install a release ([above](#install)), or build from source:

```
cargo build --release -p celestia-rs
```

Running a node or a validator on Mocha or mainnet:
[`docs/operator-guide.md`](docs/operator-guide.md) (§11 is the Mocha
validator runbook).

## Development

```
cargo test --workspace
cargo xtask repo-health --offline   # local gate
cargo xtask refresh-fixtures        # regenerate Go fixtures (needs Go)
```

`crates/` holds the workspace, tiered from primitives up to the node
binary; `conformance/` the Go fixture generators and devnet harness;
`docs/` the [design](docs/superpowers/specs/2026-05-14-celestia-rs-design.md),
[ADRs](docs/adr/), benchmarks and [security notes](docs/security.md).

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
