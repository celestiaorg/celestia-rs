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
