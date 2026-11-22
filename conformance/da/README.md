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

To accept paid submissions, start the bridge with an additional
`--bridge-signer <account>`. This selects a funded account from the node's
configured keyring. A client with a write token can spend that account's funds
on blob submissions. The usual read token can query its address. For a private
devnet, the submission probe sends both v0 and signer-bearing v1 blobs and
verifies their commitment proofs at the returned height.

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

With the DA peer enabled, `--bridge-p2p-listen` selects the TCP interface and
port. Mainnet and Mocha 5 use the bootstrap addresses from the pinned Go DA
releases. Private networks have an empty default bootstrap list. Repeat
`--bridge-bootstrap <multiaddress>/p2p/<peer-id>` to supply an explicit list.
For a node behind NAT, repeat `--bridge-announce <multiaddress>` with its
reachable TCP addresses. The DA RPC listener is configured independently.

## Standalone Rust DA light mode

Build the same executable with `--features da-light`. A fresh
`da-light --network celestia` or `da-light --network mocha-5` requests signed
heads concurrently from the release's trusted bootstrap peers. The node uses
the go-header v0.8.10 agreement threshold and falls back to the highest valid
trusted response if the peers do not reach agreement. Both modes persist the
initial trust anchor across restarts.

The light node also queries the network-scoped Kademlia DHT for providers
advertising the Go `/full/v0.1.0` topic. It dials up to 16 discovered peers
for header exchange and SHREX. Only configured trusted peers participate in
initial checkpoint selection. Every fetched header and sample is verified
against the stored trust chain before it advances the light head.

To use an external checkpoint and an explicit peer on a private chain, run
the following command with a fresh light home. Repeat `--peer` for additional
trusted peers.

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

The same light RPC listener supports authenticated Go WebSocket clients.
Header subscriptions advance as the node verifies and samples each height.
Historical blob subscriptions start at a retained sampled header and verify
the square from a DA provider before delivering the namespace's blobs.
The private fixture replays its paid blob, cancels the subscription and
checks the next two live headers through the released Go client.

The light peer serves retained sampled headers through the chain's
go-header exchange. To accept a new incoming TCP connection, supply a
listen multiaddress such as `--p2p-listen /ip4/127.0.0.1/tcp/39621`.
The private fixture disconnects the Go bridge, connects a fresh Go
exchange client directly to the Rust light peer and verifies the paid
checkpoint by height and hash, its next two signed headers and the head.

For proof-verified light balance reads, supply the Go core gRPC endpoint with
`--core.ip HOST` and optionally `--core.port PORT` (default 9090).
Use `--core.tls` to verify the server with the system trust store. A protected
TLS endpoint can receive its X-Token from `--core.xtoken.path DIRECTORY`.
A read token permits `state.BalanceForAddress` to query the utia balance
at the height preceding the sampled head. The node checks the bank proof and
multistore proof against that head's app hash before returning the amount.
The private fixture checks a funded account, a proven absent account and
rejection of a changed core response over an authenticated TLS connection.

Build the same executable with `--features da-fibre` to expose
`fibre.Download` on the authenticated light RPC listener. Configure a
Fibre-capable core with the light role's `--core.ip` flags. The client uses
the configured core for validator and provider discovery, verifies the
provider's rows, and reconstructs the blob for the requested commitment.
The provider must retain the blob. The request accepts the Go JSON encoding
of a blob ID and returns the recovered bytes in the Go result's `data` field.

To exercise the complete read path privately, pass the pinned celestia-node
checkout, `--rust-node`, and the absolute path of the feature-enabled executable
to `conformance/da/fibre/test-go-provider.sh`. Its Go node service uploads
random bytes to a registered local FSP. A released Go RPC client reads them
from the Rust light listener and checks missing and malformed commitments.

In bridge mode the same Fibre client discovers providers through the node's
own application gRPC listener. Build with `--features da-fibre`, enable app
gRPC and supply `--bridge-rpc-listen` at startup. The bridge admits one Fibre
download at a time. Run `conformance/da/fibre/test-bridge-download.sh` with
the binary and pinned Go checkout to start a private Rust chain, register a
Go FSP, deposit escrow, upload through the Go service and read the same bytes
through the Rust bridge.
