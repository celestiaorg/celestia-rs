# Fresh authz and feegrant query slice

This is a bounded addition to #188 and #181. An isolated celestia-app v10.5.0
devnet ran the authz/feegrant workload through height 89. The public inputs retain
generic, send and stake authorizations, nested execution, spend limits, expiry,
revocation, basic/periodic/allowed-message fee allowances and explicit pruning
transactions. The current release generated every ABCI response and state root;
historical golden answers were not relabelled.

The oracle is celestia-app commit `6013e758d5dceb75827ad191c8d739d7c43cfa04`
with the `ledger` build tag, celestia-core v0.42.3 at
`a5537d84a008d566abc503d2ea41e30f2301403d`, and Go 1.26.9. The compiler differs
from issue #188's original Go 1.26.6 baseline. A fresh read-only mainnet
observation selects v10.5.0. The receipt binds raw observation bodies, source,
build, genesis, ABCI stream, block archive, state snapshots and query corpus.
The initial account mnemonics and validator/node keys are the existing public
devnet-only inputs; additional workload keys and live block timestamps are
recorded in the resulting public transaction/genesis/block inputs.

Replaying those same inputs into a fresh Go application produced 3,536 ABCI and
2,285 REST cases. The Rust regression replays all 89 heights and requires every
app hash to agree before comparing 3,520 ABCI and 2,274 REST cases. Sixteen ABCI
and eleven REST consensus-backed cases are explicitly excluded by the shared
application-only query harness. Unlike the historical corpus tests, this fresh
test permits neither the legacy empty-version adapter nor the old malformed
Simulate gas exception.

The shared comparator still sorts its documented map-order lists, strips Go
diagnostic source frames and removes build identity from GetNodeInfo. REST JSON
is parsed and compared with the same documented list/build rules. Proof fields
and protobuf response values remain part of the ABCI comparison. These are
application/gateway-handler regressions: the Go capture uses real ABCI and REST
listeners, while the Rust comparison invokes the app and gateway handlers.
It does not establish Rust socket metadata or Cosmos gRPC/Comet transport parity.
Proof-byte comparison is not independent ICS23 validity verification. Other
module campaigns, malformed-state campaigns and universal provider parity remain
outside this slice.

Regenerate with an unused local port offset and a fresh observation:

```sh
MAINNET_OBSERVATION=/tmp/mainnet-observation.json \
  bash conformance/mainnet-rpc/populated-authz/record.sh
```

The wrapper uses a separate temporary `CELESTIA_APP_HOME`: Go client commands
pre-instantiate an app before executing their requested command. Validator keys,
local node databases and that incidental CLI state are not committed.

Verify the receipt, then run the actual Rust regression:

```sh
python3 conformance/mainnet-rpc/populated-authz/verify.py
cargo test --locked -p devnet-conformance --test query_corpus \
  corpus_v105_authz_feegrant -- --exact --nocapture
```

Passing receipt validation establishes integrity and the stated capture scope.
Source hashes are checked against the preserved capture sources in merge commit
`8f9729c9a300751b09679e5f48133bd652a7553f`, whose bytes match the original
pre-squash capture receipt. Artifact hashes still cover the published files.
Later dependency changes do not rewrite the historical lockfile receipt; a full
Git checkout containing that merge commit is required for source verification.
The separate Rust test establishes its recorded replay/comparison result. This
package is not admitted as complete provider readiness by the evidence ledger.
