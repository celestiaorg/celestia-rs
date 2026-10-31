# RPC version and build identity

The node reports two separate concepts: its Rust software build and the
celestia-app release targeted by its application API. A compatibility target
does not establish provider readiness or universal equivalence; that evidence
is tracked in [#181](https://github.com/celestiaorg/celestia-rs/issues/181).

| Surface | Value and meaning |
| --- | --- |
| ABCI `Info.data` (`/abci_info`) | `celestia-rs`, the implementation identity |
| ABCI `Info.version`, `/app/version` | `10.5.0`, the explicit celestia-app API compatibility target |
| ABCI `Info.app_version`, node-info numeric `app` protocol | The application protocol from committed consensus parameters; independent of either software release |
| `/status` and P2P `node_info.version` | `0.38.17-celestia-rs.<Rust package version>` |
| Cosmos gRPC/REST `GetNodeInfo.application_version` | Rust name, application name, package version, git commit, build profile and Rust compiler/target |
| CLI `version`, `version --long` | Rust software version; long output labels the application/core release target as `compatibility_target` |

The current core compatibility target is celestia-core v0.42.3. The node-info
software string retains the Comet API compatibility prefix `0.38.*`, allowing
clients that dispatch on that prefix (including CosmJS) to keep selecting the
Comet 0.38 adapter. Its suffix explicitly identifies the Rust implementation
and version; it does not claim to be a Go Comet build. P2P protocol
8 and block protocol 11 remain wire-protocol numbers, rather than software
versions. The `go_version` field of the Cosmos `VersionInfo` schema carries
the actual `rustc` version and target because the existing wire schema names
its compiler field that way. No Go compiler or Go module dependency list is
invented. Node startup registers the node binary's build information even
when called through the Rust library. A standalone `app` library without a
node registration reports that library's package version and leaves binary
build details empty.

These identity differences from Go are intentional: Go reports `celestia-app`
as its ABCI application/build name, Comet's Go software version as its
node-info version, and its Go build/toolchain/modules in `application_version`.
Clients should use `Info.version` or `/app/version` to select application API
features, and `application_version` to identify the implementation they are
actually operating. The compatibility target is a build-time constant; it
does not follow a live mainnet endpoint automatically and does not substitute
for checking the active release before collecting new evidence.

## Regression scope and retained limitations

`cargo test --locked -p celestia-rs --no-default-features --test runtime_metadata`
starts the production binary, commits real blocks, and reads Comet HTTP,
Cosmos gRPC and REST sockets. It checks app identity/version, chain and numeric
protocols, node moniker/index setting, build fields and CLI agreement. It also
checks a real `/net_info`, then performs an authenticated P2P handshake and
checks that the advertised application protocol, index setting, RPC address
and software identity agree with the configured node. The genesis is the
existing historical node fixture: this regression verifies runtime wiring and does not
claim matched-state parity with the current Go release.

The historical query corpus adapts only a successful `/app/version` whose
recorded payload is empty, and only when Rust returns the declared target and
all other ABCI fields match. The historical Comet corpus adapts only
`/abci_info.response.data` and an absent `version` from the unversioned Go
recordings. It preserves app version, height, hashes and timeout fields.
Current-release version responses require no empty-version adaptation.

The older corpus still constructs node info from a recorded Go `/status`,
drops Cosmos `application_version` in its comparison helper, and replaces
entire `consensus_state`/`dump_consensus_state` results with a marker. Those
tests do not verify production node metadata or even consensus-state shape.
The runtime regression and the current-release capture are the evidence for
the behavior above; the older exclusions must not be counted as exact
current-release metadata comparisons. A matched-state Go/RS peer comparison
would provide stronger evidence, but it is outside the field-shape and stable
metadata claims made here.

A separate current-source capture at
`conformance/node-metadata/v10.5.0/consensus.json` records actual Go HTTP
`status`, `net_info`, `abci_info`, `/app/version`, `consensus_state` and
`dump_consensus_state` responses, headers,
application source commit/module checksum, compiler, complete `go version -m`
output, and binary/genesis/recorder hashes. It was built with Go 1.26.9 from
v10.5.0 sources with standalone `ledger` tags and explicit release identity
ldflags (including version `10.5.0` and the full release commit). It excludes
the `multiplexer` tag and official release packaging; those differences are
preserved in the provenance. This is not an official release-binary capture.
The runtime regression compares exact stable `status`/`net_info` field sets
and scalar encodings, plus consensus-result/round-state field sets, scalar
encodings, proposer fields, vote-array field sets, and validator/key fields
to this capture. Heights,
rounds/steps, timestamps, hash/signature values, vote contents and list
cardinality vary across independent live nodes. Optional proposal/block/part
objects and live peer-round-state contents are not compared by this schema
regression. It does not substitute for a matched-state comparison of their
values.

Capture is explicit and never runs in an ordinary test. With a current-source
isolated devnet already running, regenerate from a shell with the pinned Go
compiler selected:

```sh
python3 conformance/node-metadata/record.py \
  --rpc http://127.0.0.1:27657 \
  --binary /path/to/celestia-appd \
  --genesis /path/to/devnet/n0/config/genesis.json \
  --output conformance/node-metadata/v10.5.0/consensus.json
```

The recorder requires the successful pinned-source `.module` stamp and
`.build.json` recipe emitted by `conformance/devnet/build-bins.sh`. It checks
the release commit, core/SDK dependencies, build tags and running application
version, and accepts only a loopback HTTP listener. Before updating the
capture, independently verify that the selected application release is still
active on mainnet.

REST build fields are checked in the runtime regression, but metadata
forwarding remains a separate gap: the Go gateway emits
`Grpc-Metadata-Content-Type` and `Grpc-Metadata-X-Cosmos-Block-Height`; the
current Rust REST handler does not forward them. Full header/error-path
coverage is tracked by [#191](https://github.com/celestiaorg/celestia-rs/issues/191),
and this test does not mask that difference.
