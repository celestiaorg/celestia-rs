# Authentic evidence-bearing block

This bounded fixture comes from `conformance/devnet/double-sign-evidence.sh`:
two Go and two Rust validators commit a real `DuplicateVoteEvidence` after the
byzantine peer sends conflicting signed prevotes to a Rust validator. Capture
runs only after the mixed chain's block/app hashes agree and the double signer
has left the validator set. It contains no private validator keys.

This recording contains two distinct proofs for the double signer at vote
height 7, committed together at height 9. The validator's normal vote can
conflict with both votes from the byzantine peer, so a run may commit more than
one proof. Capture preserves the complete evidence list.

`meta.json` identifies the evidence block, evidence hash, double signer, vote
height, chain ID, source commits/module checksums, binary hashes, capture source
hashes, and SHA-256 of each captured file. `go-build-info.txt` records both Go
binaries' toolchains and dependencies. The node sources are celestia-app
v10.5.0 (`6013e758d5dceb75827ad191c8d739d7c43cfa04`) and celestia-core
v0.42.3 (`a5537d84a008d566abc503d2ea41e30f2301403d`). The Rust runtime source
revision is recorded separately from the capture script hashes.

## Regenerate

From the repository root with Rust stable, Go's toolchain downloader, Python 3,
curl, and the normal native build dependencies installed:

```bash
GOTOOLCHAIN=go1.26.9 APP_VERSION=v10.5.0 CORE_VERSION=v0.42.3 APPD_ONLY=1 \
  conformance/devnet/build-bins.sh
GOTOOLCHAIN=go1.26.9 RUST_LOG=info DEVNET_NO_DEFAULT_FEATURES=1 EVIDENCE_FIXTURE_OUT=/tmp/full-block-evidence \
  conformance/devnet/double-sign-evidence.sh
```

The output directory must be empty. The script builds the Rust node and
`double_sign` peer with redb only and rebuilds the Go recorder. The API server
forwards its optional RocksDB feature so the node's `--no-default-features`
build does not pull in native storage implicitly. Unset
`DEVNET_NO_DEFAULT_FEATURES` for the default native build. `DEVNET_BIN` and
`DEVNET_PORT_OFFSET` retain their usual meanings. The Go binary must have the
successful-build version stamp produced by `build-bins.sh`; capture rejects
different app/core versions. Copy the resulting five files here after reviewing
their identities, preserve this README, and update the regression's pinned
fixture identity. Network keys, timestamps, proposer ordering, and consequently
the evidence height and hashes vary between runs; regeneration reproduces the
scenario rather than identical random chain bytes.

## Contents and regression

- `genesis.json`: the Go node's genesis.
- `blocks.bin.gz`: the unchanged `rpc-recorder dump` stream format, two
  uvarint-length-prefixed protobufs per height (block, seen commit), bounded at
  `EV_H` with `-through`.
- `validation.json`: original Go database protobuf records for validator sets
  and consensus parameters from initial height through the evidence block
  (including the next validator set), plus the pre-evidence `stateKey` and the
  exact Go duplicate-evidence rejection. Core retains only its latest `State`;
  the recorder reconstructs the earlier state's block ID/time, hashes, protocol
  version, validator sets, parameters, and change heights from committed
  headers and persisted state history. Capture enables ABCI response retention
  on the source Go node; app/results hashes and timeouts come from the original
  pre-height `FinalizeBlock` response. Go's full
  `BlockExecutor.ValidateBlock` must accept this reconstructed state/block and
  reject a copy with duplicated evidence and recomputed evidence hash before
  capture succeeds. The evidence DB is fresh, without post-height committed
  markers from the live source chain.
- `go-build-info.txt` and `meta.json`: provenance and integrity bindings.

Run `cargo test --locked -p evidence --test full_block`. The test verifies every
recorded seen commit against its historical validator set, rebuilds the block
store only through `EV_H - 1`, imports the state history into a real redb state
store, closes/reopens both stores, and constructs a fresh persistent
`evidence::Pool`. It passes `Pool::executor_handle()` to `BlockExecutor` and
requires the authentic block to validate and its evidence to persist as pending.
After reopening the pool, a block with duplicated evidence and a recomputed
`evidence_hash` passes pure block validation but must fail full validation with
Go's exact duplicate-evidence error. ABCI methods panic if called: this is a
validation regression, with no application replay.

This closes the full-block/evidence-pool composition coverage gap. It does not
establish production qualification, candidate volume/replay coverage, no open
findings, or three consecutive acceptance nights. The application-level
`fixtures/devnet/evidence` replay and isolated evidence vectors retain their
separate scopes.
