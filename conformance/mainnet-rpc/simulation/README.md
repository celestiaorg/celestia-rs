# Current-release Simulate error contract

This fixture covers [#185](https://github.com/celestiaorg/celestia-rs/issues/185)
using actual Cosmos gRPC calls to a fresh, isolated celestia-app v10.5.0
standalone node. It contains new Go responses. Only the public genesis and
ABCI request stream from the historical `empty` scenario are reused; recorded
v10.4 answers are ignored during capture.

The oracle is app commit `6013e758d5dceb75827ad191c8d739d7c43cfa04`, core
v0.42.3 at `a5537d84a008d566abc503d2ea41e30f2301403d`, and Cosmos SDK
v0.52.12 at `2dd23a60cf34c3ab90c8a4f15a7f35d9b2ac69b4`. The binary uses the
standalone Makefile recipe's `ledger` tag and SDK version/name/commit ldflags,
with Go **1.26.9** on Linux amd64. That compiler differs from the tracking
issue's original Go 1.26.6 capture. `oracle-build.json`, `dependencies.json`
and `go-build-info.txt` preserve the exact build identity and module checksums.
This is a source-built standalone oracle, not a downloaded release binary or
an enabled historical-version multiplexer.

`mainnet-observation.json` records a read-only mainnet observation before
capture. `metadata.json` binds public inputs, capture sources, Go build identity
and recorded output hashes. `go-state.json` preserves raw ABCI Info and the
matched chain ID, height 20 and app hash. The Rust transport test reconstructs
that state from the same public requests and checks its committed hash before
comparing requests over a real gRPC socket.

## Supported comparison and diagnostic identity

The tested contract compares gRPC code, message text including gas, absence of
protobuf status details, and the presence/value of `x-cosmos-block-height`.
Successful controls compare protobuf response bytes exactly. Repeating the same
successful simulation verifies that its discarded writes do not change the next
simulation's behavior.

Go's bracketed diagnostic source frames (` [path/file.go:line]`) are an explicit
identity deviation. Rust does not impersonate Go source locations. The fixture
retains every original Go annotation; the scoped test removes only syntactically
identified Go source frames from the expected message. It preserves all remaining
error text, source-independent punctuation, gas, status and metadata. Production
code does not special-case transaction request bytes or manufacture a Go origin.
This decision applies to the Simulate comparison and does not silently broaden
normalization policies for other endpoints.

The 14 cases cover:

- Missing transaction bytes and malformed outer protobuf requests.
- Wrong-wire-type and truncated transaction decoding, plus an empty legacy Tx.
- Missing signatures, wrong sequence and an oversized memo in ante handling.
- Bank execution rejected for insufficient funds.
- Successful simulation and repeat success, ensuring cancellation of simulation
  writes rather than committing them.
- Latest/default, historical and future query heights.

Height metadata belongs to the resolved query context. Valid requests that fail
in their handler retain it; a malformed outer request and a failed query context
have none. The implementation records decoder stage separately from handler
failure so adding error metadata does not fabricate a context height for a
request that never reached the interceptor.

The corpus contains no nonempty protobuf status details. Raw HTTP/2 header versus
trailer placement, dynamic HTTP framing, general error coverage of every message
family, operator limits, transaction submission, pre-v10 execution and the all-API
soak remain separate coverage. This test does not establish universal provider
readiness.

## Reproduce

First verify the active mainnet release with the explicit read-only observer
from `conformance/mainnet-rpc`, then build the selected isolated oracle with the
repository's release-recipe builder. With Rust and the pinned Go toolchain
available:

```sh
APPD=/path/to/celestia-appd \
MAINNET_OBSERVATION=/path/to/fresh-mainnet-observation.json \
  conformance/mainnet-rpc/simulation/record.sh /tmp/new-simulation-capture
cargo test --locked -p devnet-conformance --test simulation_errors
```

The adjacent `$APPD.build.json` must match the binary hash, app commit and ledger
tag; `APP_BUILD_MANIFEST` overrides its location. The capture creates an isolated
home and local listeners, replays the public request stream, records Simulate
results and shuts the node down. It creates no production writes. Output must be
empty; ports can be changed with `SIM_ABCI_PORT`, `SIM_GRPC_PORT` and
`SIM_REST_PORT` if another local capture is active. Preserve the logs for diagnosis,
but copy only the reviewed JSON/text artifacts into the fixture directory.
