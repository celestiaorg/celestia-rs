# Bounded persistent provider closure

This extends the ordered Comet campaign with fresh actual gRPC/REST listeners,
reconnecting WebSockets, bidirectional snapshot state sync, and pre-v10 queries
through the **official v10.5.0 multiplexer**. It completes the tested scope of
[#189](https://github.com/celestiaorg/celestia-rs/issues/189); startup ordering,
wire deviations and retention exclusions below remain conditions of the result.

## Persistent matrix

Four profiles combine retained/custom-pruned application versions with `kv` or
`null` transaction/block indexing. Custom pruning keeps ten recent versions at
interval ten. Block retention is unlimited, FinalizeBlock responses are kept,
and Cosmos gRPC and REST are explicitly enabled. Persistent Go and Rust full
nodes follow the same Go validator and stop at a common tip for each capture.

Each profile compares forty Comet requests and eighty-four gRPC/REST requests
before restart, after restart on the same homes, and after crossing the pruning
boundary: **480 Comet + 1,008 gRPC/REST response pairs**. The matrix includes
latest/historical blocks, commits/results/validators, transactions and proofs,
event searches with ordering/page pagination, application/store queries,
key/offset/reverse/empty/invalid-cursor pages, and exact errors for missing,
future, pruned and disabled-index data. API height inputs include latest, height
one, the selected historical height, future, negative and malformed values.

The grpc-go client retains initial headers and trailers separately. Stable
content types and height metadata are compared along with status/message and
protobuf bytes. REST compares status, stable metadata and decoded JSON bodies;
array order and pagination bytes remain significant. Only Go diagnostic source
frames, JSON object order/whitespace, Date/server timing, derived Content-Length
and unrelated HTTP headers are excluded. Transport-unavailable/deadline errors
are rejected as invalid capture results.

For each profile, clients subscribe while the validator is stopped, observe
both providers' sockets closing during restart, reconnect and receive the same
three ordered NewBlock heights. **Twenty-four deliveries** bind their block roots
to the retained archive; event indexes and data match after one explicit wire
normalization: Go's live empty `result_finalize_block.validator_updates` list is
`[]`, while Rust emits `null`. This allocation difference remains in raw frames;
no other event value or list is removed. Events already queued before disconnect
are retained separately, so a legitimate delayed delivery cannot masquerade as
a failed restart.

`batch-indexed` supplies both indexed profiles. A collision with an active
outbound ephemeral port prevented the next profile's P2P listener from starting;
its controller and startup log are preserved. `batch-null` supplies the two
complete disabled-index profiles on lower listener ports. No failed request was
rewritten or counted. The driver now defaults to lower ports. Both batches
archive exact capture/helper/native source snapshots and binary/compiler
receipts. Their native binaries have the same transport changes but different
build identities: the first is the earlier base plus the archived API patches;
the second is signed source `f51ce12`. These are separate recorded builds.

## Catch-up and state sync

Restart order is deliberate: the validator advances three heights beyond the
persisted tip before the full nodes restart. Both catch up and resume before
queries are captured. The older simultaneous-start shared Go/Rust stall remains
an explicit unsupported condition; this evidence does not diagnose or fix it.
Its preserved reproduction and verifier remain registered separately.

`state-sync` records the current source-built Go validator's snapshot at height
20. The Rust network syncer fetches and verifies light-client headers and
consensus parameters, restores its persistent application, and matches app hash
`8D525FA32A866020433779F2565974DF8ECC82051FDC0E6C3BEF852F36FF4E5D`
against header 21. Rust then serves its snapshot to a fresh Go node whose only
peer is Rust; Go independently verifies and restores that same height/root.
The snapshot/chunk, public genesis/config, trusted block, summaries and logs are
retained. This is the supported snapshot-reactor/application path, not a claim
of every production state-sync configuration or post-sync provider restart.

## Official historical multiplexer

`historical` uses the official Linux x86_64 v10.5.0 release asset, validated
against its published SHA-256 checksums. Unlike the standalone source-built
oracle above (Go **1.26.9**), this binary was built with Go **1.26.6**, matching
the issue's original compiler baseline. Its embedded v9.0.8 initializes the
isolated chain, the real multiplexer upgrades to v10, and Go/Rust providers retain
matching state on both sides of that boundary.

The current release serves **200 Comet + 420 gRPC/REST comparisons** at heights
1, 13, 14, 15 and 29 around upgrade height 14. Raw version observations establish
that the queried live oracle is v10.5.0/app version 10 while history predates
v10. Some height-one API inputs intentionally coincide; these are comparison
counts, not unique-request counts. Historical answers are freshly captured;
no older v9/v10.4 golden answer was relabelled.

The multiplexer delegates Query to the active application; it does not select
an old binary by requested query height. These results therefore cover retained
pre-v10 state queried with the current schema/handlers. They do not promise old
binary query semantics or arbitrary pre-v2 history. The exact official build,
archive checksum, configuration, upgrade/version sequence, complete blocks,
public workload and paired responses remain inspectable.

## Verification and limits

```sh
python3 conformance/mainnet-rpc/persistence/verify_closure.py
```

The verifier binds every artifact/source snapshot, pruning/index/API setting,
request branch, response pair, chronological progress, archived root, reconnect
payload and restored snapshot root. `rpc-persistence-closure.sh`,
`historical-multiplexer.sh` and `statesync-closure.sh` reproduce the isolated
campaigns; their headers document required binary/output settings. Select a fresh
mainnet observation, build the pinned helper commands `persistent-api` and
`persistent-ws`, and use an isolated `CELESTIA_APP_HOME`. State sync uses the
`statesync_node` example; the official multiplexer and its extracted v9 binary
are supplied explicitly. Public archives omit runtime private keys/databases.

Block-store pruning, discarded FinalizeBlock responses, unbounded reconnect or
slow-reader stress, other snapshot/network configurations, and universal
provider equivalence remain outside this bounded matrix. The persistent Comet
controller compares proof bytes; the module campaign separately checks proof
validity against committed roots. A passing integrity check alone does not
execute fresh nodes or enlarge these scopes.
