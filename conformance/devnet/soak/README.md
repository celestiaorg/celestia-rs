# Soak client failures

The bank workload records exceptions during account initialization, broadcasts,
and sequence resynchronization as `client-error` entries in `events.log`.
Each entry retains the exception type, bounded message and traceback, operation,
node kind and endpoint, chain height, and public account context. Broadcast
failures also retain the transaction SHA-256, byte length, sequence, account
number, and request timeout. Private keys and raw signed transactions are not
recorded.

`report.json`, `report.md`, and restart state retain counts and the first/latest
samples per category/operation in `metrics.client_errors`. The grouping excludes
exception messages and transaction identifiers to bound memory and report size.
The event log retains every occurrence; its disk usage grows with the run like
the existing node and workload logs. The existing `bank.errors` count still
counts broadcast exceptions only. Account-query exceptions have separate counts.
This also works when resuming older state without a `client_errors` entry.

A recorded client exception needs investigation: a timeout or invalid response
alone does not attribute a fault to either implementation. These diagnostics do
not make an earlier unclassified failure explainable, and do not establish an
all-API soak result for tracking issue #192.

Run the failure-injection regressions with Python 3.11+ and `cryptography`:

```sh
python3 -m unittest discover -s conformance/devnet/soak -p 'test_*.py' -v
```

One test serves malformed JSON through a real local HTTP socket to the independent
Cosmos transaction client and verifies the actual workload records the failure
and request context. The other exercises repeated large exception messages to
check that persisted summaries stay bounded.

## All-API workload

`SOAK_PROVIDER_APIS=1` opts into independent traffic on Comet URI HTTP,
JSON-RPC HTTP, WebSocket, core gRPC, Cosmos gRPC, and REST on every node.
`make_net` still creates isolated nodes. The provider setup then gives every
listener a unique loopback address; native SDK gRPC and REST are explicitly
enabled, grpc-web disabled. Existing bank, blob, staking, governance, hash,
progress, and resource monitoring continue to run.

The Go `conformance/provider-client` uses standard HTTP, Gorilla WebSocket,
and grpc-go clients without either node implementation's client code. Each
probe cycle runs twelve operations concurrently on one node, including
historical block/commit requests, an ABCI proof request, bank pagination with
height metadata, and regular/one-second-delayed subscriptions. WebSocket
subscriptions close and reconnect twice. Each operation has a twelve-second
deadline and an eight-MiB response limit. These deliberately short delayed
readers exercise cancellation/reconnect; they do not establish queue saturation
or arbitrary backpressure safety. Probe output records status, errors, metadata,
elapsed time, byte count and response hash in `provider-result` events.

The independently signed bank workload rotates submissions across six
transports. Cosmos gRPC/REST use `BROADCAST_MODE_SYNC`; Comet uses sync;
core gRPC `BroadcastTx` waits for inclusion and retains CheckTx and execution
codes separately. Successful CheckTx results are queued for the existing tx-index
inclusion check. After any uncertain submission error, that account stops
submitting until a committed account-sequence query proves advancement; other
accounts continue. A deadline does not imply rejection or permission to resend.

`metrics.provider.operations` records requested, completed, errors and unknown
admission counts per node/operation, plus accepted/rejected CheckTx and failed
core execution counts for broadcasts. A rejected transaction can complete its
transport operation. `bank.ok` counts accepted CheckTx responses; `bank.included`
and `bank.failed_exec` come from the tx index. The final gate fails if any
node/operation completed no traffic, if provider errors occurred, or if accounts
retain unresolved admission outcomes. Errors during chaos remain explicit;
the report does not silently classify them as node implementation bugs or turn
them into successful requests.

For a bounded two-Go/two-Rust validator network plus a Rust full node, from the
repository root with the usual build dependencies and Python `cryptography`:

```sh
GOTOOLCHAIN=go1.26.9 APPD_ONLY=1 conformance/devnet/build-bins.sh
cargo build --locked --no-default-features -p celestia-rs
curl --fail --max-time 30 https://rpc.celestia.pops.one/abci_info > /tmp/current-mainnet-abci-info.json
SOAK_PROVIDER_APIS=1 SOAK_MAINNET_OBSERVATION=/tmp/current-mainnet-abci-info.json \
  SOAK_PROFILE=memory SOAK_SECS=300 SOAK_MIN_HEIGHTS=0 \
  SOAK_TX_RATE=10 SOAK_LOAD_ACCOUNTS=48 DEVNET_PORT_OFFSET=1000 \
  CELESTIA_RS="$PWD/target/debug/celestia-rs" SOAK_DIR=/tmp/provider-soak \
  conformance/devnet/soak.sh
```

Use an empty `SOAK_DIR`. Check the independently observed active app version
before building: this scenario currently requires version `10.5.0` and the
builder's matching successful module stamp. Source commits, compiler, build
flags and binary hashes must describe the binaries actually used; a module stamp
alone is rejected. The oracle's `.build.json` must match the binary hash,
pinned source commit and ledger build. `provider-provenance.json` records binary
SHA-256 values, Go build info, Rust version output, the Go builder recipe,
the mainnet observation with its file timestamp, genesis hash and
capture source hashes. Keep that file together with `report.json`, `report.md`,
`events.log`, resource samples and public node logs. Do not publish keyrings or
validator keys from node homes. A resuming run must preserve provider mode.

These are workload/availability probes, with chain hashes checked by the
existing soak. They do not compare every response byte at matched state, verify
proof cryptography, traverse all pagination cursors, use external provider SDKs,
or cover TLS/proxies, all transaction families on every transport, subscription
quota saturation and indefinite slow readers. These omissions appear in every
provider metrics report. The memory profile disables restart/partition chaos;
choose the existing chaos/long profile for those scenarios. No passing result is
claimed by adding this harness: fresh candidate results and unresolved findings
remain necessary to complete #192.
