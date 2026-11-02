# Current-release bounded baseline

This closes the investigation tracked by #181 at a reproducible, bounded local
baseline. It does **not** establish universal provider equivalence or silently
promote the ledger's strict exact-admission class. The compatibility target is
celestia-app v10.5.0, core v0.42.3 and SDK v0.52.12, freshly observed on mainnet;
all writes and pressure occurred on isolated devnets.

The issue's completion requirements are now represented by nineteen registered
packages covering every child issue #182–#192, plus the explicit coverage/CI
accounting from #193. Earlier scoped packages retain their original provenance
limits. The new packages supplement them with committed paired artifacts rather
than rewriting what earlier tests established.

| Completion requirement | Evidence |
| --- | --- |
| Actual Go/Rust transports and runtime identity | Existing reflection, limits, identity, simulation, empty-query, transaction, WebSocket and HTTP packages; registered source selectors and offline verifiers |
| Populated state, pagination, history, malformed/Any/UTF-8 and proofs | Ten fresh campaigns, 150,991 socket response comparisons, 259 method paths, 686 identical committed heights; 616 independently valid proofs and 24 explicit shared rejections |
| Persistence, pruning, disabled indexing and restart | Four profiles × three phases; 480 Comet and 1,008 gRPC/REST pairs, retained roots/data/indexes and exact stable error metadata |
| Reconnect and supported catch-up/state sync | Twenty-four paired NewBlock deliveries after socket closure/reconnect; ordered full-node catch-up; current Go→Rust→Go snapshot restoration |
| Pre-v10 state through the current release | Official Go 1.26.6 v10.5.0 multiplexer, real v9→v10 upgrade, 620 fresh historical comparisons with active-schema limitations |
| Relevant regression validation | Full ten-campaign replay/socket/proof test; api-server/grpc-transport suites and targeted all-target Clippy; existing scoped Python gates; workspace CI remains required |
| Bounded all-API mixed-node soak | Signed final candidate: 312 seconds, 112 height advances, 960 completed provider operations, zero client errors/stalls/hash failures; all nodes checked through111, four through114 |

The source-built captures use Go **1.26.9**; the official release multiplexer uses
**1.26.6**. Both identities and that deliberate baseline difference are recorded.
Signed source archives, native test executables' hashes, exact oracle/helper
receipts, public signed inputs, blocks, snapshot chunks and paired responses
remain available without relying on pre-squash commits staying fetchable.

The final changes fix the explicit protobuf response subtype and SDK error
header/trailer phase. Other deviations remain visible: narrow Go map-order and
diagnostic policies, earlier packages' accepted transport/parser deviations,
24 shared invalid proof results, and one `[]`/`null` empty validator-update
allocation in NewBlock events. No nonempty event list is normalized.

Operational conditions remain deliberate. Restart is validator-first; the
separate shared simultaneous-start follower stall remains preserved and
unresolved. Block-store pruning/discarded responses, every possible state-sync
configuration, old-binary historical query semantics, unbounded production
load, all message/input permutations and infrastructure/SDK combinations are
not certified. The soak's final Go consistency poll lagged three heights at
cutoff; those comparisons and cutoff-pending transaction inclusion are not
claimed. These conditions are part of the completed bounded investigation,
not evidence of universal readiness or a production deployment recommendation.

`scope-audit.json` binds every package, source selector, comparison class,
normalization, known mismatch and exclusion. Its reviewed canonical hash remains
in code. `coverage.py` retains the separate strict exact-admission contract and
continues to report universal readiness as not established. The CI job verifies
the new frozen module, persistent/historical/state-sync and final-soak packages
as well as the prior packages. The workspace job runs the Rust regressions;
passing static integrity alone does not substitute for those tests.

```sh
python3 conformance/mainnet-rpc/readiness.py
python3 conformance/mainnet-rpc/scope_audit.py --verify
python3 conformance/mainnet-rpc/coverage.py --run-current --verify-scoped
```

The first command checks the bounded completion record and its validation/source
bindings. The other commands execute the complete registered offline evidence
suite. None performs a fresh network capture or upgrades a comparison claim.
