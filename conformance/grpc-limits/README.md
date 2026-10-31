# gRPC message limits and listener capacity

This regression work is part of [#183](https://github.com/celestiaorg/celestia-rs/issues/183).
It wires operator message limits into the production SDK listener, including
its merged BlockAPI, and retains the standalone core listener's separate limits.
It does **not** complete #183's keepalive, handshake-deadline, compressed-message,
long-running resource-growth or comprehensive cancellation requirements.

## Behavior

`app.toml [grpc] max-recv-msg-size` and `max-send-msg-size` accept the quoted
integers emitted by celestia-app and ordinary TOML integers. Missing values use
celestia-app v10.5.0's initialized defaults: 64 MiB each. An explicit zero uses
the Cosmos SDK server fallback: 10 MiB receive, `MaxInt32` send. Negative or
malformed settings stop configuration loading. Rejecting negative sizes is an
intentional operator-facing difference: Go can construct a listener whose
negative limit rejects every message; RS reports the invalid configuration.

Limits count uncompressed protobuf bytes, excluding the five-byte frame header.
They apply to every stream message individually. Declared oversized requests are
rejected before tonic reserves their message buffer; the regression sends only
a five-byte declaration of `4294967295` bytes. Encoding checks a response's size
before copying it into tonic's output buffer. Handlers still construct their
protobuf response first, as Go marshals its response before the send check.

Actual Go socket captures distinguish unary and streaming send errors:

```
grpc: received message larger than max (65 vs. 64)
grpc: trying to send message larger than max (6 vs. 5)   # unary
trying to send message larger than max (709 vs. 128)    # streaming; size varies
```

The streaming encoder yields each complete message so a later oversized message
cannot discard an earlier valid buffered frame. Only tonic 0.12.3's transport
size errors are translated; application statuses and metadata are preserved.
The socket regressions protect that version-dependent conversion.

Unknown services and methods are dispatched before reading their request body,
matching Go's precedence over message-size and invalid-height errors. The SDK
listener registers BlockAPI; core BroadcastAPI and BlobstreamAPI remain on the
standalone core listener. The latter uses 4 MiB receive / `MaxInt32` send limits
and honors `[rpc] grpc_max_open_connections` with capacity acquired before
accepting a socket. A TCP connect alone can succeed in the kernel backlog;
the capacity regression therefore checks an actual HTTP/2 RPC. Dropping a
new-height stream immediately removes its event subscription without requiring
another published block.

## Evidence and scope

`v10.5.0/` is a fresh isolated actual Go transport capture. Its `meta.json` binds
the raw answers, input genesis/configs, capture source hashes and oracle build
manifest. The oracle is celestia-app v10.5.0 commit
`6013e758d5dceb75827ad191c8d739d7c43cfa04`, core v0.42.3, SDK v0.52.12, built with
its `ledger` build tag and release identity flags. The compiler is **Go 1.26.9**,
the repository's explicit oracle pin; it is not the issue's earlier 1.26.6
recording. `go-build-info.txt` records the complete actual dependencies and
`go-build.json` the binary SHA-256, target and flags. Read-only mainnet
observations are captured before selecting this oracle.

The captures cover SDK receive sizes 63/64/65 with a custom 64-byte limit,
SDK unary response sizes below/at/above a six-byte Config response (send limits
7/6/5), oversized unary and streamed BlockAPI responses, unknown-route precedence,
and core receive sizes `4 MiB - 1`, `4 MiB`, `4 MiB + 1`. Each request is also sent
with an invalid height header so the raw precedence result remains reviewable.
These are bounded transport cases, not an all-endpoint equivalence result.

Config's successful height metadata and BlockAPI Status's runtime node identity
are captured without modification. Heights vary during recording; Status size
includes the Go node's identity. The RS production regression uses the existing
**historical** single-validator test genesis and verifies wiring, exact size
errors, stream frames and capacity recovery; it is not relabeled as a matched
current-release chain replay. The record does not normalize source annotations
or hide invalid-height discrepancies elsewhere in the API.

Connection capacity zero/two, keepalive policy, handshake deadlines, compressed
messages, exhaustive cancellation/slow-reader loads and memory-growth campaigns
remain coverage gaps. No universal provider-readiness claim follows from this
corpus or from the passing runtime tests.

## Regenerate and verify

Fresh network capture is explicit, separate from ordinary tests. Use a reviewed
v10.5.0 `ledger` oracle with the adjacent `.build.json` provenance stamp. It
creates its own temporary validator/home and only writes to loopback devnets;
no validator keys or databases are copied into the output.

```bash
ORACLE_BIN=/absolute/path/celestia-appd OUT=/tmp/grpc-limit-capture \
  GOTOOLCHAIN=go1.26.9 DEVNET_PORT_OFFSET=5000 conformance/grpc-limits/record.sh
cargo test --locked -p grpc-transport
cargo test --locked --no-default-features -p celestia-rs --test grpc_limits
cargo test --locked --no-default-features -p celestia-rs --lib grpc_limit_tests
python3 conformance/grpc-limits/verify.py
```

`OUT` must be empty. New test keys, timestamps, genesis hashes and node IDs vary,
so regeneration reproduces the scenario rather than identical random bytes.
The `recorder` rejects non-loopback endpoints and bounds request sizes and stream
frame counts. Its protobuf padding is a valid unknown field, allowing exact byte
boundaries without malformed-request errors.
