# celestia-rs: Celestia consensus node and data availability bridge in Rust

Follow Celestia, verify blocks, and serve data availability proofs from one
process. Run a non-validating Celestia consensus node on Mainnet.

celestia-rs is an early, experimental project. Expect breaking changes, and do
not rely on it for production yet.

This is the Markdown version of <https://celestiaorg.github.io/celestia-rs/>.
An index of all agent-readable pages is at
<https://celestiaorg.github.io/celestia-rs/llms.txt>.

## Inside the node

A Go bridge operator typically runs `celestia-appd` for consensus and a
separate `celestia-node` bridge connected to it. With bridge listeners
enabled, `celestia-rs start` performs both jobs.

- **Consensus and execution:** run a full node or sign as a validator. The
  ABCI++ application executes transactions. Consensus, block propagation, peer
  exchange, evidence handling and the CAT mempool interoperate with Go peers.
- **Data availability bridge:** publish extended headers, reconstruct data
  squares and answer namespace proof and blob requests. The DA JSON-RPC and
  libp2p listeners are opt-in.
- **State and recovery:** block sync, state sync from a trusted snapshot, and
  `celestia-rs follow` to verify application execution over HTTPS RPC.
- **Light node and sampling:** `celestia-rs da-light` samples data
  availability from a trusted DA peer.
- **Application compatibility:** Go replay fixtures check app hashes across
  the application modules. Nightly differential fuzzing complements
  mixed-validator devnets.
- **Operator tooling:** Go-compatible keys, keyrings, genesis, query and tx
  commands; CometBFT RPC, gRPC and REST; Prometheus metrics; systemd and
  launchd service files.

## Benchmarks

Mainnet runs on 26 Nov 2026 against celestia-app v11.0.0 on identical cloud
VMs: celestia-rs reached a verified tip 4.9x and 4.1x as fast, restored a
snapshot about 8x as fast, used 65% less peak memory with its DA bridge
than celestia-app plus celestia-node, and its node home was 62% smaller after
1,000 blocks. Reports:
<https://celestiaorg.github.io/celestia-rs/benchmarks/operator-sync-2026-11-26.md>
and
<https://celestiaorg.github.io/celestia-rs/benchmarks/operator-block-range-2026-11-26.md>.
See the [benchmark index](https://celestiaorg.github.io/celestia-rs/benchmarks/README.md)
for the comparison protocol and earlier results.

## Hardware requirements

| Profile | CPU | Memory | Disk |
|---|---|---|---|
| Pruned full node | 8 CPU cores | 16 GB | 500 GB NVMe SSD |
| Validator | 8 CPU cores | 16 GB | 500 GB NVMe SSD |
| Archive node | 8 CPU cores | 16 GB | 4 TB NVMe SSD, expandable |

Details: <https://celestiaorg.github.io/celestia-rs/hardware.md>.

## Start on Mainnet

Joining Mainnet and syncing blocks need no wallet, TIA, stake or fees. Release
binaries run on Apple silicon macOS and on x86-64 or ARM64 Linux with glibc
2.35 or newer.

Install the release that the `mainnet` channel recommends (use
`--channel mocha` for Mocha):

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/celestiaorg/celestia-rs/main/scripts/install.sh | sh -s -- --channel mainnet
~/.celestia-rs/bin/celestia-rs version --long
```

Then start from a verified state-sync snapshot with the first-launch block in
the [quickstart](https://celestiaorg.github.io/celestia-rs/#start), which
has options for Mainnet or Mocha, full node or validator, and pruned or archive. The
[operator guide](https://celestiaorg.github.io/celestia-rs/operator-guide.md)
covers configuration, validators, the DA bridge, services and upgrades.

## Enable the integrated Rust DA bridge

With the bridge enabled, the consensus node serves DA data from its blockstore.
This replaces a separate Go `celestia-node` bridge. Start a non-validating Mocha
node with the quickstart, stop it, then restart with the same home and the DA
listeners enabled.

```sh
~/.celestia-rs/bin/celestia-rs start --network mocha --home ~/.celestia-rs/mocha \
  --bridge-rpc-listen 127.0.0.1:26658 --bridge-p2p-listen 0.0.0.0:2121
```

DA RPC stays on loopback. Allow inbound TCP on port 2121 for the DA peers you
want to serve. After the node catches up, use its DA peer address to
[verify reads from a Go light node](https://github.com/celestiaorg/celestia-rs/blob/main/docs/operator-guide.md#mocha-5-non-validating-bridge-canary).

## More

- [Node census](https://celestiaorg.github.io/celestia-rs/census/): Mainnet and Mocha nodes by implementation
- [Operator FAQ](https://celestiaorg.github.io/celestia-rs/#faq)
- [Source code](https://github.com/celestiaorg/celestia-rs)
- [Releases](https://github.com/celestiaorg/celestia-rs/releases)
- [Report an issue](https://github.com/celestiaorg/celestia-rs/issues)
