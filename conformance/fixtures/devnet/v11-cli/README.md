# v11 candidate ABCI replay

This recording uses the signed transactions, timestamps and validator set of
the `cli` devnet fixture (chain ID `rs-devnet-1`). InitChain selects app
version 11, as do the app versions in the block headers supplied to ABCI.
The block hashes and commits come from the original fixture. These requests
exercise the application on synthetic headers. Full-node consensus, network
freeze lists and upgrade migration require separate checks.

Both official Linux x86-64 standalone releases at celestia-app commit
`58be8ae90d00518800df5506b3a1e988f009c1c6` produced the same recorded
exchanges. The recording includes InitChain, CheckTx, PrepareProposal,
ProcessProposal, FinalizeBlock and Commit. It excludes the binary's Info
version string. The 40 blocks include 11 proposals with transactions.

| Release | Archive SHA-256 | Extracted celestia-appd SHA-256 |
| --- | --- | --- |
| [v11.0.0-corto](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0-corto) | `7804c1f1546e7786ca8402b4201ef377cea86a33a2800d1d1b403d00024394ec` | `bd4d4ced675de95e4c98418204d2fd6b4832333d1c23aefe1812f2c1e23a9588` |
| [v11.0.0-mocha](https://github.com/celestiaorg/celestia-app/releases/tag/v11.0.0-mocha) | `addc971635bc91bb666d8a05f90d5b7db7983f993ea5b0540eb93edb7399676d` | `8ee28e467e6a53d4d36216825a567d97a9208fb20a1613ac77eb12ec7db4e689` |

A standalone Go 1.26.10 source build of `v11.x` at
`8d5146e046b6c680639bd5f39a194941ea3b9078` also produced the identical
recording. Its executable SHA-256 was
`21c392808d5b53862c97ff82499b5a0a5b2d6461b0e1ee0a0e201ef16226d408`.
The three captures have SHA-256
`5ceacc180b1939e7cddc08172a3a6bbd486756f0efa8c65fd9dd4f65c0b3cbf5`.

To regenerate, verify the archive against the release's `checksums.txt`.
From the repository root, initialize a fresh home and start the binary:

```sh
probe_home=$(mktemp -d)
celestia-appd init replay --chain-id rs-devnet-1 --home "$probe_home"
celestia-appd start --force-no-bbr --with-comet=false --grpc.enable=false \
  --address tcp://127.0.0.1:28761 --home "$probe_home"
```

From another terminal at the repository root:

```sh
cargo run --locked -p devnet-conformance --no-default-features \
  --example record_v11_cli -- 127.0.0.1:28761 /tmp/v11-cli.rec
cmp /tmp/v11-cli.rec conformance/fixtures/devnet/v11-cli/abci.rec
cargo test --locked -p devnet-conformance --no-default-features \
  --test replay_scenarios v11_cli_candidate_parity
```
