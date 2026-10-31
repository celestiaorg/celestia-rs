# Paired live-transaction closure campaign

This directory contains the fail-closed harness and offline verifier for the
final current-release transaction-submission campaign. The campaign runs only
on an isolated loopback network with one tagged Celestia App v10.5.0 validator
and one reviewed `celestia-rs` validator. Both validators share the same chain,
committed state and proposal boundaries, while P2P mempool broadcast is
disabled so the harness can submit identical bytes to both local mempools.

The frozen inventory has 28 cases. It includes Cosmos gRPC and REST, core gRPC,
Comet JSON-RPC/URI/WebSocket async, sync and commit forms, exact rejection and
success behavior, mempool saturation and recovery, client disconnect and gRPC
cancellation, server timeout and slot reclamation, a terminal malformed
protobuf case, a proxy-observed original BlobTx envelope, and a successful
staking delegation. The verifier rejects missing, extra, renamed or reordered
cases.

`harness.py` supplies the capture-side security and integrity primitives:

- strict JSON parsing with duplicate-key and non-finite-number rejection;
- loopback-only endpoint and listener validation;
- exact case inventory and request-byte equality checks;
- proposal-boundary admission-window validation;
- stable HTTP, WebSocket and gRPC metadata projection;
- continuous disk-floor monitoring with process-group termination;
- exact normal/timeout configuration-delta validation; and
- symlink-safe manifest finalization.

`driver.py` executes one frozen case at a time with a receipt-bound local
`provider-client` binary. Two workers rendezvous before an event releases both
submissions; the output preserves the release/invocation/completion monotonic
times, exact request and result hashes, and the driver/client executable
hashes. The transport client has dedicated paths for an unmodified invalid
REST body, an empty Cosmos gRPC request, a direct malformed core protobuf
request, complete-write disconnect and cancellation, and a raw WebSocket
handshake with a fixed key. It does not accept a scenario command.

`blob_proxy.py` is a one-shot loopback server. It accepts only one non-batch
JSON-RPC `broadcast_tx_sync` request with integer id 0, canonically parses its
BlobTx protobuf, records the raw request and envelope with exclusive file
creation, and returns a synthetic response without forwarding the broadcast.
The final artifact also binds the frozen CLI argv/source, decoded inner tx,
namespace, data, share version, signer field and the exact envelope submitted
to both nodes.

`campaign.py` is the receipt-bound outer controller. It validates every binary,
source, seed and configuration hash; owns the normal, timeout and restored node
generations; invokes the fixed paired driver; performs lifecycle retries and
indexed state queries; runs the one-shot BlobTx proxy and exact Go CLI; and
writes only the public artifacts named by `verify.py`. It refuses an existing
output directory. Node homes, databases, keyrings, mnemonics, validator or node
keys, environment dumps and raw logs are never admissible evidence.
Before starting, it proves that both home genesis files equal the public
genesis and that each validator key's public identity matches its named genesis
validator. The evidence preserves every complete parsed config and rejects any
normal/timeout/restored delta except the declared commit timeout.

`verify.py` verifies a single explicitly named capture root. It never discovers
or selects a convenient capture. It checks exact file inventory and hashes,
current-release provenance, topology and sanitized configuration, the 28-case
matrix, paired pre-state and request identity, admission timing, response
equivalence, inclusion and final-state equality, cancellation/timeout cleanup,
BlobTx proxy provenance, process identity and resource cadence, and continuous
free-space samples. The only allowed response difference is the fully spelled
out terminal malformed-protobuf outcome. HTTP `Date` and the Go-only
`x-server-time` value are the only excluded header values.

The lifecycle artifact is event based. It records every capacity,
disconnect, cancellation and timeout phase with monotonic time, transaction
hash, local mempool size, status height and the complete transport response
when a response exists. The verifier freezes the allowed transition sequence,
links retries and inclusions to their original hashes, requires a distinct
capacity probe and follow-up, and checks that all events occur inside the
receipt-bound node sampling interval. Listener PIDs, process samples and
binary hashes must identify the same two processes within each generation.
The controller compares the complete owned listener inventory, including P2P,
with the loopback-only configured endpoints and rejects any extra, missing or
non-loopback listener.

The node evidence is generation aware. `normal`, `timeout`, and `restored`
each have distinct Go/Rust PID start ticks, listener snapshots, resource
samples and bounded monotonic intervals. The timeout config differs only in
`timeout_broadcast_tx_commit`; the restored config must equal the normal
config byte-for-byte at the structured level. Cases and lifecycle events name
their generation. Timeout intervals are derived from four consecutive raw
common-block observations rather than accepted as free duration integers.

Lifecycle retry, capacity probe and follow-up calls carry receipts bound to
the reviewed outer campaign and transport-client hashes. Their raw Comet or
core-gRPC CheckTx result is decoded into the asserted success, mempool-full,
duplicate or sequence/capacity outcome. Capacity B and every successful
follow-up have paired indexed inclusion/result/post-state records that advance
the state consumed by the next main case.

Each main response embeds the complete canonical driver document and its hash,
then cross-binds every projected response and chronology field to that document.
Disconnect, cancellation and server timeout deliberately have no client-side
CheckTx body; their inclusion records label CheckTx as admission-only instead
of inventing gas, events or log fields. All other successful calls retain the
concrete broadcast response. After the terminal malformed request, the
controller waits for a later common block and freshly queries paired state and
mempools, proving restored-generation liveness and final continuity.

`capture.py` accepts only the local `campaign.py` source with its exact SHA-256
and fixed Python entry point. The campaign remains unexecuted until its exact
static tree receives independent runtime approval and fresh build receipts bind
the final source hashes.

No fixture is committed until a fresh read-only mainnet observation selects
the active 10.5.0 release, both binaries are built and receipt-bound, all 28
cases complete on real transports, every verifier mutation fails for the
intended reason, and an independent reviewer approves the exact artifacts.
The prior live-transaction fixture remains a regression input; it is not
promoted into paired closure evidence, and its reconstructed BlobTx wrapper is
not described as captured bytes.

