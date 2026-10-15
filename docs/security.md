# Dependency security triage

Last triage: 2026-10-14. Rust: `cargo deny check advisories` (cargo-deny
0.18.9, RustSec advisory DB of that day); Go: `govulncheck` v1.1.4 on every
module under `conformance/` against the OSV Go database (symbol-level:
*called* = reachable from the module's code, *imported* = package imported
but vulnerable symbol not reached, *module* = only required).

Nothing in this file affects the node binary except the Rust section: the
Go modules are test tooling (fixture generators, oracles, recorders). Their
outputs are committed fixtures, so a Go dependency is bumped only when the
generator's output provably stays byte-identical.

## Rust (workspace)

CI runs `cargo deny check` (advisories included again: cargo-deny >= 0.18
parses CVSS 4.0 advisories, which crashed older versions).

| Advisory | Crate | Action |
|---|---|---|
| RUSTSEC-2026-0190 (unsound `downcast_mut`) | anyhow 1.0.102 | upgraded to 1.0.104 |
| RUSTSEC-2026-0009 (DoS, stack exhaustion) | time 0.3.36 | upgraded to 0.3.55 (needs rustc 1.88: workspace `rust-version` / clippy MSRV raised from 1.85 to 1.88; the toolchain is `stable`) |
| RUSTSEC-2026-0002, RUSTSEC-2026-0253 (unsound `IterMut`, `pop`) | lru 0.12.5 (iavl node cache) | upgraded to lru 0.18.5 |
| RUSTSEC-2026-0253 | lru 0.7.8 via reed-solomon-erasure 6.0.0 (block propagation, latest release) | **accepted**: reed-solomon-erasure only calls `LruCache::get` / `put`; the unsound `pop()` path is never reached |
| RUSTSEC-2024-0384 (unmaintained) | instant 0.1.13 via parking_lot 0.11 via reed-solomon-erasure | **accepted**: no maintained release of reed-solomon-erasure |
| RUSTSEC-2024-0436 (unmaintained) | paste 1.0.15 (proc macro) via ark-ff (x/zkism) and tendermint-proto | **accepted**: build-time only |

The accepted ones are the `ignore` list of `deny.toml`, each with its reason.

## Go (`conformance/*` test tools)

### Fixed

- **Go toolchain** of every module: `toolchain go1.26.9` (was go1.24.7 to
  go1.26.6). This removes all *called* standard-library findings (net/http
  HTTP/2 and HTTP/1 desync / DoS, crypto/tls, crypto/x509, net/url,
  encoding/asn1, html/template, ...: GO-2026-6603, -6605, -6607, -6608,
  -6610, -6611, -6612, -6613, -6617 and, for fixturegen's old go1.24.11,
  ~20 more).
- **golang.org/x/net** v0.28 / v0.58 / v0.59 → v0.60.0 everywhere it is in
  the graph (GO-2026-6603, -6611, -6612, -6617), with the x/crypto, x/sys,
  x/text, x/sync, x/term versions MVS pulls along.
- **fixturegen** (M0-M2 fixtures): cosmos-sdk v0.50.10 → v0.50.14
  (GO-2024-3339, GO-2025-3476/3516/3803), cometbft v0.38.12 → v0.38.21
  (GO-2026-4361, GO-2025-3443/4025), cosmossdk.io/math v1.3.0 → v1.4.0
  (GO-2024-3279), grpc v1.64.1 → v1.83.1, klauspost/compress → v1.18.7.

Byte-identity of the outputs after the bumps (regenerated into a temp dir
and compared with the committed fixtures):

- fixturegen: all 71 offline scenarios identical (`square-mainnet-blocks`
  and `m2-integration-state-diffs` fetch mainnet data and cannot run here;
  their generators only use hashing / encoding code the other scenarios
  cover).
- ctypesvectors (10 files) and evidencevectors: identical.
- lightvectors: the output with the old and the new dependency set is
  identical; it differs from the committed `client.json.zst` in both cases
  (generator/environment drift unrelated to the bump, so the fixture was
  not regenerated).
- p2pvectors: identical except the address book's random `key`, which the
  generator draws fresh on every run (with or without the bump).

### Intentionally left

| Module | Remaining findings | Why |
|---|---|---|
| `antereplay` | called: go-ethereum v1.15.8 (GO-2026-4314/4315/4507/4508), otel v1.39.0 (GO-2026-4394/5506/6505), grpc v1.78.0 (GO-2026-6061/6348), go-jose v4.1.3, go-getter v1.7.9, aws eventstream v1.7.4, ulikunitz/xz v0.5.14 | The M3 ante replay oracle pinned to celestia-app **v6.4.10**'s exact dependency set (its replace directives) so it reproduces v6's ante outcomes. It only regenerates the mainnet tx corpus, which needs public mainnet RPC access (`OPEN_QUESTIONS.md` #1), so byte-identity after a bump cannot be checked. The vulnerable code is reachable only through celestia-app's package initialisation; the tool reads fixture files and never serves the network. |
| every module | golang.org/x/crypto `openpgp` (GO-2026-5932), *module* level | No fixed version (package deprecated); not imported. |
| fixturegen | cometbft v0.38 blocksync (GO-2025-3442, fixed only in v1.0.1), cosmos-sdk x/crisis (GO-2023-1821/1881, no fix), grpc GO-2026-6443 (*imported*) | Not reachable from the generators; cometbft v1 would change the Go types the fixtures are generated from. |
| antevectors, txdecode, mintvectors, consensusvectors, ctypesvectors, evidencevectors, lightvectors, p2pvectors, rpccorpus/recorder | only the x/crypto `openpgp` entry | Pinned to celestia-app v10.4.0 / celestia-core v0.42.3 (the parity target); nothing reachable remains. |

## Fuzzing

Two harnesses, both run nightly (`.github/workflows/nightly.yml`, jobs
`fuzz-diff` and `cargo-fuzz`). Findings are fixed in the crates by porting
Go and pinned by regression tests that run on stable without Go.

### Differential fuzzing against Go (`crates/fuzz-diff`)

`conformance/fuzzoracle/{v10,v9}` is celestia-app **v10.4.0** / **v9.0.8**
(their exact module graphs, go-square v4.0.1 / v4.0.0-rc4, celestia-core
v0.42.3 / v0.40.9) built as a line server: `<op> <hex>` in, one JSON
outcome out. `fuzz-diff` sends the same inputs to it and to the Rust code
and compares:

| op | Go | Rust | compared |
|---|---|---|---|
| `tx` | `TxConfig.TxDecoder()` (panics recovered like `BaseApp`) + `GetSigners` | `txdecode::decode_for` + `x_ante::txview::signers` | accept/reject + ABCI code, inner tx, raw and gogoproto-re-encoded body / auth info / msgs, signatures, unknown-non-critical flag, blobs, share indexes, signers |
| `blobtx` | go-square `tx.UnmarshalBlobTx` | `txdecode::unmarshal_blob_tx(_v9)` | is-blob, error kind, contents |
| `iw` | celestia-core `UnmarshalIndexWrapper` | `txdecode::unmarshal_index_wrapper` | all |
| `fibre` | `x/fibre/types.TryParseFibreTx` (v9: rc4 `tx.TryParseFibreTx`) | `app::fibre_tx::try_parse(_v9)` | is-fibre, error, system blob |
| `eds` | `da.ConstructEDS` + `NewDataAvailabilityHeader` | `app::proposal::block_data_root_with_max` | square width + data root, or error |
| `amino_dec` / `amino_enc` | authtx `TxDecodeAmino` / `TxEncodeAmino` | `legacy_amino` | output, error and panic text |

Inputs: every tx of every devnet fixture (`CheckTx`, proposals, blocks),
the txdecode and amino vectors, and any extra corpus directory
(`--corpus`, e.g. a cargo-fuzz corpus), put through protobuf-aware
mutations (`fuzz-diff/src/mutate.rs`: recursive field edits, boundary
varints, unknown and non-critical fields, wrong wire types, non-minimal
varints, groups, invalid UTF-8, splices). `eds` inputs are random tx sets
(real txs, synthetic blob txs at share-boundary sizes, random max square
sizes) and every devnet block.

`fuzz-diff pp` (`crates/fuzz-diff/pp-diff.sh <scenario>`) is the
ProcessProposal differential: the recorded scenario is re-driven into a Go
`celestia-appd` (standalone ABCI) and the Rust app in lockstep, and before
each recorded `ProcessProposal` mutated variants (txs dropped, reordered,
duplicated, mutated, borrowed from other heights; DAH recomputed) are sent
to both at the same state.

```bash
# Go oracles (once; GOPATH/GOTOOLCHAIN as for the other conformance modules)
for v in v9 v10; do (cd conformance/fuzzoracle/$v && GOFLAGS=-mod=mod \
  go build -o ../../../target/fuzzoracle/fuzzoracle-$v .); done
cargo build --release -p fuzz-diff
target/release/fuzz-diff --app-version 10 --ops tx,blobtx,iw,fibre --iters 500000 --seed 1 --findings /tmp/f
target/release/fuzz-diff --app-version 9  --ops eds --batch 64 --max-secs 600 --findings /tmp/f
target/release/fuzz-diff --app-version 10 --ops amino_dec,amino_enc --iters 100000
GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/pp-diff.sh blob 30
# pin a finding: record Go's outcome into the regression file
echo "tx <hex>" | target/release/fuzz-diff record 10 "what it shows" >> crates/fuzz-diff/tests/regressions.jsonl
```

`crates/fuzz-diff/tests/regressions.jsonl` holds every finding with Go's
recorded outcome; `cargo test -p fuzz-diff` replays them without Go.

### Crash-only fuzzing (`fuzz/`, cargo-fuzz)

libFuzzer targets for untrusted input; a panic, stack overflow, allocation
above `-rss_limit_mb` or a timeout is a bug. Not a workspace member (needs
nightly); targets are grouped by feature so a build compiles only what it
needs.

| target | feature | surface |
|---|---|---|
| `txdecode` | | full decoder chain at both app versions, wrappers, duplicate-field check |
| `amino` | | `TxDecodeAmino` / `TxEncodeAmino` (ABCI / gRPC / REST query) |
| `square` | | go-square v4 construction, erasure coding, DAH, share parsing |
| `shares_parse` | | compact / sparse share parsing, namespace ranges |
| `abci_frame` | `net` | ABCI socket framing and `Request` decoding |
| `p2p_handshake` | `net` | SecretConnection handshake with a hostile remote |
| `mconn_recv` | `net` | MConnection packet parsing and chunk reassembly |
| `p2p_msgs` | `net` | every reactor's message decoding + `ValidateBasic` (consensus, propagation, state sync, block sync), node info, net addresses, PEX |
| `light_json` | `rpc` | light / state-sync RPC client: HTTP framing, CometBFT JSON |
| `rpc_request` | `rpc` | JSON-RPC bodies, params of every type, URI args, event queries |

```bash
rustup toolchain install nightly --profile minimal && cargo install cargo-fuzz
cargo +nightly fuzz run --sanitizer none txdecode /tmp/corpus fuzz/corpus/txdecode -- -max_total_time=600
cargo +nightly fuzz run --sanitizer none --features net mconn_recv /tmp/c fuzz/corpus/mconn_recv -- -max_total_time=600
```

Seed corpora (`fuzz/corpus/<target>`, < 1 MB) come from
`fuzz-diff seed-corpus fuzz/corpus` (devnet txs, amino vectors, blocks) and
hand-written network messages.

### Findings (2026-10-15)

Fixed (each pinned by a test; consensus-relevant ones also by
`crates/fuzz-diff/tests/regressions.jsonl` with Go's recorded outcome):

| # | Where | Finding | Impact | Fix / test |
|---|---|---|---|---|
| 1 | `x-ante` `GetSigners` (both versions) | Rust decoded every message with prost (strict UTF-8 everywhere). Go's `anyutil.Unpack` uses the generated pulsar unmarshal for the 116 types `protoregistry.GlobalTypes` knows (no UTF-8 check) and `dynamicpb` for the rest, which rejects invalid UTF-8 only in fields whose resolved descriptor is proto3 (ibc `UpgradeFields` is not) and rejects 10-byte varints with a last byte above 1 (gogoproto accepts them) | a signed tx Go runs (fees, sequence, execution) failed Rust's ante, or the reverse: app-hash divergence | `getsigners_schema.rs` dumped from the Go binary (`v2types`, `utf8schema`), `txview::tx_msg_signers`; regressions |
| 2 | v9 `ProcessProposal` / `ConstructEDS` | go-square v4.0.0-rc4 `Construct` classifies txs itself with `TryParseFibreTx`, decoding go-square's trimmed `Tx{TxBody body}` with protobuf-go: a `TxRaw` repeating `body_bytes` (the SDK decoder keeps the last body) whose merged bodies start with a `MsgPayForFibre` is laid out as a pay-for-fibre tx with a system blob (or fails the square when malformed); Rust laid it out as a normal tx | a malicious v9 proposer's block: Go validators accept, Rust rejects (different data root) | `fibre_tx::try_parse_v9`, `proposal::classify_txs_v9`; regressions |
| 3 | v9 square construction | rc4 has no `ClassifiedTx.Validate`: an empty normal tx is laid out; Rust rejected it | ConstructEDS / proof parity | `square::v4::construct_rc4`; regression |
| 4 | v9 decoder | hyperlane-cosmos v1.1.0 `RemoteRouter.receiver_contract` is a plain string (v1.3.0: `HexAddress` customtype): Rust failed to decode `MsgEnrollRemoteRouter` with a non-hex receiver and re-encoded an empty one as a zero address | tx decode divergence at app version 9 | `txdecode` v9 schema patch; regressions |
| 5 | v10 `TryParseFibreTx` | prost decoding of `TxRaw` / `TxBody` / `MsgPayForFibre` rejected invalid UTF-8 (e.g. in the memo) that gogoproto accepts, hiding a pay-for-fibre tx; the signer used the bech32 crate instead of btcutil `DecodeToBase256` (90-character limit) | square layout divergence | `txdecode::gogo_unmarshal`, `encoding::bech32::decode_to_base256`; regressions |
| 6 | RPC (`rpc::json`) | recursive JSON parser: a JSON-RPC body of nested `[` overflowed the worker stack and aborted the node; Go fails the 10 001st level with `exceeded max depth` | remote crash through the public RPC port | iterative parser with Go's limit, iterative drop and id formatting; `rpc/tests/deep_json.rs` |
| 7 | `legacy-amino` JSON scanner (`TxEncodeAmino`) | same recursion | remote crash through ABCI / gRPC / REST queries | iterative scanner with Go's limit; `gojson` unit test |
| 8 | `legacy-amino` codec | recursion per nested value: a `StdTx` with `MsgExec` nested a few thousand levels (Go decodes 8000) overflowed a 2 MiB query-handler stack | remote crash through queries | large-stack thread for inputs > 4 KB; `legacy-amino/tests/deep_nesting.rs` |

Open:

- **Invalid UTF-8 in executed messages** (found while fixing #1): the
  router decodes each message with prost, which rejects invalid UTF-8 in
  any string, while Go's handlers get the gogoproto struct and store the
  bytes (e.g. a validator moniker, a proposal title). Such a tx now passes
  the ante handler like in Go but fails with `tx parse error` in Rust
  where Go executes it: app-hash divergence a user can trigger with one
  tx. An exact fix needs messages whose string fields keep arbitrary bytes
  (prost `String` cannot); the decoded-struct normalization
  (`Tx::normalized_msgs`) would only make the stored bytes differ.
- The same for messages executed through `MsgExec` / gov proposals (their
  signers are decoded from the raw inner bytes, which is stricter than Go
  only where Go executes the message anyway).
- v9 `x/warp` handlers still treat `receiver_contract` as a hex address
  (ADR 0004 "Not ported").
- Go itself: `TxDecodeAmino` of a 2 MB `StdTx` with ~20 000 nested
  `MsgExec` grew the Go process past 10 GB (quadratic copying in
  go-amino); the oracle caps amino inputs at 256 KB.

## Re-running the triage

```bash
# Rust (fetch the advisory DB with git if cargo-deny cannot reach GitHub)
cargo deny check advisories

# Go: govulncheck needs vuln.go.dev; without it, build a local database from
# https://osv-vulnerabilities.storage.googleapis.com/Go/all.zip
# (index/db.json, index/modules.json, ID/<id>.json) and pass -db file://...
cd conformance/<module> && GOFLAGS=-mod=mod govulncheck ./...
```
