# REST POST and protobuf-JSON boundary evidence

`v10.5.0/rest-post-boundaries.json` is a frozen 82-request capture from an
isolated celestia-app v10.5.0 REST server. It records the exact HTTP request
and response bytes plus parsed status, body, headers, and trailers for:

- all nine registered POST routes with an empty object and an empty body;
- absent, JSON, parameterized JSON, text, binary, empty, and repeated
  `Content-Type` fields;
- unsupported and case-mismatched methods;
- unknown, trailing-slash, double-slash, percent-encoded, and malformed paths;
- query and path escaping, including repeated query fields; and
- malformed and edge protobuf JSON, integer and enum boundaries, and valid
  decode, simulation, and decode/encode round-trip controls.

The capture stores every raw request and response as base64 and binds its
size and SHA-256 digest. The offline verifier reparses those raw messages and
checks that the structured status, body, headers, trailers, and stable-header
projection match. It also checks all local provenance files, the repository
inputs used to construct the valid transaction and replayed state, and the
successful transaction round trip.

`v10.5.0/provenance.json` pins the source-built oracle binary hash, source
commit, compiler, build tags, complete Go build information, module file,
genesis, application configuration, replayed state, capture programs, and
input fixture hashes. The batch replayed 194 requests to height 20 before the
capture. A read-only observation of an active `celestia` mainnet endpoint
selected application release 10.5.0 three minutes before capture. That
observation selects the oracle release; it is not evidence that the provider
matches every response in this fixture.

The build used Go 1.26.9 rather than the issue baseline's Go 1.26.6. It is the
standalone `celestia-appd` package built from celestia-app v10.5.0 commit
`6013e758d5dceb75827ad191c8d739d7c43cfa04` with the `ledger` tag, not a
release multiplexer.

The exact build information and application configuration are stored with
deterministic gzip compression. Their uncompressed hashes are bound separately
and checked by the verifier.

`replay-receipt.json` preserves the exact command, source, inputs, oracle
binary, configuration, and output hash for the 194-request replay that produced
height 20 and the captured application hash. It also binds the public ABCI
recording's original fixture metadata and generation source. The replay program
is frozen byte-for-byte as `replay-source.py` and consumed `abci.rec` directly.
The fixture toolchain's `rpc-recorder` binary is
identified separately by its SHA-256 digest, Go build ID and complete build
information, rebuild command, and hashes for every module source file. The
receipt states explicitly that `rpc-recorder` was not invoked by the replay.

`case-inventory.json` freezes all 82 case names grouped by category. The
verifier requires the capture to match that complete inventory; counts and a
small representative subset are not sufficient.

For response comparisons, `Connection`, `Content-Length`, `Date`, `Trailer`,
`Transfer-Encoding`, `X-Server-Time` are explicitly excluded from the stable
header projection. Their exact bytes remain in the raw capture for integrity.
Actual trailer fields are parsed separately; the captured responses emitted
none.

Run the verifier without a network connection:

```sh
python3 conformance/mainnet-rpc/rest-post-boundaries/verify.py
```

## Deliberate omissions

The oracle ran without CometBFT, so this batch does not include a successful
or state-mutating transaction broadcast. Valid broadcast modes would only
capture an environment-specific connection failure; malformed broadcast JSON
and enum boundaries that fail before the Comet call remain covered. The
fixture also does not claim a semantically valid request for every complex
POST message.

Malformed HTTP framing, request-size limits, persistent connections, TLS,
reverse proxies, browser CORS, and transport timeouts are outside this batch.
It does not define JSON object field order as an API contract, exhaustively
enumerate protobuf JSON behavior, or treat provider-specific responses as the
oracle. These observations are evidence for focused REST compatibility work,
not by themselves a claim that issue #191 is complete.
