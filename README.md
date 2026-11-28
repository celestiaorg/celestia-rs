# celestia-rs

One Rust binary runs a Celestia consensus full node or validator, serves data
availability (DA) requests as an integrated bridge, or samples shares as a
standalone light node. Rust validators sign alongside Go validators on
[Mainnet](https://celenium.io/validator/1499834894?tab=proposed+blocks) and
[Mocha](https://mocha.celenium.io/validator/122597103?tab=proposed+blocks).

The project is experimental. General Mainnet qualification for non-archival
RPC nodes, full nodes and validators is still open.
[Qualification milestones](MILESTONES.md) · [Open questions](OPEN_QUESTIONS.md)

## Install

Install the release recommended for your network on Linux or Apple silicon macOS.
Choose `mainnet`, `mocha` or `corto`.

```sh
curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/celestiaorg/celestia-rs/main/scripts/install.sh | sh -s -- --channel mainnet
```

Run the command again to update. New daily releases land on Corto first.
After 24 hours on Corto, a release becomes eligible for Mocha. After another
24 hours on Mocha, it becomes eligible for Mainnet. [Release channels](docs/releasing.md#automatic-releases-and-release-channels)

<a id="architecture"></a>

## Run a node

| Role | How to run it |
|---|---|
| Consensus full node | `celestia-rs start --network mocha` initializes a home and joins Mocha. [Quick starts](docs/operator-guide.md#3-quick-starts) |
| Validator | The consensus node votes with a local signing key and persistent double-sign protection. [Validator setup](docs/operator-guide.md#4-setting-up-a-validator-from-scratch) |
| Experimental DA bridge | Configure DA listeners on the consensus node to serve verified headers and shares. [Bridge setup](docs/operator-guide.md#experimental-da-bridge-on-a-private-network) |
| Experimental DA light node | Run `celestia-rs da-light` in its own home to verify signed headers and sample shares. [Light node setup](docs/operator-guide.md#standalone-da-light-node) |

A pruned node or validator targets **8 CPU cores, 16 GB RAM and 500 GB of NVMe
storage**. Track commits, peers, mempool load and validator signing in Grafana.
[Hardware sizing](docs/hardware.md) · [Linux dashboard](packaging/linux/monitoring/README.md) · [macOS dashboard and alerts](packaging/macos/monitoring/README.md)

## Performance vs Go

In two cold Mainnet syncs on 8-vCPU VMs, celestia-rs reached an independently
verified tip in **307 and 329 seconds**, versus **1,493 and 1,361 seconds** for
celestia-app v11.0.0. Over the next matched 600-block DA workload, the Rust
node and bridge used **65% less peak cgroup memory** while serving the same
shares. [Paired Mainnet measurements](docs/benchmarks/operator-sync-2026-11-26.md)

Snapshot discovery can dominate sync time. In separate 1,000-block runs on
32-vCPU VMs, Rust restored snapshots about 8× faster, but time to verified tip
ranged from 18.6 vs 4.4 minutes to 16.9 vs 17.7 minutes (Rust vs Go).
[Block-range measurements](docs/benchmarks/operator-block-range-2026-11-26.md)

## Development

```sh
cargo build --locked --release -p celestia-rs --features da-light
cargo test --workspace
cargo xtask repo-health --offline
```

[Operator website](https://celestiaorg.github.io/celestia-rs/) · [Operator guide](docs/operator-guide.md) · [Go conformance tests](conformance/README.md) · [Quality and fuzzing](docs/quality.md) · [Security](docs/security.md)

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
