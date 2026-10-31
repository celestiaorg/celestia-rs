# REST height boundary evidence

`v10.5.0/rest-heights.json` is a frozen 42-request capture from a local
single-validator celestia-app v10.5.0 oracle. It covers successful queries,
handler errors, and unknown routes across absent, malformed, negative,
future, overflowing, and repeated `x-cosmos-block-height` inputs.

`v10.5.0/provenance.json` pins the source-built oracle, build recipe,
compiler, genesis, chain state, capture program, and contemporaneous mainnet
release observation. The build used Go 1.26.9 rather than the issue baseline's
Go 1.26.6; application, celestia-core, and Cosmos SDK source versions remain
the pinned v10.5.0 release versions. The binary is the standalone
`celestia-appd` package built with the `ledger` tag, not a release multiplexer.
The referenced build recipe, replayed chain state, and mainnet observation are
stored beside the capture so their hashes can be checked locally.

Run `capture.py --port <REST port> --out <path>` against an isolated Go
oracle to collect a new candidate. The frozen `capture-source.py` is retained
byte-for-byte so the source hash embedded in the accepted capture is
independently verifiable.

Run `python3 verify.py` to check all committed evidence hashes, the case
count, and the oracle identity recorded by the build recipe.

Tests compare status, exact JSON body bytes, and stable `Content-Type` and
`Grpc-Metadata-*` headers. `Date`, `X-Server-Time`, `Content-Length`,
`Transfer-Encoding`, and trailer declaration are transport-generated and are
explicitly outside this fixture.
