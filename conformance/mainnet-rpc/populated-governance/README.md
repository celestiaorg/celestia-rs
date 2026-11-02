# Current-release populated governance slice

This focused evidence slice advances issue
[#188](https://github.com/celestiaorg/celestia-rs/issues/188) beyond the
staking slice. It runs a fixed governance transaction sequence against a
fresh isolated celestia-app v10.5.0 and celestia-core v0.42.3 devnet, then
records identical protobuf query inputs through the direct ABCI socket,
Comet JSON-RPC `abci_query`, Cosmos gRPC and grpc-gateway REST.

The workload leaves three proposals populated at capture time: a standard
proposal in voting with a plain and a weighted vote, a proposal in deposit
period with two depositors, and an expedited proposal in voting. Each
proposal contains a `MsgUpdateBlobParams` protobuf `Any`; one also uses
structured metadata. Queries cover individual proposals, filters by status,
voter and depositor, pagination by key and offset, totals, reverse and empty
pages, individual and paginated votes and deposits, tally, params,
constitution, and successful historical reads. Every request is pinned to
its recorded height.

No response normalization is admitted for this slice. Direct ABCI, Comet
RPC and Cosmos gRPC must return identical protobuf bytes. The offline
verifier decodes each proposal's `Any` type URL and payload, then requires
the REST `@type`, authority and blob parameters to represent those exact
typed values; JSON object-key order is never used for comparison. Vote and
deposit page order remains part of the expected response and is not sorted
by the verifier.

The committing block binds the capture to its app hash. `genesis.json`, the
raw ABCI stream, state snapshots and complete block archive preserve the
inputs needed to inspect the same chain. `capture/metadata.json` hashes every
listed source and artifact plus each generator binary and build receipt.
`verify.py` checks those hashes, current-release provenance, observation order
and freshness, state identity, block/state structure, exact case inventory,
four-transport protobuf agreement and the populated governance controls. The
listed sources include the Rust replay, workspace and crate manifests, and the
exact `Cargo.lock` used for that replay. They also bind the production REST
parser change needed to accept the numeric governance enum form admitted by
the Go grpc-gateway capture.

The read-only mainnet observation must precede the local capture by no more
than 24 hours. It selects the active release only; no public endpoint is
mutated and no old fixture is relabelled. The source-built app uses its
recorded `ledger` tag and pinned module commit.

This remains a partial #188 artifact. The Go transport capture sends no
proof-bearing request and records no Go proof bytes, so it makes no Go/Rust
proof-parity claim. The Rust replay separately generates an IAVL plus
multistore proof from the replayed `gov` store and verifies it against the
admitted query-height root. Completed/cancelled/expired proposal lifecycles,
governance v1beta1 compatibility, failed historical queries, historical key
and offset page traversal, invalid pagination cursors, malformed or error
requests, invalid UTF-8, and other populated modules remain out of scope.
Those omissions are recorded in the transport capture and metadata.

The batch boundary follows an audit of the current Rust query router rather
than an assumption that the other issue items are covered. Governance v1 and
v1beta1 are registered, so this batch selects the v1 API and leaves v1beta1
explicitly out. Authz and feegrant have populated query implementations but
need their own `Any` workloads. IBC exposes a broad query surface and needs a
separate multi-node/relayer oracle. Vesting has no dedicated query service;
its populated representation is the account `Any` returned by auth queries.
Hyperlane, forwarding and zkism have registered Rust queries but require a
larger linked-state workload. Fibre's five Go query methods are registered in
Rust and likewise remain a separate payment workload. The blob module only
exposes `Query/Params`; pay-for-blob data is not a populated module-query
surface, so this proposal's blob `Any` tests its protobuf/JSON representation
without claiming populated blob-query coverage.

Regenerate with an unused port offset and a fresh observation produced by
`conformance/mainnet-rpc/observe-mainnet.py`:

```sh
MAINNET_OBSERVATION=/tmp/current-mainnet-observation.json \
DEVNET_PORT_OFFSET=6400 \
  conformance/mainnet-rpc/populated-governance/record.sh
```

Verify offline afterward:

```sh
python3 conformance/mainnet-rpc/populated-governance/verify.py
```
