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

For SHREX and go-header, build with `--features bridge-p2p` and add
`--bridge-p2p-listen 127.0.0.1:32121` to the Rust start command. Use the
public peer ID in the DA listener log to construct the multiaddress. From
the pinned celestia-node checkout, run the peer probe against the block
containing the paid blob:

```sh
go run "$PROBE_DIR/peer/read_peer.go" celestia-rs-local \
  "/ip4/127.0.0.1/tcp/32121/p2p/$PEER_ID" "$HEIGHT" 726f6c6c7570 68656c6c6f2d6461
```

The Go peer exchanges a signed header, reconstructs the EDS, verifies a
range, verifies half rows and samples in all four quadrants, and retrieves
the submitted bytes with a namespace inclusion proof.

Run `go run /absolute/path/to/conformance/da/historical/roots.go` from the pinned
celestia-app v11.0.0 checkout to regenerate the historical data roots used by
`go_historical_bridge_controls`. The signed blobs span the first-share capacity
boundary where the square builders used in app versions 5 and 6 diverge.

An enabled bridge listener opens a verified DA index in `data/da-bridge`. The
index follows committed consensus heights in order, verifies the block's own
commit, validator set and data root, and atomically saves its extended header,
original square and cursor. `header.LocalHead` reports the indexed height.
Use `--bridge-archival` on a fresh DA index to retain every indexed square.
The default retention window is seven days plus one hour. Restarting an archival
index in pruning mode permits removal of expired data. A pruned index needs a
fresh resync to become archival.
An index first opened on a pruned consensus store starts at its retained base.
