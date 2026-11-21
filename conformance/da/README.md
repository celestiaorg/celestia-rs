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
export DA_TEST_READ_TOKEN=$(celestia-rs da-auth read --home /tmp/da-local)
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

## Standalone Rust DA light mode

Build the same executable with `--features da-light`. Get a recent header hash
from a trusted source and a TCP multiaddress with the DA peer ID of a bridge
on that chain. Use a fresh light home:

```sh
celestia-rs da-light --home ./light-home --network "$CHAIN_ID" \
  --peer "$BRIDGE_MULTIADDR" --trusted-hash "$HEADER_HASH"
```

The process verifies the checkpoint, follows adjacent signed headers and
samples up to 16 unique random cells per height. It accepts row and column
proofs, including parity cells. Its separate light database records the
coordinates before sampling and advances the head after every proof passes.
Rerunning the command with the original trust inputs resumes from its last
sampled height. A header outside the seven-day trusting period requires a
fresh checkpoint and light home. Use `--until HEIGHT` for a bounded run.

Add `--rpc-listen 127.0.0.1:26658` to `da-light` to read the sampled head,
tail, or headers by height and hash over HTTP JSON-RPC. Create a read token
with `celestia-rs da-auth read --home "$LIGHT_HOME"`. The light store indexes
a header after verifying its selected shares, and retains headers for the Go
storage window of seven days plus one hour, measured from the sampled head.
A home created by an earlier light build starts its history at its saved head.

### Private non-validating bridge qualification

Run `conformance/da/light/test-go-bridge.sh RUST_BINARY GO_APP_TREE GO_NODE_TREE` on an isolated host using a Rust build with `--no-default-features --features da-light`. The fixture checks the Go revisions before startup. A fresh Rust consensus peer receives the Go genesis, syncs from the sole validator, and serves paid data over authenticated RPC and SHREX with zero voting power.

On Linux the fixture also prints a JSON receipt for concurrent verified reads of the same paid data through both bridges. It records consensus height before and after the workload, completed read suites per bridge, maximum sampled lag, process CPU time during the workload, process lifetime peak RSS, and final allocated home sizes. Go runs a validator and a separate bridge process. Rust runs a non-validating consensus peer and its bridge in one process. These short private runs exercise progress under reads. Hardware, build profiles and process roles affect the resource figures.
