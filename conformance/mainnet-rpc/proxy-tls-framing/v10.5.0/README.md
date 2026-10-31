# Issue 191 proxy, TLS and framing campaign

This bundle targets the acceptance slice left after merged PRs #205, #211 and
#216: provider-relevant proxy/TLS handling and malformed HTTP framing with
bounded local tests. The tagged nodes ran in fresh isolated homes, the raw
capture and local identities are frozen, and the offline verifier binds the
complete published inventory.

The release was selected by the exact read-only `observe-mainnet.sh` command.
Its `/status` and `/abci_info` responses agreed on mainnet chain `celestia`,
height `15449478`, application protocol `10`, and celestia-app `10.5.0`. This
selects the local oracle release; it does not claim public-provider parity.

## Oracle and corpus

The local binary remains the independently bound v10.5.0 build used by the
earlier issue 191 campaigns:

- module `github.com/celestiaorg/celestia-app/v10@v10.5.0`, source commit
  `6013e758d5dceb75827ad191c8d739d7c43cfa04`;
- embedded `github.com/celestiaorg/celestia-core@v0.42.3`;
- binary SHA-256
  `eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38`;
- Go compiler `go1.26.9 linux/amd64` (the compiler drift from the issue's
  original 1.26.6 baseline remains explicit).

The exact `run-source.sh` command created isolated single-validator homes and
ran the complete campaign:

```sh
./run-source.sh
```

The runner exercises six configurations sequentially: certificate-only and
key-only plaintext, valid relative and absolute TLS paths, a missing pair, and
a mismatched certificate/key pair. Tagged Go's `IsTLSEnabled` requires both
fields, so either half-configuration must remain plaintext without opening the
named file. Both bad full pairs must fail closed without a plaintext fallback.
The exact conformance-only certificate, private keys, OpenSSL config/version
and generation command are retained. The certificate is valid for roughly a
century so offline tests do not acquire a short expiration time bomb; the
verifier binds its DER hash, SANs and validity interval.

`campaign.json` freezes the exact case names in six categories. The TLS cases
cover verified HTTPS HTTP/1.1 health, JSON-RPC and preflight; a real HTTP/2
health exchange; a WSS JSON-RPC request/response and clean close; the TLS
node's enabled REST GetLatestBlock query; hostname rejection; and
plaintext/TLS protocol confusion. Proxy cases
cover public versus backend Host, inert standardized and de-facto forwarded
headers, CORS behind those headers, and absolute-form targets. Framing cases
cover same/conflicting lengths, Transfer-Encoding plus Content-Length,
transfer-coding order, chunk extensions/terminators/sizes, trailers, malformed
lines and headers, truncation, and three pipelined connection survival/closure
boundaries.

Every request and response is retained as raw base64 plus byte count and
SHA-256. Semantic checks compare status, body and stable headers exactly.
For semantic header comparison, names are lowercased and each name maps to an
exact duplicate-preserving value multiset. This permits HTTP/2's required
lowercase names and harmless Go/Hyper casing or order differences without
discarding repeated values. Raw response bytes retain order and casing as
integrity evidence.
The stable projection excludes exactly `Connection`, `Content-Length`, `Date`,
`Transfer-Encoding`, and `X-Server-Time`; raw bytes remain available and the
pipeline cases assert connection reuse or closure separately.

## Source audit affecting implementation

Tagged Go has no trusted-proxy middleware in this path. `Forwarded` and
`X-Forwarded-*` are ordinary request headers. Endpoint-list HTML is derived
from the effective HTTP Host, and CORS is derived from Origin. A reverse proxy
therefore needs to preserve or intentionally rewrite Host; forwarded host and
scheme fields do not override it.

Tagged Go calls `http.Server.ServeTLS` only when both TLS config fields are
nonempty. Rust now resolves both fields and follows that pair rule. Adding TLS
to the listener alone is insufficient: Rust's SDK query adapter self-dials the
RPC listener, while Go uses an in-process local client. The implementation
therefore creates a private ephemeral loopback HTTP listener for those
internal calls only when the public RPC listeners use TLS.
The configured public listeners remain TLS-only and expose H1 plus H2; H1
continues to use Hyper upgrades for WSS. Focused production tests start the
TLS RPC plus enabled app gRPC/REST services and exercise cmtservice
GetLatestBlock through the query surface, so the loopback adaptation cannot
hide a broken SDK query path.

The offline verifier binds the complete source/build/config/certificate/
input/output manifest, exact case inventory and category counts, observer and
capture ordering/freshness, raw request/response/frame hashes and stable header
projection. It hard-codes the captured semantic contract rather than only
checking self-consistency. Mutation tests independently change an
inventory name, raw request, status, body, stable header, H2 protocol, WSS
response/close, config outcome, source hash, and observation time/height and
must reject every mutation.

## Explicit campaign boundaries

Node homes, databases, keyrings, validator/node private keys and ordinary
generated node logs remain scratch-only. Published inputs are limited to
public genesis/config files, the explicitly labeled conformance TLS key, exact
source/build receipts and captured exchanges.

This bounded campaign does not establish public-provider response parity. It
does not cover PROXY protocol v1/v2, mTLS/client authorization, certificate
rotation without restart, ACME, provider-specific load-balancer timeouts,
retries, rate limits or rewrites, slowloris/resource fuzzing, or malformed
HTTP/2/HPACK below the HTTP request abstraction. Neither tagged Go nor current
Rust configures PROXY protocol or mTLS in this RPC path. These omissions do not
hide response fields: stable metadata remains exact, dynamic fields are named,
and connection behavior is asserted separately.

## Captured Go contract and bounded Rust differences

The tagged server negotiated both HTTP/1.1 and HTTP/2 over TLS, served an
actual WSS JSON-RPC health exchange followed by close code 1000, and kept the
app REST and gRPC services enabled while REST GetLatestBlock queried the TLS
node at height 1. A certificate-only or key-only setting stayed plaintext.
Relative and absolute complete pairs enabled TLS. Missing and mismatched full
pairs failed closed without a plaintext fallback.

Forwarding headers did not change the effective scheme or host. Root endpoint
links followed the HTTP Host, except that an absolute-form request target's
authority took precedence. The raw framing cases freeze Go's acceptance of
duplicate identical Content-Length, chunk extensions, declared trailers,
obsolete line folding, and HTTP/1.2; its rejection of the other malformed
inputs; and connection reuse or closure after three pipelined boundaries.

The Rust implementation deliberately retains native Hyper parsing for bytes
that are rejected before an application request exists. Those automatic
responses have an empty body and no stable Content-Type instead of Go's
`text/plain` parser diagnostic. Hyper also rejects obsolete line folding and
HTTP/1.2 with 400 where this Go version accepts them, and rejects unsupported
`gzip` or non-final `chunked` transfer codings with 400 where Go returns 501.
Hyper also closes after a request containing both Transfer-Encoding and
Content-Length, while Go ignores the length and permits one more request.
The TLS acceptor closes a plaintext client during handshake rather than
returning Go's HTTP/1.0 plaintext-on-TLS diagnostic. Focused tests freeze each
of these bounded parser differences. After the matching WSS close code 1000,
the upgraded Rust TLS stream terminates promptly but may surface as
`UnexpectedEof` because it does not send a separate TLS `close_notify` alert.
The socket test requires that termination under a one-second bound.
Reimplementing an HTTP/TLS pre-parser to copy diagnostic text would duplicate
security-sensitive framing logic.
