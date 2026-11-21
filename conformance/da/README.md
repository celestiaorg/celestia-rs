# Go client against a Rust bridge

Build the operator binary with `cargo build -p celestia-rs --features bridge`.
Start a private single-validator network using a fresh home:

```sh
target/debug/celestia-rs start --network local --home /tmp/da-local --bridge-rpc-listen 127.0.0.1:32658
```

Run the probe from a checkout of celestia-node at v0.36.0 (or v0.36.0-mocha).
Set `PROBE` to the absolute path of `read_bridge.go` in this directory.
After the first commit:

```sh
go run "$PROBE" http://127.0.0.1:32658
```

The probe uses the release's JSON-RPC client. It validates the extended
header with the Go implementation, retrieves the same header by height,
and reads the first share. This exercises the DA HTTP listener on the
same process that commits the block. Peer-to-peer exchange has its own
interoperability test requirement in issue #455.

For a blob round trip, submit a paid v0 blob to that devnet and record
the height returned by the CLI:

```sh
celestia-rs tx blob pay-for-blob 726f6c6c7570 68656c6c6f2d6461 \
  --home /tmp/da-local --chain-id celestia-rs-local --from validator \
  --keyring-backend test --fees 50000utia --node tcp://127.0.0.1:26657 --wait
go run "$PROBE" http://127.0.0.1:32658 "$HEIGHT" 726f6c6c7570 68656c6c6f2d6461
```

The client recomputes the blob commitment, verifies the namespace rows against
the signed header, and compares the data and share index returned by
`blob.Get` and `blob.GetAll`.
