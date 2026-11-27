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

Rust and Go sync time, memory and disk measurements:
<https://celestiaorg.github.io/celestia-rs/benchmarks/README.md>.
Mainnet cloud measurements against celestia-app v11.0.0 are pending.

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
the [quickstart](https://celestiaorg.github.io/celestia-rs/#start). The
[operator guide](https://celestiaorg.github.io/celestia-rs/operator-guide.md)
covers configuration, validators, the DA bridge, services and upgrades.

## More

- [Node census](https://celestiaorg.github.io/celestia-rs/census/): Mainnet and Mocha nodes by implementation
- [Operator FAQ](https://celestiaorg.github.io/celestia-rs/#faq)
- [Source code](https://github.com/celestiaorg/celestia-rs)
- [Report an issue](https://github.com/celestiaorg/celestia-rs/issues)
