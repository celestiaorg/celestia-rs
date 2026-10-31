# Empty ABCI Query envelope over public Comet transports

The current-release capture in `../v10.5.0/empty-query.json` contains 28 exchanges
against an actual isolated embedded celestia-app v10.5.0 node: nine P2P-query paths
through URI HTTP, JSON-RPC HTTP and WebSocket, plus a JSON-RPC batch containing a
successful health request before and after an unknown filter query.

An unknown P2P filter type (`/p2p/filter/other/x`, an empty type, and the same path
with an extra component) produces a nil application Query response. Go's RPC
handler recovers its dereference and returns:

```json
{"jsonrpc":"2.0","id":-1,"error":{"code":-32603,"message":"Internal error","data":"runtime error: invalid memory address or nil pointer dereference"}}
```

Both HTTP forms use status 500. The original JSON-RPC request ID is replaced by
`-1`; the batch is replaced by the single error, including its earlier successful
entry. WebSocket returns the same error text and ID in a text frame. In every
capture the same HTTP connection or WebSocket session subsequently answers
`health` successfully. Valid `addr` and `id` filters return a present successful
Query with JSON `value: null`; short paths and unknown commands return SDK code 6
inside a successful RPC result.

Rust models the absent envelope and recovery outcome explicitly. It does not
panic a task to construct this response. The ABCI socket adapter also preserves
an absent envelope and can continue with the following Flush and Query. The
in-process `App::query` helper requires a concrete response and returns an
explicit SDK error for the absent case; wire adapters use `query_response` or
`handle_envelope`. Cosmos gRPC's own ABCIQuery wrapper is outside this comparison.

`empty-query-meta.json` records source/module checksums and origin commits,
compiler, standalone ledger build tags/linker stamps, binary/capture/config/genesis
hashes, the local chain ID, scope and exclusions. This is a source-built standalone
oracle; equivalence to the official multiplexer binary is not claimed. The compiler is **Go 1.26.9**, matching the repository's current
oracle pin; this differs from the tracking issue's original Go 1.26.6 observation.
The oracle release is v10.5.0, independently observed active on mainnet on
2026-10-30 before this capture. These stateless default peer-filter routes do not
consult committed application state or height. No custom peer-filter callbacks,
proxy/TLS behavior, subscriptions or resource-load scenarios are established.

HTTP status, response JSON bytes, Content-Type and Cache-Control presence/value
are compared exactly. Date and X-Server-Time values are dynamic; server-time
presence is checked. WebSocket response opcode/payload bytes and subsequent
health frames are compared; raw frames and handshake headers remain in the
capture. Handshake accept values depend on the client's key. Transport failures
are represented by their exception type/message, never by a successful response.

To regenerate a fresh isolated scenario and its provenance, use an empty output
directory (the script first verifies that v10.5.0 is still active on mainnet):

```sh
DEVNET_PORT_OFFSET=1300 conformance/mainnet-rpc/empty-query/record.sh /tmp/empty-query-capture
```

The script requires the provenance-aware standalone oracle builder introduced in
[PR 197](https://github.com/celestiaorg/celestia-rs/pull/197); it rejects old
module-only stamps and verifies the tagged binary's content hash. It builds the
pinned oracle, starts two temporary local Go validators,
collects responses and source/config/binary hashes, then stops both nodes and
removes their temporary homes. Random genesis/key bytes vary across captures;
these stateless requests and the default peer-filter behavior are reproducible.

To capture an already running isolated node, use explicit loopback addresses:

```sh
python3 conformance/mainnet-rpc/empty-query/record.py \
  --rpc http://127.0.0.1:27657 --binary /path/to/celestia-appd --out /tmp/empty-query.json
```

The recorder uses Python's standard library with three-second socket timeouts and
one-MiB response/frame limits. It sends no transactions. Start an isolated Go
oracle with `conformance/devnet/build-bins.sh` and the `make_net`/`start_node`
helpers in `conformance/devnet/lib-mixed.sh`; set every listener to loopback and
use a fresh temporary home and distinct `DEVNET_PORT_OFFSET`. Record the exact
module/compiler/binary and input hashes for each fresh capture rather than
relabeling this artifact.

Run the Rust real-socket comparisons with:

```sh
cargo test --locked -p rpc --no-default-features --test empty_query
cargo test --locked -p devnet-conformance --no-default-features --lib
```

The latter also guards recorder integrity and real App ABCI envelope/Flush reuse.
Legacy corpora whose recorder fabricated a default Query for the unknown filter
are counted as skipped; the fresh empty-envelope cases have explicit comparisons.
