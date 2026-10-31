# gRPC lifecycle source audit for the v10.5 oracle

The fresh read-only observation in `mainnet-observation.json` selected
celestia-app v10.5.0 before any local transport capture. The local oracle must
use app commit `6013e758d5dceb75827ad191c8d739d7c43cfa04`, core commit
`a5537d84a008d566abc503d2ea41e30f2301403d`, Cosmos SDK v0.52.12, grpc-go
v1.83.2 and Go 1.26.9. Captures are valid only when their metadata binds those
identities, the executed binaries and configs, and every recorder/verifier
source and input.

The Rust executable must be created by `build-rs.sh`. That single build step
requires a clean signed source commit, uses the locked release command with the
documented default features, and emits a preexisting adjacent JSON receipt. The
orchestrator verifies its binary, source, lockfile, compiler, target, command,
features, and embedded `version --long` commit before starting either node; the
offline verifier binds the same receipt and rejects binary/source rebindings.
Every enabled Go and Rust listener is explicitly loopback-bound and PEX is
disabled before startup. Frozen configs are independently parsed to enforce
those constraints.

## Frozen behavior

| Area | Go v10.5 behavior | RS behavior before this batch | Campaign assertion |
| --- | --- | --- | --- |
| Core connection quota | `[rpc] grpc_max_open_connections`; negative is invalid, zero skips `netutil.LimitListener`, positive values acquire before `Accept` | Same zero/unlimited and acquire-before-accept semantics | At quota 0, four established clients plus a probe all work. At quota 2, two established clients make a third RPC time out at 350 ms; closing one connection lets that same probe succeed within 10 s. |
| Core message sizes | No operator setting; grpc-go defaults to 4 MiB receive and MaxInt32 send | Fixed 4 MiB receive and MaxInt32 send | Existing 4 MiB - 1 / at / + 1 corpus remains authoritative. A gzip request whose decompressed message is 4 MiB + 1 is separately frozen. |
| Core keepalive | Hard-coded grpc-go `Time=30s`, `Timeout=10s`, `PermitWithoutStream=true`, `MinTime=10s` | Hyper keepalive disabled | The Go-only transport capture requires a raw, idle HTTP/2 connection to receive one non-ACK PING at 25–36 s and close at 35–50 s when the client sends no ACK. RS absence is established from bound production source and is recorded as an open deviation; no impossible RS timing capture is asserted. |
| HTTP/2 setup deadline | grpc-go default `ConnectionTimeout=120s`; neither app nor core overrides it | No explicit deadline on either production gRPC listener | The Go-only capture requires a zero-byte TCP client to close at 115–130 s. RS absence is established from bound production source and remains an open deviation; the verifier does not invent an RS close. |
| Cancellation cleanup | Canceling `SubscribeNewHeights` removes its listener when the stream context ends | The handler also removes the subscription on stream drop | On core, receive and decode one height, cancel, require exact `Canceled/context canceled`, then require a Ping through the same `grpc.ClientConn`. On SDK, receive and decode the reflection service inventory, cancel, and require the same inventory through the same `grpc.ClientConn`. These prove client reuse; they do not assert physical socket retention. |
| Slow reader recovery | A completed HTTP/2 setup and open height stream retain a connection quota slot until the socket closes | Same connection-lifetime quota | Each of two clients reads and decodes one valid framed height response, then stops reading. At quota 2 a probe is blocked; after closing the exact sockets it succeeds. At quota 0 the concurrent probe succeeds before cleanup. |
| Request compression | The actual app binary registers gzip. grpc-go applies the receive limit to decompressed bytes | Tonic request compression is disabled | Core sends 4 MiB + 1 decompressed bytes; SDK sends an exact 65-byte reflection request against its custom 64-byte limit. Go returns ResourceExhausted with exact sizes. RS returns Unimplemented `Content is compressed with \`gzip\` which isn't supported` on both listeners. |
| Process resources | The isolated node remains alive for each bounded campaign | Same | A 250 ms Linux `/proc` trace retains RSS, thread and file-descriptor counts through all six campaigns and a two-second settle. PID start time and executable hash bind every trace to the reviewed binary. The verifier enforces sampling continuity, caps absolute RSS/threads/FDs, and rejects unrecovered FD/thread growth or more than 256 MiB retained RSS growth. |

Successful Ping calls require code 0, an empty protobuf response, exact
`content-type: application/grpc` headers and empty trailers. Saturated probes
require `DeadlineExceeded/context deadline exceeded`, no response, and no
headers or trailers. Receive-limit errors require code ResourceExhausted, no
headers, and `content-type: application/grpc` trailers.

The raw HTTP/2 ledger retains the client preface and every emitted frame, plus
each complete server frame. The verifier decodes the nine-byte frame header and
checks its length, type, flags and stream ID against the recorded fields. It
also proves the keepalive client emitted no PING ACK.

## Acceptance composition

Merged PR #203 supplies the production SDK configuration wiring and its actual
Go v10.5 evidence for custom receive/send boundaries, below/at/above requests,
unary and streaming oversized responses, metadata, route/decode precedence,
the independent core defaults, and cancellation cleanup. This campaign covers
the remaining core connection quotas 0/2, core and SDK cancellation recovery,
core slow-reader recovery, core and SDK gzip behavior, Go keepalive/no-pong and
setup-deadline behavior, the source-audited RS absences, and bounded process
resources. The merged provider
soak under `conformance/mainnet-rpc/provider-soak/` separately supplies a
310-second mixed-node resource envelope under ordinary provider traffic; it is
supporting resource evidence and is not relabeled as this transport campaign.

## Unsupported settings and bounded omissions

- Neither the Go SDK listener nor the RS SDK listener has an app connection
  quota setting. This campaign does not invent one.
- Neither core listener exposes message-size settings. The fixed Go defaults
  remain the compatibility target.
- Go SDK keepalive uses grpc-go defaults: a two-hour server ping, 20-second
  timeout, five-minute client-ping enforcement minimum and no pings without a
  stream. A two-hour live capture is outside this bounded batch. The lack of an
  operator setting is recorded separately from the core listener's explicit
  30/10 policy.
- Compression has no app/core TOML setting. gzip availability is a build-level
  transport behavior. RS's rejection is safe because it happens before
  decompression, but it is not Go parity.
- Slow-reader evidence first reads and semantically decodes a valid positive
  height, 32-byte block hash and timestamp, then stops reading while the quota
  probe runs. It proves connection-slot recovery after close; it does not claim
  an application-level write deadline or an exhaustive memory-growth soak.
- HTTP/2 timing windows are relational evidence. Exact monotonic milliseconds
  are retained, while the verifier admits bounded scheduler tolerance.
- The resource trace is a bounded acceptance campaign, not an indefinite leak
  proof. It retains every sample used by the stated absolute and recovery
  checks, and does not infer allocator internals from RSS.

## Source anchors

- Core keepalive: `celestia-core/rpc/grpc/client_server.go`
- Core quota configuration and listener: `celestia-core/config/config.go`,
  `celestia-core/node/node.go`, `celestia-core/rpc/jsonrpc/server/http_server.go`
- grpc-go setup timeout and defaults: `google.golang.org/grpc/server.go`,
  `google.golang.org/grpc/internal/transport/defaults.go`
- SDK size settings: `cosmos-sdk/server/grpc/server.go` and
  `cosmos-sdk/server/config/config.go`
- RS connection quota and listener: `crates/tier-4-runtime/rpc/src/grpc.rs`
- RS SDK production merge and settings:
  `crates/tier-5-node/celestia-rs/src/rpc_service.rs`
- RS codec and compression defaults:
  `crates/tier-4-runtime/grpc-transport/src/lib.rs`

## Capture and verification

Immediately before capture, create a fresh read-only mainnet observation with
the repository observation helper. Build the Rust executable and its adjacent
receipt from the exact signed candidate, then run only against isolated
loopback nodes:

```sh
python3 conformance/mainnet-rpc/observe-mainnet.py \
  --endpoint https://reviewed-mainnet-comet-rpc.example \
  --out /tmp/fresh-mainnet-observation.json
conformance/grpc-lifecycle/build-rs.sh /tmp/celestia-rs-grpc-lifecycle
ORACLE_BIN=/absolute/path/celestia-appd \
CORE_BIN=/absolute/path/cometbft \
RS_BIN=/tmp/celestia-rs-grpc-lifecycle \
RS_BUILD_RECEIPT=/tmp/celestia-rs-grpc-lifecycle.build.json \
MAINNET_OBSERVATION=/absolute/path/fresh-mainnet-observation.json \
OUT=/absolute/path/empty-capture \
GOTOOLCHAIN=go1.26.9 \
conformance/grpc-lifecycle/record.sh
python3 conformance/grpc-lifecycle/verify.py /absolute/path/empty-capture
```

`ORACLE_BIN` and `CORE_BIN` must have the reviewed adjacent Go build receipts.
The output inventory excludes node homes, databases, keys and logs; it contains
only sanitized configs/genesis, response and frame ledgers, resource traces,
source/build receipts, the fresh public observation, hashes and metadata.
