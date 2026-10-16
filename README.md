# celestia-rs

A Rust implementation of a Celestia consensus node, wire- and
consensus-compatible with Go [celestia-app](https://github.com/celestiaorg/celestia-app)
and [celestia-core](https://github.com/celestiaorg/celestia-core). One
binary, `celestia-rs`, runs the state machine, Tendermint consensus, P2P,
mempool, RPC and gRPC.

## Status

**celestia-rs is validating on the Mocha testnet.** Validator
[`celestiavaloper13v73gcz3k923ajt23alr5h3wuvsgqxu3jag0sd`](https://www.mintscan.io/celestia-testnet/validators/celestiavaloper13v73gcz3k923ajt23alr5h3wuvsgqxu3jag0sd)
runs this code on a Mac Mini M4. It state-synced from Mocha's Go peers over
P2P, is at the tip of the chain, and is signing blocks alongside the Go
validators with no app-hash mismatch.

| | |
|---|---|
| Go parity target | celestia-app v10.4.0 / celestia-core v0.42.x, plus app version 9 (v9.0.8, mainnet today) and the v9 → v10 upgrade |
| Mocha | Validating (M13: 24 h signing soak) |
| Mainnet | Follows the tip with matching app hashes; full node next, then a mainnet validator (M14) |
| Not yet | Production use on mainnet. Treat it as experimental. |

Roadmap and progress: [`MILESTONES.md`](MILESTONES.md). Open decisions:
[`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md).

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
