# Read-only pooled RPC relay

The HTTPS follower uses a curl process for each request. A long historical
replay can therefore spend substantial time establishing TLS connections.
This loopback relay forwards replay reads through Go's reusable HTTPS
transport, with HTTP/2 when negotiated and a global request-start rate cap.
It uses only the Go standard library and keeps certificate verification enabled.

Build and run with the module's pinned Go version:

```sh
cd conformance/rpc-relay
GOTOOLCHAIN=go1.26.10 go build -trimpath -buildvcs=true -o /tmp/celestia-rpc-relay .
/tmp/celestia-rpc-relay --upstream https://rpc.celestia-mocha.com \
  --listen 127.0.0.1:39119 --rps 16 --connections 8 --duration 20h
```

The first stdout line is JSON identifying the listener, upstream, process and
rate. With `--listen 127.0.0.1:0`, the OS chooses a free port. Use the printed
loopback URL as a follower RPC primary and retain independent witnesses.
Both the requested and resolved listen address must be loopback.

Only GET requests to status, block/header/commit, validator, consensus-parameter,
genesis and ABCI read routes are forwarded. Other methods and paths are rejected
locally, including transaction broadcasts and WebSocket traffic. The upstream
must be an HTTPS origin without credentials, a query, fragment or non-root path.
RPC query parameters, JSON payload bytes and upstream HTTP status codes are
forwarded without JSON parsing or reserialization. Transport failures return
502. Each accepted upstream request consumes a shared rate slot (burst one),
including concurrent callers. `--connections` limits TCP connections; HTTP/2
can multiplex requests on a connection.

The relay adds transport pooling, not a cache, application execution or a new
trust root. Keep the follower's genesis/snapshot and header/data checks enabled.
For retained evidence, identify the relay source revision, Go build information,
executable hash, upstream and rate separately from the Rust execution candidate.
The relay's stdout metadata and follower ledger do not independently authenticate
network data. Use a new ledger for each replay invocation and retain interrupted
runs. `--duration` optionally bounds the relay lifetime; SIGINT/SIGTERM initiate
shutdown, with a ten-second grace period before closing remaining connections.

Validation: `go test -race ./...`, `go vet ./...` and `go mod tidy -diff`.
The TLS test sends four RPC reads over one upstream connection and checks byte,
query, Host and status preservation. Other tests cover denied mutation paths,
canceled acquisition and rejection of an untrusted TLS certificate.
