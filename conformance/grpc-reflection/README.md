# Streaming gRPC reflection

`grpc-reflection` implements the bidirectional reflection wire protocol used by
Celestia RPC providers. The separate core listener registers both
`grpc.reflection.v1.ServerReflection` and `grpc.reflection.v1alpha.ServerReflection`.
The Cosmos SDK listener registers only v1alpha. Merging BlockAPI into the SDK
listener does not register the core listener's reflection versions or advertise
BroadcastAPI/BlobstreamAPI there.

The two listeners have different descriptor registries as well as different
service registrations. The embedded registries are derived from original Go
streaming responses, preserving custom protobuf options byte for byte. They are
immutable protocol schema data; query state and block height do not affect them.
File lookup returns the requested file even if it was sent previously, followed
by previously unsent dependencies in breadth-first import order. The sent-file
set belongs to the stream and is discarded on completion or cancellation.
Lookup failures are response messages and allow subsequent requests. A malformed
request or missing operation terminates the stream. Half-closing an empty stream
completes successfully. Initial errors and empty streams produce trailers-only
responses, like grpc-go; initial response metadata carries no Cosmos height.

The handler reads the next input only when the transport polls its output.
There is no detached worker or unbounded response queue. Configured listener
message limits apply to every request/response, including the first response
before headers are sent. Descriptor traversal and stream deduplication are
bounded by the immutable registry. Unknown request fields survive in the echoed
original request; repeated known fields have protobuf's last-value semantics.

## Evidence and tests

`v10.5.0/{app,core}.json.gz` records actual request bytes, response messages,
terminal statuses and initial/trailing metadata from isolated Go sockets.
`meta.json` pins the source versions/commits, compiler, binary identity, capture
sources and artifact hashes. `go-build-info.txt` records the oracle's linked
modules and build settings. Old v10.4 fixtures are not used as expected answers.

Run:

```sh
cargo test --locked -p grpc-reflection
cargo test --locked -p api-server --no-default-features --test reflection
```

Socket replay covers service lists, filenames and dependencies, service/method/
message/field/enum/extension symbols, extension lookup/enumeration, absent objects,
repeated descriptor requests, multiple exchanges on one stream, both supported
versions, unsupported SDK v1, malformed protobuf and missing operations, unknown
fields, repeated fields and empty half-close. A separate socket test repeatedly
cancels open streams and confirms the connection still serves new requests.
The production SDK-router regression checks reflection dispatch outside the
height interceptor and verifies listener-specific service/version registration.

Two explicit normalizations apply when comparing Go responses:

- SDK extension-number enumeration is sorted. Its Go implementation iterates an
  extension map, so order varies without changing the set of extensions.
- Only ASCII space versus NBSP immediately after the `proto:` error prefix is
  normalized. Go protobuf's `internal/errors` deliberately randomizes that space
  per binary. Raw transcripts retain the actual bytes; codes and remaining
  error text remain exact.

One captured core extension request exposes an upstream Go failure rather than
a response to reproduce. Looking up extension 65003 of
`google.protobuf.FieldOptions` dereferences a descriptor with no parent file and
panics the isolated Go node. `v10.5.0/core-extension-crash.{json,log}` preserves
the exact request and stack trace. The Rust listener intentionally returns an
in-band NotFound response for that request and continues the same stream. The
socket regression verifies both the bounded error and subsequent stream reuse;
regeneration excludes that lookup from the ordinary capture so it cannot crash
the oracle again.

This evidence covers streaming reflection for the pinned provider build. It does
not establish query-state equivalence, every operator limit, general provider
readiness, or arbitrary protobuf framing behavior. The parser retains prost's
bounded recursion policy for deeply nested unknown groups.

## Regenerate

First verify the active mainnet release through a read-only public query. Build
that release's local oracle with its official standalone build tags and version
stamps, retain `go version -m`, binary SHA-256 and source identities, then start an
isolated devnet with both gRPC listeners enabled. Do not capture from production
or reuse older golden answers. From this directory, with pinned Go 1.26.9:

```sh
go build -buildvcs=false -o /tmp/reflection-capture .
/tmp/reflection-capture -endpoint 127.0.0.1:10090 -out /tmp/app.json
python3 - <<'PY'
import json
capture = json.load(open('/tmp/app.json'))
json.dump(capture['candidate_files'], open('/tmp/reflection-files.json', 'w'))
PY
/tmp/reflection-capture -endpoint 127.0.0.1:27658 -core \
  -seed-files /tmp/reflection-files.json -out /tmp/core.json
```

The SDK's unary FileDescriptors response supplies current linked filenames as
lookup inputs. Each embedded descriptor is independently fetched through
streaming reflection on its own listener; the core capture probes the same
current filename candidates. Service descriptor roots and their dependencies
are also discovered directly through streaming symbol lookup. Review the
transcripts, gzip them with timestamp zero into `v10.5.0`, run `embed.py`, update
all provenance/hashes, and rerun the tests. The script does not contact mainnet,
build/start a node, infer source identities, or assert release readiness for you.
