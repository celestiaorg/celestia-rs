# Current-release RPC evidence

This directory is the evidence ledger for [#181](https://github.com/celestiaorg/celestia-rs/issues/181)
and its provider compatibility issues. It separates source declarations, historical
regression fixtures, and admitted current-release comparisons. **A green inventory
check does not establish provider readiness.**

The selected oracle is celestia-app v10.5.0 at
`6013e758d5dceb75827ad191c8d739d7c43cfa04`, celestia-core v0.42.3 and
Cosmos SDK v0.52.12. Recheck an active mainnet node before each fresh isolated
capture. Record the actual compiler; do not substitute the `go` language version
from `go.mod` for the binary's toolchain. Issue #181 originally used Go 1.26.6;
the repository's oracle build scripts now select Go 1.26.9. Evidence must state
that difference rather than attributing a new capture to the older compiler.

## Offline inventory and CI

From the repository root:

```sh
python3 -m unittest discover -s conformance/mainnet-rpc/tests -v
python3 conformance/mainnet-rpc/coverage.py --json /tmp/rpc-coverage.json
python3 conformance/mainnet-rpc/coverage.py --run-current
```

These commands perform no fresh network capture. The CI `rpc-evidence` job
validates the ledger and runs the exact regression selectors registered by
admitted current-release manifests. It rejects a selector that selects zero
tests. With no admitted manifests it explicitly reports zero current-release
regressions, not a parity pass. Existing workspace tests retain their separate
regression role.

`policy.json` identifies every remaining behavior area and issue, and lists
comparison policies, known mismatches and exclusions separately. The JSON
inventory enumerates:

- Comet URI, JSON-RPC and WebSocket declarations from `rpc/src/env.rs`, including
  the optional unsafe routes; subscriptions are WebSocket-only.
- Core gRPC handler paths from `rpc/src/grpc.rs`.
- Cosmos unary descriptor candidates from generated `services.rs` and REST
  binding candidates after the gateway's exclusion list.
- Both streaming reflection versions on both listener types as required
  registration checks, even when the unary method table cannot represent them.

A descriptor candidate is **not** proof that Go or the production RS listener
registers the service. A registration check may establish that a version or
service is intentionally absent; an unknown-service error is evidence to capture,
not permission to expose the service. For each endpoint, `unassessed_behaviors`
means the behavior's applicability and coverage still need review. It does not
assert every behavior applies to every endpoint. Case counts never imply branch
completeness.

The committed RPC v10.4.0 and query v9/v10.4 corpora remain regression-only.
The inventory records their original metadata and hashes and identifies missing
capture provenance instead of filling it with today's dependency versions.
`evidence-reviews.json` audits the merged v10.5 packages for #182 and #184-#188.
It hashes their available build receipts, inputs, capture sources, Go responses,
state receipts and exact regression source/selectors. The review validator also
requires every missing admission artifact to remain explicit. These records are
`not-admitted`: they appear in `reviewed_not_admitted` in the JSON report and do
not add cases or selectors to `current_evidence`.

The review found no package that satisfies the complete admission contract:

- #182 has pinned Go reflection transcripts and runtime replay, but no committed
  RS binary, matched-state receipt or RS response capture. Its two narrow
  normalizations and the upstream Go crash diagnostic remain explicit.
- #184 deliberately compares consensus field sets and scalar types while
  excluding asynchronous values; its own capture scope disclaims matched state.
- #185 reconstructs and checks the Go state before a live Rust Simulate test,
  but does not commit the Rust state/binary/responses and intentionally removes
  Go source frames from diagnostic text.
- #186 compares live Rust responses with committed Go URI, JSON-RPC, WebSocket
  and ABCI fixtures, but has no committed Rust capture or matched state. Its
  embedded release observation is not a complete raw status/ABCI observation.
- #187's partial BroadcastTx regressions use Rust fixtures and a local core
  stand-in, without a pinned Go capture or paired state.
- #188 has a strong Go four-transport populated-staking capture and committed
  Go state, but no Rust matched-state or response capture. Its proof and broader
  query gaps remain recorded by that fixture.

This audit does not infer missing identities from release tags, convert a live
test result into a committed response artifact, or turn schema/diagnostic checks
into exact comparisons. A later capture can replace a blocker with an admitted
manifest only by committing the corresponding evidence.

## Current behavior checklist and scoped CI gate

`scope-audit.json` separately indexes the merged evidence available at its exact
`audited_main` revision. The strict paired-response admission contract above is
unchanged. A scoped package is a hash-bound review of what the capture and its
tests establish; it does not add an exact case or satisfy every branch of its
tracking issue. `scope_audit.py` requires the complete package/transport/selector
set and every issue #182–#192, including explicit remaining gaps for issues with
no merged current-release campaign.

| Issue | Current capture/test scope | Comparison limit |
| --- | --- | --- |
| #182 | Port-specific reflection streams and descriptors | Narrow ordering/whitespace normalization; upstream crash diagnostic excluded |
| #183 | Custom unary/streaming byte limits and listener configuration | Current Go capture and unmatched Rust operator tests; resource campaign incomplete |
| #184 | Production identity and consensus schema | Build identity intentionally differs; asynchronous values unconstrained |
| #185 | Fourteen Simulate error/success/height cases | Go diagnostic source-frame deviation; paired runtime artifacts unrecorded |
| #186 | Public empty-envelope errors and reusable connections | Exact runtime replay with incomplete paired capture provenance |
| #187 | Seventeen Go exchanges replayed in sequence through Rust production listeners after two admitted state roots | Explicit cleanup CheckTx 12-gas difference; original envelope bytes and final root/balance equality excluded; broader matrix remains open |
| #188 | Eleven populated staking/reward cases across transports, Rust committed-root proof check | One list-order normalization; other modules and Go proof bytes remain open |
| #189 | No merged current-release persistence campaign | Historical tests and local work are not substituted for capture evidence |
| #190 | One bounded Go subscription/quota/deadline/reconnect cycle plus Rust cleanup regressions | Relational timing/order/resource checks; sustained workload remains open |
| #191 | REST height, REST POST and Comet HTTP boundary packages | REST callback/Go-only subsets and explicit Hyper431 diagnostic difference |
| #192 | Merged310-second mixed-node all-API soak:111 heights,990 provider operations | Candidate809bfca, explicit disabled scenarios and cutoff accounting; no exact paired response parity |

Run the scope gate independently:

```sh
python3 conformance/mainnet-rpc/scope_audit.py --verify
python3 conformance/mainnet-rpc/coverage.py --verify-scoped --json /tmp/rpc-coverage.json
```

The `rpc-evidence` CI job now runs the registered offline current-release
regressions for limits, WebSocket lifecycle, live transactions, populated staking,
REST height/POST, Comet HTTP and the merged bounded provider soak. Their verifier sources and primary receipts are
hash-bound. The full reviewed audit content is canonically hash-bound in code,
including its merged baseline, deviations, provenance limits and remaining-gap
text. Rebinding the baseline or hiding a gap requires an explicitly reviewed
code change. It runs no fresh network capture. The ordinary workspace test job
also executes the named Rust socket/production/proof regressions; the audit binds
their exact source files and function names without creating a second Rust
workspace build. A passing offline verifier confirms its stated captured
contract and provenance, not untested provider equivalence.

The scope audit records normalizations, demonstrated differences, exclusions and
missing provenance separately. In particular, the older query corpus still
contains its historical Simulate skip and stale gas-zero rationale. That retained
skip is not evidence of a current Simulate mismatch: the merged fresh14-case
contract is recorded separately. No source code or historical answer is silently
changed to erase this distinction.

## Fresh oracle selection and capture

Oracle selection is a separate, explicit read-only network operation:

```sh
python3 conformance/mainnet-rpc/observe-mainnet.py \
  --endpoint https://rpc.celestia.pops.one \
  --out /tmp/mainnet-observation.json
```

This records the raw `/status` and `/abci_info` bodies with hashes and time,
checks chain ID `celestia`, and refuses a version different from policy. It does
not submit transactions or establish provider parity. If the active version
changes, review the release commit, dependencies, compiler, capture tooling and
policy before selecting a new oracle. Keep old evidence under its original
identity; do not relabel it. The capture must follow the observation within 24
hours; longer campaigns need a fresh observation before a new capture batch.

Run oracle and candidate nodes only in isolated local devnets. Use the same
genesis and workload, confirm chain ID, committed height and app hash, and record
requests and responses through real listeners. Capture statuses, stable headers,
metadata, frames and terminal errors as well as response bodies. Record failures
without inventing empty successful responses. Do not use public mainnet as the
transaction, malformed-input or resource-pressure test target.

Add a manifest to `policy.json`'s `current_evidence` list only after the
regression exists. Each manifest uses the following fields:

```json
{
  "oracle": {
    "celestia_app": "v10.5.0",
    "celestia_app_commit": "6013e758d5dceb75827ad191c8d739d7c43cfa04",
    "celestia_core": "v0.42.3",
    "celestia_core_commit": "a5537d84a008d566abc503d2ea41e30f2301403d",
    "cosmos_sdk": "v0.52.12",
    "cosmos_sdk_commit": "2dd23a60cf34c3ab90c8a4f15a7f35d9b2ac69b4",
    "compiler": "go1.26.9",
    "build_tags": []
  },
  "binary_sha256": {"go": "<SHA-256>", "rs": "<SHA-256>"},
  "inputs": {
    "genesis": {"path": "<repo-relative file>", "sha256": "<SHA-256>"},
    "workload": {"path": "<repo-relative file>", "sha256": "<SHA-256>"},
    "capture_source": {"path": "<repo-relative file>", "sha256": "<SHA-256>"},
    "mainnet_observation": {"path": "<repo-relative file>", "sha256": "<SHA-256>"}
  },
  "matched_state": {
    "go": {"chain_id": "<local chain>", "height": 20, "app_hash": "<64 hex digits>"},
    "rs": {"chain_id": "<same chain>", "height": 20, "app_hash": "<same hash>"}
  },
  "capture_command": "<reproducible isolated capture command>",
  "captured_at": "<ISO 8601 time with timezone>",
  "cases": [{
    "transport": "comet-uri",
    "endpoint": "status",
    "behavior": "runtime-identity",
    "comparison": "exact",
    "outcome": "exact",
    "request": {"path": "<request artifact>", "sha256": "<SHA-256>"},
    "go_response": {"path": "<complete Go response artifact>", "sha256": "<SHA-256>"},
    "rs_response": {"path": "<complete RS response artifact>", "sha256": "<SHA-256>"},
    "test": {"package": "rpc", "target": "<integration test>", "filter": "<exact test name>", "source": "<test source file>"}
  }]
}
```

The placeholders above are documentation, not valid evidence. Paths must resolve
to committed repository files; binary hashes record build identities without
committing binaries. Commit the capture source, public inputs and responses,
never private validator keys or local node databases. The capture command/source
must preserve all source and build flags needed to reproduce those binaries.
For non-exact cases use outcome `normalized`, `mismatch`, or `excluded`, record a
specific `reason`, and select a documented comparison policy. New semantic
normalizations need their own narrow policy and linked issue. Exact comparisons
may record mismatches; they must not be relabelled normalized successes.

## Provider readiness review

The ledger always reports readiness as not established. A maintainer must assess
complete behavior coverage and accepted deviations before changing that claim.
The review requires:

- Current oracle identity, reproducible input/capture provenance and actual Go/RS
  listener registration on every transport.
- Success/error contracts over populated state, including historical/proof and
  pagination branches, with committed-root and matched-height verification.
- Transaction admission/inclusion/failure, subscription lifecycle, default/custom
  operator limits, cancellation, bounded allocation and capacity recovery.
- Persistent restart/pruning/index/catch-up cases and narrow, explicit dynamic
  field and build-identity policies.
- Full relevant regression validation and a bounded all-API mixed-node soak with
  classified client failures, hashes, progress, resources and excluded scenarios.
- Each issue #182–#192 satisfied by reviewable evidence, and a distinction between
  local changes, merged code and released behavior.

Until then, green samples, source declarations and ledger validation remain
scoped evidence, not a promise of universal provider equivalence.
