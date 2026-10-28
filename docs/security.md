# Dependency security triage

Last triage: 2026-10-15 (previous: 2026-10-14). Tools, all with the
advisory data of that day:

- Rust: `cargo audit` (cargo-audit 0.22.2) and `cargo deny check
  advisories` (RustSec database).
- Dependabot's view: `osv-scanner` v2.3.5 on every manifest and
  `api.osv.dev` queries for every `require` of every `go.mod`. GitHub
  advisories (GHSA) are part of OSV, so this reproduces what Dependabot
  matches.
- Reachability: `govulncheck` v1.1.4 on every module under `conformance/`
  (symbol-level: *called* = reachable from the module's code, *imported* =
  package imported but vulnerable symbol not reached, *module* = only
  required).

Nothing in this file affects the node binary except the Rust section: the
Go modules are test tooling (fixture generators, oracles, recorders). Their
outputs are committed fixtures, so a Go dependency is bumped only when the
tool's output provably stays byte-identical.

## Dependabot alerts (2026-10-15)

GitHub showed 28 open alerts. The alert list cannot be read from the
development container (the API needs the Dependabot-alerts permission), so
the set was rebuilt from the same advisory data: GitHub-reviewed advisories
(those with a GHSA record) matched against every manifest. That gives
these 29 (manifest, package, advisory) alerts, all in Go test tools, none
in `Cargo.lock`:

| Manifest | Package | Advisories (GHSA) | Severity | Fixed in | Disposition |
|---|---|---|---|---|---|
| `conformance/antereplay` | google.golang.org/grpc v1.78.0 | GHSA-p77j-4mvh-x3m3; GHSA-hrxh-6v49-42gf, GHSA-vp52-pcj8-j9qc, GHSA-2v4p-qf9q-27wj; GHSA-qc2q-p7wx-3px3 | critical; high ×3; moderate | v1.83.2 | bumped to v1.83.2 |
| `conformance/antereplay` | github.com/ethereum/go-ethereum v1.15.8 | GHSA-mq3p-rrmp-79jg, GHSA-mr7q-c9w9-wh4h, GHSA-2gjw-fg97-vg3r; GHSA-689v-6xwf-5jf3, GHSA-m6j8-rg6r-7mv8 | high ×3; moderate ×2 | v1.17.0 | bumped to v1.17.0 |
| `conformance/antereplay` | go.opentelemetry.io/otel v1.39.0 | GHSA-mh2q-q3fh-2475 | high | v1.41.0 | bumped to v1.45.0 |
| `conformance/antereplay` | go.opentelemetry.io/otel/sdk v1.39.0 | GHSA-9h8m-3fm2-qjrq, GHSA-hfvc-g4fc-pqhx; GHSA-8wmf-6v46-5gfg | high ×2; low | v1.45.0 | bumped to v1.45.0 |
| `conformance/antereplay` | github.com/go-jose/go-jose/v4 v4.1.3 | GHSA-78h2-9frx-2jm8 | high | v4.1.4 | bumped |
| `conformance/antereplay` | github.com/hashicorp/go-getter v1.7.9 | GHSA-92mm-2pjq-r785 | high | v1.8.6 | bumped |
| `conformance/antereplay` | github.com/ulikunitz/xz v0.5.14 | GHSA-jc7w-c686-c4v9 | moderate | v0.5.15 | bumped |
| `conformance/antereplay` | aws-sdk-go-v2 eventstream v1.7.4, service/s3 v1.95.1 | GHSA-xmrv-pmrh-hhx2 (×2) | moderate | v1.7.8 / v1.97.3 | bumped |
| `conformance/antereplay` | filippo.io/edwards25519 v1.1.0 | GHSA-fw7p-63qq-7hpr | low | v1.1.1 | bumped |
| `conformance/fixturegen` | google.golang.org/grpc v1.83.1 | GHSA-2v4p-qf9q-27wj | high | v1.83.2 | bumped |
| `conformance/fixturegen` | github.com/dvsekhvalnov/jose2go v1.6.0 | GHSA-9mj6-hxhv-w67j | high | v1.7.0 | bumped |
| `conformance/fixturegen` | filippo.io/edwards25519 v1.0.0 | GHSA-fw7p-63qq-7hpr | low | v1.1.1 | bumped |
| `conformance/fuzzoracle/v9` | google.golang.org/grpc v1.82.1 | GHSA-2v4p-qf9q-27wj, GHSA-vp52-pcj8-j9qc; GHSA-qc2q-p7wx-3px3 | high ×2; moderate | v1.83.2 | bumped to v1.83.2 |
| `conformance/fuzzoracle/v9` | otel/sdk, otlptrace, otlptracehttp v1.43.0 | GHSA-8wmf-6v46-5gfg (×3) | low | v1.45.0 | bumped to v1.45.0 |

None of these modules is part of the behavior being reproduced (they are
transport, telemetry, cloud-storage and Ethereum libraries that
celestia-app's module graph pulls in), so they were bumped (category (a)).
The reference modules themselves (celestia-app, the cosmos-sdk / ibc-go
forks, celestia-core, go-square, hyperlane-cosmos) did not move. The same
bumps also cleared Go-database-only findings Dependabot does not show:
klauspost/compress GO-2026-5841 (antereplay, devnet-oracle, fuzzoracle/v9 →
v1.18.7); x/net GO-2026-6603/6610/6611/6612/6617, x/crypto
GO-2026-6303/6354/6355, x/text GO-2026-6629, x/mod GO-2026-6179/6180, otel
GO-2026-5158 and the standard library (toolchain go1.26.6 → go1.26.9) in
fuzzoracle/v9.

Versions were raised with `go get <module>@<fixed>` only (no `go mod
tidy`), so every other `require` line of the pinned graphs is unchanged.
Verification, old binary vs new binary:

- **antereplay** (celestia-app v6.4.10 ante oracle): every case of
  `conformance/fixtures/ante/mainnet-tx-corpus.json.zst` (1,062 mainnet
  txs, account / balance snapshots from the corpus) replayed through the
  binary: output byte-identical before and after, and equal to the
  committed `expected` outcomes; `go test ./...` passes. (The 2026-10-14
  note that this needs mainnet RPC holds only for building a new corpus;
  replaying the committed one is offline.)
- **fixturegen**: all 73 `xtask refresh-fixtures` scenarios regenerated
  into a temp dir, byte-identical to the committed fixtures; `go test`
  passes.
- **fuzzoracle/v9** (celestia-app v9.0.8 oracle): 10,226 oracle requests
  (every input of `crates/fuzz-diff/tests/regressions.jsonl` plus seeded
  byte mutations of the tx inputs, sent as `tx`, `blobtx`, `iw` and
  `fibre`): responses byte-identical.
- **devnet-oracle**: state dumps of a devnet `application.db` at four
  heights: identical.

### Alerts that may remain (dismiss in the GitHub UI)

GitHub's Go dependency graph reads `require` lines; whether it applies
`replace` directives decides if the alerts below exist at all (OSV
tooling applies them and reports none). If they are listed, they stay
by design. Dismiss each with the given reason (Security → Dependabot →
alert → Dismiss):

| Manifest(s) | Package | Advisory | Dismiss as | Justification |
|---|---|---|---|---|
| antereplay, antevectors, mintvectors, txdecode, fuzzoracle/v9, fuzzoracle/v10 | github.com/cosmos/cosmos-sdk v0.50.13 | GHSA-p22h-3m2v-cmgh | Vulnerable code is not actually used | `replace`d by the celestiaorg/cosmos-sdk fork the parity target ships (v0.52.12 for v10.4.0, v0.52.8 for v9.0.8, v0.51.8 for v6.4.10); upstream v0.50.13 is not in the build. |
| antereplay, consensusvectors, ctypesvectors, evidencevectors, lightvectors, rpccorpus/recorder | github.com/cometbft/cometbft v0.38.17 | GHSA-c32p-wcqj-j677, GHSA-hrhf-2vcr-ghch | Vulnerable code is not actually used | `replace`d by celestia-core (v0.42.3 / v0.39.23); upstream cometbft is not in the build. |
| fuzzoracle/v9 | github.com/tendermint/tendermint v0.0.0-00010101000000-000000000000 | GHSA-3fm3-m23v-5r46, GHSA-6jqj-f58p-mrw3, GHSA-v24h-pjjv-mcp6 | Vulnerable code is not actually used | Placeholder `require` of celestia-app v9.0.8's go.mod, `replace`d by celestia-core v1.55.0-tm-v0.34.35; no tendermint code at that version exists. |
| Cargo.lock (only if GitHub lists it) | lru 0.7.8 | RUSTSEC-2026-0253 | Vulnerable code is not actually used | Only via reed-solomon-erasure 6.0.0 (latest), which calls `get` / `put`, never the unsound `pop()`; accepted in deny.toml. |

Any other alert still open after this commit is new and needs triage.
GitHub also offers auto-triage rules (Settings → Code security →
Dependabot rules) that can auto-dismiss alerts by package name for the
`conformance/` manifests, so the replaced-module alerts do not come back.

### Dependabot configuration

`.github/dependabot.yml` (there was none; the alerts' pull requests came
from repository-level security updates):

- **cargo** (root): weekly; all minor / patch version updates in one
  grouped pull request, all security updates in one grouped pull request.
  `flate2`, `blst` and `axum` (pinned with `=`) are ignored, and new
  *major* versions of the consensus-sensitive families listed under
  "Deferred majors" are ignored (migrated by hand).
- **github-actions**: weekly, one grouped pull request.
- **gomod** (all 16 `conformance/` modules in one entry): no version
  updates (`open-pull-requests-limit: 0`); security updates grouped into
  one pull request; the pinned reference modules (`github.com/celestiaorg/*`,
  cosmos-sdk, `cosmossdk.io/*`, cometbft, tendermint, ibc-go, ibc-apps,
  gogoproto, iavl, cosmos-db, hyperlane-cosmos) are ignored.

How `ignore` and security updates interact: `ignore` conditions apply to
security updates too (an ignored dependency still gets an *alert*, but no
pull request), and `open-pull-requests-limit: 0` disables only version
updates. So: pinned oracle modules raise alerts but no pull requests;
every other Go and Rust advisory raises a grouped pull request. An
advisory whose only fix is an ignored Rust major still shows up as an
alert and fails `cargo deny check advisories` in CI (`ci.yml` on every
push, `nightly.yml` job `advisories` daily, which catches new advisories
without a commit).

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

The accepted ones are the `ignore` list of `deny.toml`, each with its
reason; re-check them at every triage (next: 2027-01-15, or when
reed-solomon-erasure / ark-ff release). On 2026-10-15 `cargo audit`
reported no vulnerability, only these three warnings.

### Updates (2026-10-15)

- `cargo update`: every dependency at its newest semver-compatible
  release that supports rustc 1.88 (67 lockfile changes; among them
  serde_json 1.0.154, bytes 1.12.1, regex 1.13.1, clap 4.6.7, libc
  0.2.190, zeroize 1.9.1, blake3 1.8.7, proptest 1.12.0).
- Majors taken: zstd 0.13 → 0.14 (test fixture decompression only), and
  tikv-jemallocator / tikv-jemalloc-ctl 0.6 → 0.7 (jemalloc 5.3.1; the
  optional `jemalloc` / `memstats` features of `celestia-rs`).
- Kept below the latest semver-compatible release on purpose: `flate2
  =1.1.2`, `blst =0.3.14`, `axum =0.7.9` (exact pins); `enum-ordinalize`
  4.4 and `redb` 4 need rustc 1.89 / 1.90 (above the MSRV).

### Deferred majors

| Crates | Latest | Why not now |
|---|---|---|
| prost / prost-types / prost-build 0.13, prost-reflect 0.14, protox 0.7, pbjson* 0.7, tonic 0.12, cosmos-sdk-proto 0.26 | 0.14 / 0.16 / 0.10 / 0.9 / 0.14 / 0.27 | Protobuf encoding of every tx, block and query (consensus-critical); one coordinated migration with regenerated code, gated on the full parity suite. |
| sha2, sha3, ripemd, digest, hmac, hkdf, pbkdf2 0.10 – 0.12; k256, p256 0.13; x25519-dalek 2; aes, aes-gcm, chacha20poly1305, argon2, bcrypt | digest-0.11 generation | One RustCrypto migration (the traits changed together): hashing, signatures, secret connection, keyring file encryption. No advisory against the current versions. |
| rocksdb 0.24, redb 2 | 0.25, 4 | Storage engine of the node (on-disk format, compaction behavior); redb 4 needs rustc 1.90. |
| bech32 0.9 | 0.12 | Address encoding; the API was rewritten (and Go's btcutil rules are ported in `encoding::bech32`). |
| num-bigint 0.4, ark-* 0.5 | 0.5, 0.6 | `math` (cosmossdk.io/math port), `legacy-amino`, `x/warp`, `x/zkism` (Groth16 verification). |
| rand 0.8 / 0.9, getrandom 0.2, bip39 2 | 0.10, 0.4, 3 | Key generation and mnemonics (Go keyring compatibility); rand 0.8 / 0.9 are both in use. |
| axum 0.7.9 (exact), tokio-tungstenite 0.24, base64 0.22 | 0.8, 0.30, 0.23 | RPC / REST / gRPC-web surface at Go parity; tungstenite and base64 follow axum and tonic (taking them alone adds a second / third copy). |
| toml 0.8 | 1.1 | Operator config files (TOML 1.1 parsing / serialization differences); bump with a config round-trip test. |

## Go (`conformance/*` test tools)

### Fixed

- **2026-10-15**: the Dependabot alert set above (antereplay, fixturegen,
  fuzzoracle/v9, devnet-oracle).
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

Byte-identity of the outputs after the 2026-10-14 bumps (regenerated into
a temp dir and compared with the committed fixtures):

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
| every module | golang.org/x/crypto `openpgp` (GO-2026-5932), *module* level | No fixed version (package deprecated); not imported; no GHSA, so no Dependabot alert. |
| fixturegen, devnet-oracle | cometbft v0.38 blocksync (GO-2025-3442, fixed only in v1.0.1), cosmos-sdk x/crisis (GO-2023-1821/1881, no fix) | Not reachable from the generators; cometbft v1 would change the Go types the fixtures are generated from. The GHSA records (GHSA-22qq-3xwm-r5x4: fixed in v0.38.17; GHSA-qfc5-6r3j-jj22 / GHSA-w5w5-2882-47pc: last affected v0.47.2 / v0.50.0-alpha.1) do not cover these versions, so no Dependabot alert. |
| antereplay, fuzzoracle/v9 | aws-sdk-go v1 S3 crypto client (GO-2022-0635/0646, no fix in v1) | *module* level; the S3 crypto client is not imported (the GHSA records list v1.34.0 as fixed, so no Dependabot alert). |
| every module with celestia-app / celestia-core | cosmos-sdk / cometbft / tendermint `require` versions | Replaced by the celestiaorg forks; see "Alerts that may remain". |

## Fuzzing

Two harnesses, both run nightly (`.github/workflows/nightly.yml`, jobs
`fuzz-diff` and `cargo-fuzz`; the execution differentials `json_*`, `exec`,
`pp` and `deliver` in `.github/workflows/fuzz-exec.yml`, driven by
`crates/fuzz-diff/nightly.sh`; determinism across builds and targets in
`.github/workflows/determinism.yml`). Findings are fixed in the crates by
porting Go and pinned by regression tests that run on stable without Go
(`deliver` findings that need a whole chain are pinned as Go-backed runs in
`crates/fuzz-diff/deliver-regressions.txt`, replayed first every night).

`DELIVER_GOVPASS` makes the fuzz accounts pass every proposal (stake on
every bonded validator, a deposit and yes votes every 4 blocks): each
proposal's messages then run in gov's EndBlocker, and a failing message's
error text is stored in the proposal's `failed_reason`, so error texts of
every handler a proposal can carry are compared through the app hash
(finding 76). `DELIVER_TRACE=<file>` writes a digest of every Rust ABCI
response per committed height (`crates/fuzz-diff/determinism.sh`).

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
| `json_*` | every JSON decode reachable from consensus execution, called as the state machine calls it (`jsondecode.go`; inventory below) | the Rust ports (`fuzz-diff/src/jsonops.rs`) | accept / reject and the decoded values |

Inputs: every tx of every devnet fixture (`CheckTx`, proposals, blocks),
the txdecode and amino vectors, and any extra corpus directory
(`--corpus`, e.g. a cargo-fuzz corpus), put through protobuf-aware
mutations (`fuzz-diff/src/mutate.rs`: recursive field edits, boundary
varints, unknown and non-critical fields, wrong wire types, non-minimal
varints, groups, invalid UTF-8, splices). `eds` inputs are random tx sets
(real txs, synthetic blob txs at share-boundary sizes, random max square
sizes) and every devnet block.

`fuzz-diff exec` (`crates/fuzz-diff/exec-diff.sh <scenario>`) is the
execution differential: the recorded scenario is re-driven into a Go
`celestia-appd` (standalone ABCI, gRPC on) and the Rust app in lockstep
(every request answered before the next, so CheckTx state matches), and
after every `Commit` txs built from the scenario's own txs — string fields
of every message (and of messages nested in `Any`s) and the memo replaced by
invalid UTF-8, chars of the Go-form reserved set and checked lengths;
messages wrapped in `MsgExec` or a gov proposal; raw wire mutations — are
run through `cosmos.tx.v1beta1.Service/Simulate` (`runTx` in simulate
mode: ante handler without signatures, router, every handler) on both, over
gRPC. Compared: the `SimulateResponse` bytes (gas, data, log, events,
message responses) or the error's code and gas (a *mismatch*), and the
error text (reported separately).

`fuzz-diff deliver` (`crates/fuzz-diff/deliver-diff.sh <scenario>
[blocks] [txs-per-block] [seed]`) is the delivery differential. Simulate
skips signatures and never commits, so it cannot see a state write or a
`LastResultsHash` difference; here the generated txs are *signed* and
*finalized*. The scenario's `InitChain` app state gets 12 funded accounts
whose keys the harness holds and 6 funded multisig accounts over them
(2-of-3, 1-of-2, nested, 4-of-8 above `tx_sig_limit`, a repeated sub-key,
1-of-1; signed with any subset of sub-keys, mixed and nested sign modes,
and bit arrays, mode-info lists and `MultiSignature`s that do not match),
and short slashing window, unbonding and
voting periods; the recorded blocks are replayed into Go and Rust (each
`FinalizeBlock` compared), then generated blocks follow on the final state:
every `CheckTx` (code, gas, priority, address, sequence; logs separately),
Go's `PrepareProposal` against Rust's, `ProcessProposal` of Go's block and
of the unfiltered tx list (which is sometimes the block finalized, like a
proposer that skips the filter), each tx result (code, codespace, data, gas
wanted / used, then log, info, events, signers), block events, validator
and consensus parameter updates and the app hash. Txs: messages of every
registered `sdk.Msg` type built from scratch (values harvested from the
scenario's messages and the blocks' events, boundary numbers, `LegacyDec`
wire values, denoms, addresses of every kind, times, durations, repeated
fields up to 200 elements), scenario messages re-signed by a fuzz account
and mutated at the field level, state-building flows (sends, delegations,
redelegations, unbondings and their cancellation, validators, rewards,
grants, fee allowances, proposals of gov-signed messages of any type,
deposits, votes), `MsgExec` by a grantee, multi-message and multi-signer
txs, blob txs at share-boundary sizes (with mismatched commitments, sizes,
namespaces, share versions), and ante variants (fees, gas limits from 0 to
`MaxUint64`, memo lengths, timeout heights, extension options, fee granter
and payer, `SIGN_MODE_LEGACY_AMINO_JSON`, missing public keys, wrong
sequences, account numbers and signatures; gas limits just below what Go
used for a successful tx of the same type). With `DELIVER_DUMP_SPLIT`
set, an app-hash split commits both apps and prints every module store
entry that differs (`/store/<name>/subspace`). Blocks carry the tracked
validator set's votes (fuzz validators sometimes absent: downtime jailing)
and sometimes duplicate-vote / light-client-attack evidence; block time
sometimes jumps (unbonding completion, proposal ends, allowance and grant
expiry). `fuzz-diff coverage` prints, per registered message type, whether
each app version decodes it, whether Rust routes it, and how often Go
executed it in the recorded scenarios. `DELIVER_SWEEP` re-sends
successful txs with gas limits below what Go used (see the store access
order audit below); `DELIVER_SCRIPT=<file>` runs scripted txs first
(`<account index> <type URL> <hex value> [gas]` or `vote <account
index>`, a yes vote on the latest proposal, per line, `---` between
blocks; the block is finalized unfiltered and each result printed, and
the results of the proposals that ended are counted);
`fuzz-diff genesis <scenario>` writes a `genesis.json` from the recorded
`InitChain` for the scenarios recorded without one (`ibc-edge`,
`ibc-upgrades`), which the scripts use; `fuzz-diff accounts` prints the
fuzz accounts. `DELIVER_IBC` adds a mock IBC counterparty
(`crates/fuzz-diff/src/ibcmock.rs`): a chain whose `ibc` and `upgrade`
IAVL stores sit under a multistore root that its headers (one ed25519
validator the harness holds) commit to, so every step it "makes" is
written into its store and proven with real ICS-23 proofs. It drives the
chain under test through the success paths in order: three 07-tendermint
clients, connection init / ack, an ICS-20 channel (init / ack) and an
ICS-27 host channel (try / confirm); then packets both ways (transfers
with PFM memos, redundant relays, flipped proof bits, acknowledgements,
error acknowledgements, timeouts), ICA host txs in either encoding (with
a fee grant whose amount has a sign, and keys gogoproto skips),
misbehaviour on the second client, `MsgUpgradeClient` of the third,
ICS-27 channel upgrades the counterparty proposes (try, confirm with the
counterparty flushing or flush complete, open, cancel on an error
receipt, timeout) and transfer channels the counterparty closes
(`MsgTimeoutOnClose`, `MsgChannelCloseConfirm`). Hyperlane owner
operations target objects their signer created (a create message first
when it has none), and ownership changes are followed. `DELIVER_HL`
drives the hyperlane success paths in order, one tx per block from a
dedicated account, each step using the ids the earlier ones returned: a
noop ISM, a mailbox on it, an IGP with a gas config, a merkle tree hook
and the mailbox's hooks, a collateral token with an enrolled router,
`MsgRemoteTransfer` (the IGP charges its fee), `MsgPayForGas`,
`MsgClaim`, `MsgProcessMessage` of a transfer in (the noop ISM verifies
it; collateral released), a synthetic token minted by a second
`MsgProcessMessage` and burned by a transfer out; at app version 10 a gov
proposal of `MsgCreateNativeSyntheticToken` that the fuzz accounts pass
(delegating to the bonded validators first; with `DELIVER_HL` evidence
never names the scenario's own validators, whose jailing would leave no
bonded stake to tally); then the transfer / payment / claim / process
steps repeat (`SCRIPT ... HL step` lines give Go's result of each). In
`hyperlane`, `zkism-forwarding`, `v9-hyperlane` and `v9-zkism-forwarding`
(200 blocks, with `DELIVER_WIRE`, seed 6) Go executed `MsgPayForGas`
11-13 times, `MsgClaim` 2-13 (a generated `MsgSetIgpOwner` sometimes takes
the IGP), `MsgProcessMessage` 22-26, `MsgRemoteTransfer` 22-25 and
`MsgCreateSyntheticToken` 11-13 per scenario, and at app version 10 the
`MsgCreateNativeSyntheticToken` proposal passed in both scenarios; no
mismatch. Generated gov proposals carry `MsgCreateNativeSyntheticToken`
too (a fuzz-created mailbox; a new denom, `utia` or empty).

`DELIVER_HL_ISM=message-id` or `merkle-root` replaces the scripted mailbox's
default noop verifier with a 2-of-3 multisig whose Ethereum signing keys the
harness holds. Incoming transfers carry signed checkpoints; Merkle-root
metadata also carries a valid membership proof. Each signed incoming tx
sets its recipient token's ISM immediately before `MsgProcessMessage`, so
other fuzz transactions cannot replace verification with a noop and still
earn signed-coverage credit. `HL_VERIFIED` lines and `DELIVER_STATS` retain
successful collateral and synthetic deliveries separately. A selected
multisig mode exits nonzero if either success path was never reached,
even when every Go/Rust rejection matched. Default `noop` behavior is
preserved. Nightly hyperlane/zkism runs rotate all three modes independently
of gas-sweep selection, retaining the mode in their `.env` evidence.

`DELIVER_HL_REJECT=1` requires a signed ISM and begins with a controlled
phase: missing, duplicated, reversed and invalid-recovery signatures, plus
Merkle proof and signed-index mutations for `merkle-root`. Each invalid tx
is immediately followed by valid metadata for the exact same message ID.
Both include the ISM selection. Coverage requires adjacent finalized txs,
the expected verification-stage rejection, and a successful retry; this
checks rollback of mailbox receipts written before verification. Unrelated
generators, gas sweeps and user scripts wait until every pair completes,
then resume. `HL_RETRIED` and `DELIVER_STATS` retain per-case counts,
failures, controlled-phase block count and completion height. Missing or
failed pairs exit nonzero even if Go and Rust agree. Nightly signed
Hyperlane scenario modes enable this phase; noop runs retain their existing behavior. Upgrade jobs
rotate noop, message-ID and Merkle-root verification independently of
WIRE / SWEEP / RECHECK selection, without the rejection phase. The full
campaign runs all three modes for each Hyperlane/zkism scenario, both with
ordinary generation and with `DELIVER_GOVPASS` enabled, and distributes
its twelve distinct upgrade seeds across all three modes. These selections
are retained in each run's `.env`; signed success counters describe the
whole run and do not alone establish successful delivery on both sides of
the upgrade. Budget-skipped campaign cases still make the campaign fail.
Combining this phase with `DELIVER_UPGRADE` is rejected: postponing the
upgrade driver's early fixture windows would silently weaken that campaign.

The standalone Go decoding oracles also construct an in-memory App, whose
upgrade keeper reads on-disk metadata at startup. Each oracle therefore uses
a private temporary `NodeHome`. The Rust driver owns the parent scratch
directory and removes it after killing and reaping an oracle, including
periodic restarts; standalone EOF removes the Go-created home. The startup
smoke check exercises two concurrent instances and repeated Rust-driven
shutdowns without changing the operator's home. `fuzz-exec` runs it against
both pinned Go versions before differential jobs.

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
GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/exec-diff.sh utf8 10
GO_APPD=$DEVNET_BIN/v9/celestia-appd crates/fuzz-diff/exec-diff.sh v9-gov 10
# nil math customtypes / messages built from scratch instead of invalid UTF-8
EXEC_MODE=nil GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/exec-diff.sh hyperlane 10
EXEC_MODE=synth EXEC_SYNTH=warp GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/exec-diff.sh hyperlane 10
# JSON decodes (depth, numbers, key folding) and gov metadata in execution
target/release/fuzz-diff --app-version 10 --ops json_count,json_gov,json_pfm,json_pfm_next,json_ica_tx,json_ica_vb,json_ica_pd,json_ica_md,json_ics20,json_ack,json_ack_check --iters 40000
EXEC_MODE=json GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/exec-diff.sh gov 10
# signed txs finalized on both apps (app hash, tx results, CheckTx, proposals)
GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/deliver-diff.sh gov 300 8 1
GO_APPD=$DEVNET_BIN/v9/celestia-appd crates/fuzz-diff/deliver-diff.sh v9-hyperlane 300 8 1
target/release/fuzz-diff coverage   # message-type coverage of the recorded scenarios
# pin a finding: record Go's outcome into the regression file
echo "tx <hex>" | target/release/fuzz-diff record 10 "what it shows" >> crates/fuzz-diff/tests/regressions.jsonl
```

`crates/fuzz-diff/tests/regressions.jsonl` holds every finding with Go's
recorded outcome; `cargo test -p fuzz-diff` replays them without Go.

### JSON decoding in consensus (2026-10-16)

Go's `encoding/json` (Go 1.26, v1) fails the 10 001st nested array or
object (`maxNestingDepth`, arrays and objects counted together) and
`json.Unmarshal` checks the whole input before decoding (`checkValid`), so
the limit applies inside values the decoder skips too. gogoproto `jsonpb`
reads its input with `json.Decoder` (same scanner and limit, first value
only) and re-decodes fields with `json.Unmarshal`. protojson (its own
10 000-level `RecursionLimit`) is not reached from txs (cosmos-sdk uses it
only in `codec/collections.go`, for genesis), nor is any other JSON parser.
serde_json stops at 128 levels and its `IgnoredAny` never stops. Every
JSON decode reachable from DeliverTx, CheckTx, PrepareProposal and
ProcessProposal, at both app versions:

| Path (reached from) | Go decoder | Rust before | Divergence |
|---|---|---|---|
| `countJSONMsgs` (`MsgRecvPacket` to a proto3json ICA channel; CheckTx, PrepareProposal, ProcessProposal; v10) | `json.Unmarshal` into `struct{ Messages []struct{} }` | serde with `IgnoredAny` (no limit) | **yes** (finding 28) |
| gov `SubmitProposal` metadata (`MsgSubmitProposal`, also in `MsgExec`; both versions) | `json.Unmarshal` into `ProposalMetadata`, before the 255-byte length check | `serde_json::Value` (128), ASCII-only key folding, keys sorted and deduplicated | **yes** (finding 29) |
| PFM `OnRecvPacket` memo (ICS-20 packet to the transfer port; both) | `json.Unmarshal` into `map[string]interface{}`, then `PacketMetadata`; `JSONObject` (`orderedmap`) for `next`; retry path `json.Unmarshal(memo, Next)` | recursive parser, no limit | **yes** (finding 30) |
| ICS-20 `FungibleTokenPacketData` (transfer / PFM recv, ack, timeout), `channeltypes.Acknowledgement` (core `AcknowledgePacket`, transfer / PFM acks), ICA `InterchainAccountPacketData` (host recv, message count), ICA `Metadata` (channel handshakes and upgrades) | `jsonpb` (`json.Decoder`, then `json.Unmarshal` per field; unknown fields rejected) | serde_json `Value` (128) | depth: no (every field is a scalar, so Go fails any nested value too); lone `\ud800` escapes and invalid UTF-8: **yes** (finding 31); `bytes` as an array of numbers, `null` oneof members, `-0`: **yes** (finding 34) |
| ICA host `DeserializeCosmosTx` proto3json (host recv) | `ProtoCodec.UnmarshalJSON` (`jsonpb` with the registry as `Any` resolver) then `UnpackInterfaces` | serde_json `Value` (128), registry JSON | depth: no (`UnpackInterfaces` stops at 10 nested `Any`s, about 30 levels; no registered message nests deeper without an `Any`); the `Any` limit itself: **yes** (finding 32); `null` elements, every other `jsonpb` rule: **yes** (finding 33) |
| typed events (`TypedEventToEvent`) | `json.Unmarshal` of the module's own `jsonpb` output | serde of flat event types | no (no event nests user data) |
| x/blob legacy params, InitChain genesis | params-subspace JSON written by the chain, genesis | serde_json | not reachable from txs |

Fix: `crates/tier-0-primitives/gojson`, a port of the scanner (Go's error
texts), `checkValid`, `json.Decoder`'s first value, `unquote` and a value
tree, all without recursion (tree drop included); recursive consumers (PFM
re-encoding) run on a 256 MiB-stack thread for inputs over 1 KiB.

Evidence: `fuzz-diff --ops json_*` against both oracles (seeds with total
depths 127, 128, 129, 1 000, 9 999, 10 000, 10 001 in each JSON-carrying
field, then JSON-text mutations: deep values, `1e400`, folded keys `ſ` /
`K`, duplicate keys, `null`s, invalid UTF-8 and escapes). Before the fix
the seed round alone gave findings 28-30 and the Rust process died of a
stack overflow on a 10 000-level PFM `next` (the recursive parser); the
mutation rounds added 31-32. `EXEC_MODE=json fuzz-diff exec` (gov
proposals whose metadata nests deeply, holds `1e400`, folded or duplicate
keys): with the old gov code 161 of 940 simulated txs differed from Go
v10.4.0 in code or gas (Go fails at the metadata title check, Rust went on
to the signer check); after the fix none of 940.

### Store access order in transactions (2026-10-16)

Finding 60 was a loop that collected an iterator's entries before
processing them: same total gas, but a tx running out of gas inside the
loop stops at a different point (`gas_used`, `LastResultsHash`). Every
gas-metered iteration reachable from a tx (and the begin / end blockers,
where only the final state counts) was compared with the Go source, both
app versions. `sdk::context::Iter` charges like `gaskv` (the seek of the
first element at creation, each `Next` charges the element it leaves, a
`break` before `Next` charges nothing more), so a loop is exact when it
calls `next` where Go calls `Next` and does its reads and writes in
between.

| Keeper | Loop (tx path unless noted) | Go | Rust | Divergent? |
|---|---|---|---|---|
| distribution | `IterateValidatorSlashEventsBetween` (rewards of `MsgDelegate`, `MsgUndelegate`, `MsgWithdrawDelegatorReward`, ...) | handler per event before `Next` | `for_each_slash_event_between` | fixed (60) |
| distribution | `DeleteValidatorSlashEvents`, `DeleteValidatorHistoricalRewards` (`AfterValidatorRemoved`, last undelegation of an unbonded validator) | delete inside the loop | `delete_prefix` | no |
| gov | `ChargeDeposit` (`MsgCancelProposal`) | `GetDeposits` (collect), then refund / delete per deposit | `get_deposits`, then the same | no |
| gov | `deleteVotes` (`MsgCancelProposal`) | `Votes.Clear`: collect keys, then delete | same | no |
| gov | `RefundAndDeleteDeposits`, `DeleteAndBurnDeposits`, tally (end blocker) | walk with refund / delete inside | same | no |
| feegrant | `RemoveExpiredAllowances` (`MsgPruneAllowances`, limit 75; end blocker 200) | two deletes per entry, stop at the limit before `Next` | same | no |
| authz | `DequeueAndDeleteExpiredGrants` (begin blocker) | walk, delete inside | same | no |
| staking | `HasReceivingRedelegation` (`MsgBeginRedelegate`) | prefix iterator, `Valid` only | first `next` | no |
| staking | `GetValidatorDelegations`, `GetUnbondingDelegationsFromValidator`, `GetRedelegationsFromSrcValidator` (slashing: begin blocker) | collect with a read per index entry | same | no |
| staking | power index (reverse), last validators, unbonding / redelegation queues (end blocker) | as Go | same | no |
| bank | `GetAllBalances` (`SpendableCoins`) | `Balances.Walk` | same | no |
| signal | `TallyVotingPower` (`MsgTryUpgrade`) | validator read, delete of removed validators' signals and power read inside the loop, return at the threshold | same | no |
| signal | `ResetTally` | delete inside | same | no |
| fibre | `ReduceWithdrawalsForPayment` (`MsgPayForFibre`) | `for ; Valid() && remaining.IsPositive(); Next()` (the post `Next` runs before the condition) | same, `next` before the `break` | no |
| fibre / valaddr | withdrawals, processed payments, stale providers (begin / end blockers) | as Go | same | no |
| hyperlane | `AnnounceValidator`: storage locations (`Iterate` + `Values`) | collect | same | no (the only iteration in hyperlane-cosmos tx paths) |
| IBC 07-tendermint | `pruneOldestConsensusState` (`MsgUpdateClient`; not in CheckTx) | first element, callback stops before `Next` | first `next` only | no |
| IBC 07-tendermint | `GetNextConsensusState` / `GetPreviousConsensusState` (misbehaviour checks) | `Valid`, `Value`, one `Next` / reverse iterator | same | no |
| IBC channel | `HasInflightPackets` (upgrades, timeouts on close) | `Valid` only | first `next` | no |
| auth, slashing, upgrade, mint, circuit, consensus, blob, minfee, zkism, forwarding, evidence, capability, transfer, ICA host, PFM, warp | no iteration on a tx path (genesis export, queries, begin / end blockers only) | | | no |

Beyond loops, every read, write and charge sequence of a successful tx is
checked by the gas sweep of `fuzz-diff deliver` (`DELIVER_SWEEP`): each tx
Go executes successfully (single signer, a fuzz account) becomes a
template that later blocks re-send with consecutive sequences and gas
limits spread below what Go used (a quarter of them within 4 000 of it),
so the copies run out of gas at many points of the ante handler and the
message handlers; `gas_used` of an out-of-gas tx is the first charge past
the limit, so any difference in the order or size of the charges shows as
a result mismatch.

### Wire-level sweep (2026-10-17)

Findings 46, 56, 57, 61, 62 and 64 were all one class: Rust reading or
re-encoding protobuf bytes differently from the Go decoder of that path
(gogoproto, pulsar, protobuf-go, `dynamicpb`, `unknownproto`). Random
mutation sampled the class; `crates/fuzz-diff/src/wire.rs` enumerates it.
For a valid encoded message it produces every rewrite below, at every
depth (embedded messages, `stdtime` / `stdduration` fields, `Any` values
resolved through the schema; at most 5 levels, the first two occurrences
of a repeated field, 6 000 rewrites per message):

| Classes | Rewrite |
|---|---|
| `dup_same`, `dup_last`, `dup_first` | a singular scalar / string / bytes / customtype field repeated (same value; another value after; before): `Int` `""`, `0`, `-1`, `+1`; `LegacyDec`; `HexAddress` |
| `msg_empty_last`, `msg_empty_first`, `msg_split`, `msg_part_last`, `msg_alt_last` | a singular embedded message (`stdtime`, `stdduration`, `Any`, oneof members included) repeated: empty after / before, its fields split over two occurrences (an `Any`'s URL and value apart), a part after, a copy with one scalar changed after |
| `packed`, `unpacked`, `packed_mixed`, `packed_empty`, `repeated_interleaved`, `packed_long10`, `packed_nonmin`, `unpacked_long10` | repeated scalars packed / unpacked / both, an empty run, elements between other fields, 10-byte and non-minimal varints inside a run |
| `reorder_rev`, `reorder_rot` | fields out of order |
| `unknown`, `unknown_alias` | unknown fields: `max+1`, 1023-1025, 1031, 2047, 2048, 3072, 19000, 19999, 2^29-1, 2^29, 2^31-1, 2^31, keys past 2^32 (gogoproto's `int32` truncation, also aliasing a known field), each wire type, first and last |
| `wrong_wt` | a known field with every other wire type (group included) |
| `zero_len`, `default_explicit`, `drop` | an embedded message emptied; absent fields with explicit defaults (empty packed runs, `""` / `"0"` customtypes, empty messages); a field removed |
| `nonmin_tag`, `nonmin_val`, `nonmin_len`, `long10_tag`, `long10_val`, `long10_len` | `0x80 .. 0x00` padding and 10-byte varints with high bits in keys, values, length prefixes |
| `group`, `group_mismatch`, `group_lone_end`, `group_open`, `group_known`, `field0`, `wt67` | wire types 3 / 4 at the start, middle and end (nested, mismatched end, lone end, unterminated, on known numbers); field number 0; wire types 6 / 7 |
| `oneof_two`, `oneof_two_first` | two members of one oneof |
| `any_url_*`, `any_value_*` | type URL without `/`, with a host, `//`, trailing space, empty, two URLs; value empty, with trailing bytes (field 0, a non-critical and a critical field), truncated, two values |

No message reachable from a tx has a map field (the schema's only map is
PFM's genesis `InFlightPacketsEntry`). `SIGN_MODE_TEXTUAL` is not enabled
(`authtx.DefaultSignModes`); `DIRECT` / `DIRECT_AUX` sign the bytes as
received, so `SIGN_MODE_LEGACY_AMINO_JSON` is the only sign-bytes encoder.

`fuzz-diff wire <app version> [per type] [seed] [cap] [dir]` takes, per
registered `sdk.Msg` type, synthesized messages, the recorded scenarios'
messages (signer rewritten) and state-building flows, and sends every
rewrite as the tx's message, inside `MsgExec` and inside a gov proposal
(oracle ops `tx`: `TxDecode`, `unknownproto`, `UnpackInterfaces`, gogoproto
re-encoding of each message, `GetSigners`; and `aminosign`: amino-JSON
sign bytes) and as the message of a protobuf ICA `CosmosTx` (`json_ica_pb`:
the host's decoding and `ValidateBasic`); the `TxBody` and `AuthInfo` of a
valid tx are enumerated too. `DELIVER_WIRE=1 fuzz-diff deliver` gives
every generated message (top level, in `MsgExec`, in proposals, the
`MsgPayForBlobs` of blob txs; with `DELIVER_IBC` the ICA host's messages,
plus a periodic fee allowance with `stdtime` / `stdduration` fields) one
rewrite of the class used least so far, signed and finalized on both apps
(`CheckTx`, ante with signatures, routing and `ValidateBasic`, execution
with `DELIVER_SWEEP`'s gas sweep, the stores with `DELIVER_DUMP_SPLIT`);
`WIRE <class> delivered= go_ok= check_rejected=` lines give the volume
per class. Queries that echo stored messages are compared too: every 25
blocks and at the end of a run, the ABCI query response (code, codespace,
value bytes; log separately) of both apps on the committed state for gov
`v1` / `v1beta1` proposals (`v1beta1` converts messages to legacy
content), votes and deposits of the latest proposals, authz grants by
granter and grantee, fee allowances by grantee and granter, accounts
(public keys as stored), validators (consensus keys), the module
parameters set by gov `MsgUpdateParams`, IBC client states, connections
and channels, and hyperlane mailboxes, ISMs, IGPs, hooks and tokens
(`QUERY <path> sent=` lines).

Volumes (one seed each; inputs / Go accepted):

| Level | v10 | v9 |
|---|---|---|
| `TxBody` / `AuthInfo` rewrites (`tx`, `aminosign`) | 505 / 208, 1 027 / 128 | same |
| message in a tx (`tx`; `aminosign`) | 200 112 / 84 226 | 192 716 / 80 302 |
| in `MsgExec`; in a gov proposal (`tx`; `aminosign`) | 200 112 / 84 226 each | 192 716 / 80 302 each |
| ICA `CosmosTx` (`json_ica_pb`) | 200 112 / 148 317 | 192 716 / 142 567 |
| total oracle inputs (`fuzz-diff wire 10 2 2`, `wire 9 2 2`) | 1 403 962 | 1 352 190 |
| `DELIVER_WIRE` campaigns (300 blocks, 10 txs per block, sweep, store dump): txs to `CheckTx` / finalized / Go code 0 | 12 scenarios: 40 133 / 11 855 / 1 799 | 10 scenarios: 33 090 / 10 736 / 1 499 |
| of which rewritten messages: in finalized txs / Go code 0 / in txs Go's `CheckTx` rejected | 2 734 / 718 / 26 149 | 2 405 / 651 / 20 948 |
| second round, oracle (`wire 10 8 7` + `wire 10 12 8`; `wire 9 8 7` + `wire 9 12 8`): message rewrites per level (tx, `MsgExec`, gov proposal, ICA `CosmosTx`; each through `tx` and `aminosign`) | 1 650 766 per level, 11 561 490 inputs | 1 592 651 per level, 11 154 685 inputs |
| second round, `DELIVER_WIRE` (seeds 5-7, with the query comparison; `hyperlane` / `zkism-forwarding` also with `DELIVER_HL`): runs, blocks / txs to `CheckTx` / finalized / Go code 0 | 21 runs, 6 106 / 69 488 / 23 237 / 3 429 | 12 runs, 3 400 / 39 175 / 13 086 / 2 018 |
| of which rewritten messages: in finalized txs / Go code 0 / in txs Go's `CheckTx` rejected | 5 010 / 1 361 / 43 395 | 2 733 / 730 / 24 230 |
| queries compared (stored messages echoed) | 30 177 | 16 468 |

Results: findings 65 (the first run: 6 distinct mismatches, all one cause)
and 66 (the first `DELIVER_WIRE` run of the `ibc` scenario); 67 from
reading every place Rust decodes a tx's messages outside the router. After
the fixes: no mismatch in the oracle sweeps above, and none in the
`DELIVER_WIRE` campaigns. The second round found no mismatch and no query
difference. It closed two harness gaps: from the third generated block on,
the commit had left out every validator without a power change (the
scenario's validators missed blocks and were jailed for downtime, so no
gov proposal could pass), and the ibctesting scenarios (`ibc-edge`,
`ibc-upgrades`: validators without slashing signing info) halted both
apps at the first vote or evidence; their blocks now carry neither.

```bash
target/release/fuzz-diff wire 10 2 2 0 /tmp/wire-findings
DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_DUMP_SPLIT=1 GO_APPD=$DEVNET_BIN/celestia-appd \
  crates/fuzz-diff/deliver-diff.sh gov 300 10 1
DELIVER_WIRE=1 DELIVER_IBC=1 GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/deliver-diff.sh ibc 300 10 1
```

### Upgrade boundary (2026-10-17)

Mainnet moves from app version 9 to 10 by signalling; the multiplexer
then swaps binaries inside `Commit`. `DELIVER_UPGRADE=<n>` runs `deliver`
across it: `v9-upgrade` (chain `test`, delay 3) is replayed up to its
validator's first signal tx, the generated blocks run on Go v9.0.8, the
held signal is sent from block `n`, and after the `Commit` that raised the
app version the harness stops Go and starts v10.4.0 on the same home
(`deliver-diff.sh` `GO_APPD_NEXT`, as `abci-recorder`'s switch mode); the
Rust app stays one process. Compared every block: app hash (every store
dumped at `H` and `H+1`), results, gas, events, `CheckTx`,
`PrepareProposal`, `ProcessProposal`; signal / upgrade / consensus /
fibre / valaddr / module-account / router queries every 5 blocks and
every block near `H`. The driver (`crates/fuzz-diff/src/deliver/upgrade.rs`):
fuzz validators (a minority of the power, signalling 0 / 9 / 10 / 11 /
`u64::MAX`), `MsgTryUpgrade` before quorum, in the signal's block, until
pending and while pending; authz grants (incl. for v10-only messages),
fee allowances (incl. limited to v10-only messages), vesting accounts
(new keys, used at v10), a locked account and a send at the fibre
address, gov proposals submitted at v9 (also while pending) and executed
at v10, hyperlane routers enrolled in v9's shapes (upper-case, no `0x`,
short, non-hex, `0X`, non-ASCII) and transferred over at v10; around `H`
fibre escrow / withdrawal, valaddr hosts, `MsgCreateNativeSyntheticToken`
(direct and proposed), `MsgForward` with and without fields 7/8, `MsgExec`
of the v9 grants, fees paid by the v9 allowances; every generated tx is
also proposed unfiltered at `H..H+2`; late at v10 the fuzz validators take
5/6 of the power and signal 10 / 9. `DELIVER_HL`, `DELIVER_IBC` (packets
in flight across the switch), `DELIVER_WIRE` and `DELIVER_SWEEP` combine
with it. Plus a live devnet with the real switch
(`workloads/v9-upgrade-boundary.sh`, fixture replayed with full parity).

Findings 68-72. Volumes after the fixes (24 runs, seeds 1-41, 0
mismatches): 3 275 generated blocks, 32 065 txs to `CheckTx` (17 526
accepted), 15 987 delivered (9 611 code 0), 3 047 gas-sweep copies; per
phase (blocks / delivered / Go code 0): before the signal 543 / 3 257 /
2 209, `H-2` 24 / 203 / 56, `H-1` 24 / 260 / 144, `H` 24 / 366 / 237,
`H+1` 24 / 446 / 339, `H+2` 24 / 351 / 269, `H+3` 24 / 349 / 267,
`H+4..20` 408 / 2 117 / 1 366, later 2 180 / 8 638 / 4 724. Driver txs:
1 909 `MsgSignalVersion`, 1 005 `MsgTryUpgrade`, 1 716 escrow deposits,
518 withdrawals, 475 valaddr hosts, 500 `MsgForward`, 941 `MsgExec`, 229
grants / allowances, 239 vesting accounts, 853 proposals (58 submitted at
v9 ended at v10: 41 passed, 10 failed on execution, 7 rejected), 269
transfers over v9 routers.

```bash
GO_APPD=$DEVNET_BIN/v9/celestia-appd GO_APPD_NEXT=$DEVNET_BIN/celestia-appd \
  DELIVER_UPGRADE=20 DELIVER_HL=1 DELIVER_IBC=1 \
  crates/fuzz-diff/deliver-diff.sh v9-upgrade 150 6 1
APP_V9=1 UPGRADE_TO_V10=1 CHAIN_ID=test conformance/devnet/devnet.sh \
  v9-upgrade-boundary 90 conformance/devnet/workloads/v9-upgrade-boundary.sh
```

### Log parity, mempool recheck and simulation (2026-10-17)

`fuzz-diff deliver` reports every difference of a tx result's log, info,
events or signers, of a `CheckTx` log and of a query's log as a `LOG`
line. Three more comparisons were added:

- `DELIVER_SIM`: every generated tx is simulated on both apps before its
  `CheckTx` (`/cosmos.tx.v1beta1.Service/Simulate` through ABCI Query, as
  signed and with its signatures emptied the way `--gas auto` builds the
  simulated tx). The `gas_info` and the code must agree (a mismatch);
  the response's other bytes and the error text are logs.
  `deliver-diff.sh` now enables Go's gRPC server: without it the SDK never
  registers the tx service on the query router and Go answers `unknown
  query path`.
- Comet's mempool recheck: after the `Commit` that switches binaries
  (`DELIVER_UPGRADE`), and after every `Commit` with `DELIVER_RECHECK`, the
  txs Go's `CheckTx` accepted and no block included are rechecked on both
  apps in order (`CheckTxType_Recheck`), and a tx leaves the modelled pool
  as in `TxPool.handleRecheckResult` (code, then `PostCheckMaxGas`).
  `RECHECK <phase>` lines give the volume.
- The mock IBC counterparty's every third `MsgUpdateClient` carries a
  commit signature flag outside 1..=3 (finding 73).

Before the fixes, 13 runs (`ibc`, `fibre`, `gov`, `hyperlane`, `staking`,
`authz-feegrant`, `utf8`, `vesting`, `blob`, `zkism-forwarding`,
`v9-ibc`, `v9-gov`, `v9-hyperlane`; 100 blocks, 8 txs per block) gave 69
tx-result / `CheckTx` log differences besides Go panic stacks and
protobuf-go's randomized space, and the earlier `DELIVER_UPGRADE` batch
(20 runs, `DELIVER_IBC` / `DELIVER_HL`) 615: IBC packet and timeout
errors without Go's details (`current height: 0-52, timeout height
0-1`, `(channel-0 ≠ channel-7)`, `next sequence send (17) ≥ (17).`),
unknown enum values printed `UNKNOWN` (Go prints the number), the
07-tendermint light client's errors without their values and wrappers
(`invalid header: new header has a time from the future <time> (now:
<time>; max clock drift: 10m0s)`, `untrustedHeader.ValidateBasic
failed: ...`), the bech32 decoder's texts (the `bech32` crate's instead
of `decoding bech32 failed: invalid checksum (expected ... got ...)`,
account / validator codec errors as `GetFromBech32`'s, a doubled
`: invalid address`), `cosmossdk.io/errors`' `%v` frame where ibc-go
formats an address error with `%v` (`addresses cannot be empty: unknown
address [celestiaorg/cosmos-sdk@v0.52.12/types/address.go:173]`; v9.0.8:
`@v0.52.8`), the staking gRPC status prefix (`rpc error: code = NotFound
desc = ...`), fibre payment-promise validation without its `invalid
request` wrap and values, `%q` printed as Rust's `{:?}`, gov's nil
weight (`<nil>`) and `VoteOption.Format` (`byte(vo)`), `json.Marshal` of
an empty options slice (`null` / `[]`), the PFM fallback's joined error,
an out-of-gas panic inside the IBC redundant-relay ante check (Go's
`out of gas in location: ReadFlat; gasWanted: ..., gasUsed: ...`). The
same 13 runs after the fixes (seeds 13 / 14): 0 mismatches; the only
tx-result / `CheckTx` log differences left are Go panic stacks (98) and
protobuf-go's space (21), plus 3 since fixed (gov `byte(vo)` and `[]`,
authz's codec error). The tx decoder's texts (`no registered
implementations of type tx.TxExtensionOptionI`, `could not consume field
value for tagNum: 7, wireType: "bytes"; unexpected EOF`, `integer out of
range; got: 300, max: 256`, gogoproto's `%#v` of timestamps and
durations, go-square's blob and namespace errors, `encoding/hex: invalid
byte: U+0067 'g'`) were fixed from the simulation's logs: Go fails an
undecodable tx in `CheckTx` without a `ResponseCheckTx`, so only
`Simulate` and a malicious block's results show them.

`Simulate` logs: of 12 228 differences in those runs, 11 672 are only
the `[file:line]` frame `cosmossdk.io/errors` appends when the tx
service formats the error with `%v`: the Go source line of the innermost
`errorsmod.Wrap` of each error (and the build machine's module cache
path for modules outside `github.com`, e.g. `cosmossdk.io/errors`
itself). Emulating it needs the Go source position of every error site;
not done. The rest are Go panic stacks, protobuf-go's space and
gogoproto's `wrong wireType = 2 for field RevisionHeight` (Rust prints
the field number; the schema table has no Go field names).

Gas estimates: 0 differences in `gas_info` or code in 32 236 simulations
(both variants; Go succeeded in 4 750) over 128 first-message types (every
message type the generator builds), both app versions and an upgrade run.

Mempool recheck after the v9 -> v10 switch: 3 `DELIVER_UPGRADE` runs
(seeds 21-23, `DELIVER_RECHECK`, with `DELIVER_IBC` / `DELIVER_HL` /
`DELIVER_WIRE` / `DELIVER_SWEEP`): 274 rechecks on the switch (every
pool tx: the fresh v10 check state has height 0, so `gas_wanted` is -1
and `PostCheckMaxGas` removes it; the check state still applies its
sequence), 27 041 rechecks in all, 0 differences in code, gas, priority,
sequence or log. The Rust node's mempool calls the same `App` after
`switch_on_commit` (`BlockExecutor::commit` -> `TxPool::update` ->
`recheck_transactions`, a port of celestia-core v0.42.3's), so the
node removes the same txs.

```bash
DELIVER_SIM=1 DELIVER_IBC=1 GO_APPD=$DEVNET_BIN/celestia-appd crates/fuzz-diff/deliver-diff.sh ibc 100 8 13
GO_APPD=$DEVNET_BIN/v9/celestia-appd GO_APPD_NEXT=$DEVNET_BIN/celestia-appd DELIVER_UPGRADE=20 \
  DELIVER_RECHECK=1 DELIVER_IBC=1 crates/fuzz-diff/deliver-diff.sh v9-upgrade 80 6 21
```

### Retained execution evidence (F1.2 / F1.3)

`fuzz-exec` uploads `findings-<job>` artifacts on success as well as
failure. Clean runs retain stdout (including `TYPE` and `GOV_EXECUTED`),
stderr, exit status, explicit run flags, source revision and worktree
status / patch and Go binary build information; successful Go app logs
are discarded. `DELIVER_STATS`
in stderr counts generated blocks and finalized **transactions**, not
CheckTx attempts or messages within a transaction.

After downloading artifacts into separate directories, summarize them:

```sh
python3 crates/fuzz-diff/saturation-report.py <artifact-dir> [<artifact-dir> ...]
```

The report groups volume by source revision, counts identical
scenario / seed / block count / txs-per-block / flags only once, and
excludes failed, mismatched, incomplete and uncommitted-source runs.
Both Go binaries must match the toolchain pinned in
`conformance/devnet-oracle/go.mod`. A newer compiler is not interchangeable:
a local Go 1.27.0 build emits literal U+FFFD for invalid UTF-8 JSON
strings, where Go 1.26.9 emits `\ufffd`. That changes amino sign bytes
and creates false CheckTx / PrepareProposal discrepancies against Rust.
`build-bins.sh` defaults to the pinned toolchain even when a newer Go is
installed; the report excludes other compiler versions.
Different finalized counts for an identical run are an error. Do not
sum revisions to claim F1.2: its volume starts after the last fix and
still needs every scenario, both app versions and 14 clean nights.
Artifacts from before `DELIVER_STATS` are not counted by this report.

The `exec` client allows HTTP/2 header lists up to 1 MiB on both
connections: invalid-denom errors can carry large percent-encoded
`grpc-message` headers. Nightly seed 13 (`fibre`, strings mode, seed
13008) hit the old client limit on Rust's response and compared a
transport reset (code 13) with Go's SDK error (code 2). With the larger
allowance, all 2,996 inputs agree on code / gas; the case is pinned in
the nightly regression job. This was a harness transport failure, not
a consensus finding.

For handler coverage, `TYPE ... go_ok` counts successful transactions
and successful authz executions, excluding proposal payloads at
submission. `GOV_EXECUTED <type> <count>` counts a proposal's messages
only after Go emits `proposal_passed` for that proposal id. Failed,
rejected and expired proposals contribute no successful executions.
`fuzz-diff coverage` attributes recorded proposals the same way
(`proposal_executed` column). `coverage-table.py` reports generated and
recorded proposal executions separately; a successful submission alone
does not satisfy the per-message success requirement of F1.3.

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

### Runs (2026-10-15)

| Harness | Inputs | Result |
|---|---|---|
| `fuzz-diff` v10 `tx,blobtx,iw,fibre` | 6 × 500 000 (seeds 1–8; ~13 000 decodable txs per run) | findings 1, 5 fixed; then clean |
| `fuzz-diff` v9 `tx,blobtx,iw,fibre` | 4 × 500 000 | finding 4 fixed; then clean |
| `fuzz-diff` `eds` v10 / v9 | 15 000 squares each (plus 8000 before the fixes) | findings 2, 3 fixed; then clean |
| `fuzz-diff` `amino_*` + crafted deep inputs | vectors + mutations | findings 7, 8 |
| `fuzz-diff pp` (ProcessProposal) | `blob` 610, `v9-blob` 800, `v9-exec` 600, `fibre` 2265, `v9-hyperlane` 600, `staking` 400 mutated proposals (≈ 60 % accepted by Go) | no mismatch |
| cargo-fuzz `txdecode` / `amino` / `square` / `shares_parse` | 5 min each: 8.3 M / 14.1 M / 20.2 M / 66.6 M execs | no crash |
| cargo-fuzz `p2p_msgs` / `mconn_recv` / `abci_frame` / `p2p_handshake` | 5 / 5 / 4 / 4 min: 51.0 M / 2.2 M / 4.6 M / 1.4 M execs | no crash |
| cargo-fuzz `rpc_request` / `light_json` | until the first crash (12 k / 19.4 M execs) | findings 9, 10 fixed |
| `fuzz-diff exec` (Simulate), v10: `utf8`, `gov`, `authz-feegrant`, `staking`, `staking-lifecycle`, `vesting`, `hyperlane`; v9: `v9-utf8`, `v9-gov`, `v9-staking`, `v9-authz-feegrant`, `v9-vesting` | 7 990 simulated txs (10 per height), 5 673 with invalid UTF-8 (310 of those executed by Go) | findings 12–16 fixed; then no code / gas / result mismatch. Error-text-only differences remain (open items) |
| `fuzz-diff exec` nil customtypes (`EXEC_MODE=nil`: every `math.Int` / `LegacyDec` field of the scenario's messages, nested ones too, dropped or sent empty), v10: `staking`, `staking-lifecycle`, `gov`, `authz-feegrant`, `vesting`, `hyperlane`, `fibre`; v9: `v9-staking`, `v9-gov`, `v9-authz-feegrant`, `v9-vesting`, `v9-hyperlane` | ~9 500 simulated txs (2026-10-16) | findings 17–20 fixed; then no code / gas / result mismatch |
| `fuzz-diff exec` synthesized messages (`EXEC_MODE=synth`: every `sdk.Msg` reaching a math customtype, built from scratch with absent / empty / small numbers, signed by a scenario account; `EXEC_SYNTH=<name>` narrows it) | v10: `hyperlane`, `gov`, `staking-lifecycle`, `fibre`, `authz-feegrant`; v9: `v9-hyperlane`, `v9-gov`, `v9-staking`, `v9-authz-feegrant` (~8 000 txs) | findings 21–22 fixed; then no code / gas / result mismatch |
| `fuzz-diff` `json_*` (2026-10-16) | v10: 4 × 40 000, v9: 2 × 40 000 | findings 28-32 fixed; then only the open proto3json `null` cases |
| `fuzz-diff` `json_ica_tx`, `json_ica_vb` (proto3json `DeserializeCosmosTx`, then each message's `ValidateBasic`; 2026-10-16) | v10: 3 × 40 000 + 4 × 60 000, v9: 3 × 40 000 (seeds: `null` messages and elements, both field-name forms, duplicate keys, enums / integers / bytes / times / durations / customtypes in each JSON form, `@type` errors; mutations: whole values replaced by `null` / `[null]` / `""` / ..., keys renamed between `snake_case` and camelCase) | finding 33 fixed (v9: the `RemoteRouter` schema in the host's re-marshal); then clean |
| `fuzz-diff` other `json_*` ops with the new mutations, `json_ack_check` | v10, v9: 30 000 each, then all 11 `json_*` ops 2 × 40 000 per version | finding 34 fixed; then clean |
| `ibc-upgrades` fixture regenerated (2026-10-16) with 13 proto3json ICA packets carrying `null`s (a `null` message, `null` coins / outputs / amounts, `null` in `MsgExec.msgs` and `MsgVoteWeighted.options`, a one-byte secp256r1 key, a quoted `null` proposal id, `time.Parse`'s lenient forms) executed by celestia-app v10.4.0 | 237 heights, 3 txs failing with Go panics, the rest error acks | finding 35 fixed; then full parity (app hash, code, gas, data at every height). Remaining log differences: the panic logs (Go's recorded stack trace) and two pre-existing error texts |
| `fuzz-diff exec` `EXEC_MODE=json` | `gov` 940 simulated txs (v10) | finding 29: 161 code / gas mismatches before, none after |
| `fuzz-diff deliver` (signed txs finalized; 2026-10-16) | v10: `gov`, `staking`, `staking-lifecycle`, `hyperlane`, `authz-feegrant`, `fibre`, `vesting`, `zkism-forwarding`, `ibc`, `utf8`, `blob`, `consensus`, `evidence`; v9: `v9-gov`, `v9-staking`, `v9-hyperlane`, `v9-authz-feegrant`, `v9-vesting`, `v9-zkism-forwarding`, `v9-ibc`, `v9-utf8`, `v9-blob`, `v9-exec` (300 generated blocks each after the replay; ~7 300 blocks, ~41 000 txs through `CheckTx`, ~9 000 finalized) | findings 36–45 fixed (the first gov run stopped at the signal app-hash split after 229 blocks); a second round (14 v10 and 10 v9 scenarios, 300 blocks each, ~40 000 more txs) found 47–50, and its CheckTx mismatches where Go failed signature verification led to the `aminosign` op and finding 46; after the fixes the v9 round is clean |
| `fuzz-diff deliver` round 3 (2026-10-16; seeds 31/32, then 41/42 with the multisig accounts) | v10: 12 + 6 scenarios, v9: 10 + 6 scenarios, 300 blocks each (~58 000 more txs through `CheckTx`) | findings 51–54 fixed; the reruns of both seeds are clean; round 4 (seeds 51/52, all 22 scenarios, 300 blocks each, ~37 000 txs through `CheckTx`, ~2 300 of them multisig-signed, ~16 % of those accepted) is clean |
| `fuzz-diff deliver` rounds 5–6 (2026-10-16; seeds 61–98, all 22 scenarios each; new flows: legacy gov votes, deposits and text proposals, proposal cancellation by its proposer, vesting accounts for fresh addresses, ICS-20 transfers; app-hash splits dump the differing store entries) | ~24 runs of 10–12 scenarios, 300 blocks each (~400 000 txs through `CheckTx`) | findings 55–60 fixed; reruns of the failing seeds clean; log-only differences down ~80 % |
| `fuzz-diff deliver` round 7 (2026-10-16; seeds 201–212 and 301–340; gas limits just below what Go used for the same message type (7× more out-of-gas inside handlers), hyperlane owner operations on objects the fuzz accounts created, multisig ISMs with sorted validator sets) | 52 runs of 10–12 scenarios, 300 blocks each (~800 000 txs through `CheckTx`) | finding 61 fixed; otherwise clean; then seeds 401–420 and, with the default (long) slashing, unbonding and voting parameters (`DELIVER_KEEP_PARAMS`), seeds 501–508: clean |
| `fuzz-diff deliver` round 8 (2026-10-16; seed 701; gas sweep, hyperlane process / pay-for-gas / announce flows, gov proposals of `MsgExecLegacyContent` and staking `MsgUpdateParams`, `ibc-edge` with a generated genesis) and `DELIVER_SCRIPT` cases | `staking-lifecycle`, `gov`, `hyperlane`, `authz-feegrant`, `ibc-edge`, 200 blocks each (~9 900 txs through `CheckTx`, ~4 200 gas-sweep copies) | sweep clean; finding 64 found with a scripted `MsgGrant`; `ibc-edge`'s recorded client updates carry part-set totals of 10 000 (finding 63) |
| `fuzz-diff --ops json_ica_pb` (2026-10-16; protobuf ICA `CosmosTx`: devnet txs' messages and fee-grant / authz / gov seeds with wide keys, groups, overlong varints, `+N` amounts, then protobuf mutations) | v10, v9: 6 × 1 000 000 each | findings 62-64 fixed; the seeds that differed before the fix are clean |
| `fuzz-diff deliver` round 9 (2026-10-17; seeds 801/802, 901, 911/912, 921/922; `DELIVER_IBC` (the mock counterparty) and `DELIVER_SWEEP` on every run; hyperlane owner operations on owned objects) | v10: 15 scenarios, then 8 again; v9: 10, then 7; 200–300 blocks each (~95 000 txs through `CheckTx`, ~38 000 gas-sweep copies) | clean (one `utf8` run stopped when a parallel run's cleanup removed its Go home directory; its rerun is clean). Go executed successfully for the first time: `MsgSubmitMisbehaviour`, `MsgUpgradeClient`, `MsgChannelOpenAck` / `Confirm` / `Try` of new channels, `MsgChannelUpgradeTry` / `Confirm` / `Open` / `Cancel` / `Timeout`, `MsgChannelCloseConfirm`, `MsgTimeoutOnClose`, ICA host txs, and the hyperlane owner operations (`MsgSetMailbox`, `MsgSetToken`, `MsgSetIgpOwner`, `MsgSetDestinationGasConfig`, `MsgEnrollRemoteRouter` / `MsgUnrollRemoteRouter`, routing ISM domains and owner) |
| `DELIVER_SCRIPT` gov proposals (2026-10-17; fuzz accounts delegate, one proposal each of `MsgExecLegacyContent` (text) and staking `MsgUpdateParams`, all accounts vote yes) | `gov`, `v9-gov`, 40 blocks | both proposals pass and execute on v10 (Go and Rust alike); on v9 the staking one does, the text proposal fails identically (no legacy handler for it). Never reachable in either version: ICS-27 controller messages (celestia-app wires no controller), mint `MsgUpdateParams` (celestia's mint has no `Msg` service), crisis (not wired), `MsgChannelUpgradeInit` / `Ack` and the other IBC `MsgUpdateParams` / `MsgRecoverClient` / `MsgIBCSoftwareUpgrade` except through gov |
| `fuzz-diff --ops aminosign` (2026-10-16; seeds: the delivery generator's amino-JSON-signed txs, nested messages included) | v10, v9: 1 400 000 inputs (~110 000 decodable); a second round with the multisig signers and new flows: 1 200 000 more | finding 46 fixed; then clean (the second round's only differences were Go's `GetSigningTxData` dereferencing a nil fee, which the ante handler reaches first on chain; the harness models it) |
| `utf8` / `v9-utf8` devnet scenarios (Go v10.4.0 / v9.0.8 behind celestia-core) | 17 workload txs each, 62 / 57 heights | full parity (app hash, tx results, CheckTx, proposals, logs and events byte for byte); `utf8` query corpus (3 537 ABCI + 2 264 REST) identical |

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
| 9 | RPC URI arguments (`rpc::params::uri_arg`) | `arg[..2]` sliced inside a multi-byte character (`?height=aʰ`): handler panic | remote panic per request | byte-wise `0x` check (Go `strings.HasPrefix(strings.ToLower(arg), "0x")`); `params` unit test |
| 10 | light client (`light::http::parse_rfc3339`) | a non-ASCII time-zone offset from an RPC provider was sliced by byte index: panic | malicious provider crashes the light client / state sync | byte checks of `±hh:mm` like Go's RFC 3339 layout; `http` unit test |
| 11 | every executed message (both versions) | gogoproto keeps any bytes in a `string` field; prost rejected invalid UTF-8, so the router failed (`tx parse error`) what Go executes and stores: validator descriptions, proposal titles / summaries / metadata, vote metadata, memos, grants, `MsgExec` / gov / ICA inner messages | any user splits the network with one tx | ADR 0005: strings carried in Go form (`gostr`), `prost-gostr` codec for the generated crates, Go semantics at every byte-level use (lengths, keys, JSON in and out, amino-JSON sign bytes, `%q`, `EqualFold`, bech32 errors); `utf8` / `v9-utf8` scenarios, `fuzz-diff exec`, `utf8` query corpus |
| 12 | `MsgExec` / gov proposal / ICA inner messages | Go's `GetMsgV1Signers` re-packs the message and unpacks it with the `dynamicpb` checks (finding 1); Rust read the signers directly (accepting what Go rejects once strings decode) | gas / error divergence | `signers::get_signers_go` (schema moved to `celestia-proto`), `txdecode::gogo_repack` |
| 13 | gov `MsgSubmitProposal` / `MsgDeposit` | Rust sanitized the initial deposit before reading the params (gas) and treated an empty amount as invalid, where Go validates the decoded slice after the params and panics on a nil `math.Int` | gas / code divergence (also for valid UTF-8) | `sdk::coins::raw_coins_is_valid` / `raw_coins_string` |
| 14 | staking `MsgCreateValidator` | an empty commission rate / max change rate / min self delegation is a nil customtype in Go: `CommissionRates.Validate` panics; Rust used zero (and could succeed) | success vs panic | nil emulation in `handle_create_validator` |
| 15 | bank `MsgUpdateParams`, `MsgSetSendEnabled`; gov `Proposal.Messages` | not routed in Rust (gov proposals carrying them failed at submission); Go stores proposal messages re-packed by gogoproto (`SetMsgs`), e.g. with `Plan.time`'s zero timestamp | gas / state divergence | handlers ported; `gogo_repack` in `SubmitProposal` / legacy content |
| 16 | v9 `x/warp` (`receiver_contract`) | hyperlane-cosmos v1.1.0 stores the receiver contract as a string, compares it lowercased and decodes it only when dispatching; Rust parsed a hex address at enrollment | v9 enrollment / transfer divergence | `RemoteRouter.receiver_contract: String` with v9 / v10 semantics |
| 17 | staking `MsgEditValidator` | a present but empty `commission_rate` / `min_self_delegation` (nullable customtypes) is a non-nil pointer to a nil value in Go: the msg server panics past the address / description checks; Rust read it as absent (success) | success vs panic | the tx decoder keeps the empty occurrence (`gogo::Mode::Struct`), `x_staking::handle_edit_validator_wire` reads it; `txdecode` / `x-staking` unit tests |
| 18 | warp `MsgRemoteTransfer`, hyperlane `MsgPayForGas` | nil `amount` (`sdk.NewCoin` panics `amount is nil`), `gas_limit` (`IsZero` / `Add`) and `max_fee.amount` (`sdk.NewCoins`) panic in Go at their first use; Rust used zero (and could succeed) | success vs panic | `x_warp::TransferNils`, `pay_for_gas_msg` / `quote_gas_payment` with an optional gas limit (`<nil>` in v9's `EventGasPayment`) |
| 19 | authz `MsgExec`, ICA host txs | `GetMsgV1Signers` marshals the message with gogoproto, and `math.Int.MarshalTo` (pointer receiver) sets a nil value to zero in place: messages executed by authz / the ICA host see zero where a top-level message sees nil (a nil `LegacyDec` stays nil) | panic vs error / success | `txdecode::gogo_marshaled` before authorization and execution; `txdecode` unit test |
| 20 | gov `MsgDeposit` | an empty amount fails `validateDeposit` (`IsAllPositive` of no coins) with `invalid coins`; Rust went on to the proposal checks | code / gas divergence | `x_gov::msgs::deposit` |
| 21 | distribution `MsgDepositValidatorRewardsPool` | Go sends the coins as decoded: `IsValid` fails on an earlier coin or panics on the first nil amount; Rust failed decoding the amount first | panic vs error | `raw_coins_is_valid` before the send |
| 22 | v9 `Simulate` / `TxDecode` (gRPC, ABCI query, REST) | decoded with the v10 registry at app version 9: fibre messages and `MsgForward` fields 7/8 decoded and ran where v9.0.8 fails `tx parse error` | query divergence (not consensus) | `txpipe::decoder(app_version)`, `txdecode::default_tx_decode_v9` |
| 23 | v9 hyperlane-cosmos v1.1.0 | multisig ISMs compare the recovered signer with each stored validator lower-cased (no decoding), `ValidateNewMultisig` sorts the strings; the IGP fee check is `requiredPayment.IsAllGT(maxFee)`; `MsgPayForGas` pays `req.Amount` without a quote and refuses zero; `SetDomain` on an existing domain keeps its ISM (loop copy); v1.1.0's amino names sign `SIGN_MODE_LEGACY_AMINO_JSON`; Rust ran v1.3.0's semantics at app version 9 | v9 consensus divergence | `is_v9` branches in `x-hyperlane` (ADR 0004) |
| 24 | `x_auth::parse_bech32` (bank send / multi-send, transfer, ...) | Go's address codec runs `VerifyAddressFormat` (at most 255 bytes); Rust accepted a longer bech32 address (found reading the Go source while fixing finding 21's text) | success vs `invalid address` | the length check |
| 25 | `crypto::secp256k1::verify` (CometBFT keys: votes / commits in `ctypes` and `tendermint-types`, ibc-go 07-tendermint headers in `x-ibc` `light.rs`) | k256 rejected `r` / `s` at or above `n`, where celestia-core v0.42.3's `PubKey.VerifySignature` (decred secp256k1 v4.4.0, the version celestia-app v10.4.0 / v9.0.8 build) reduces them modulo `n` (`ModNScalar.SetByteSlice`): `r + n` (when it fits in 32 bytes) verified in Go, `s + n` too for small `s`. Its other rules (low-S through a DER round trip, zero `r` / `s`, `02` / `03` keys with `x < p` on the curve) are cosmos-sdk's. ibc-go v8.7.2 runs this code: celestia-app's `replace` points cometbft at celestia-core (v0.42.3 for v10, v0.40.9 for v9; identical `crypto/secp256k1`), and ibc-go's own `replace` lines do not apply to a dependency. Batch commit verification is only used when every key is ed25519 (`AllKeysHaveSameType`), so secp256k1 commits are always single-verified | light-client header / commit and vote acceptance divergence on a crafted signature | `verify` is `verify_go` (libsecp256k1), k256 left the `crypto` crate; `conformance/fixtures/secp256k1/verify_cometbft.json` (563 vectors from celestia-core's own function, results identical to cosmos-sdk's), `crypto` test `cometbft_vectors` |
| 27 | `countExecutableMsgs` (CheckTx, PrepareProposal, ProcessProposal) | the ICS-27 packet's message count parsed the payload with serde: invalid UTF-8 in a string of a proto3json `CosmosTx` (or of the packet JSON) failed to parse, so the packet counted as `MaxSDKMessages`, where Go's `encoding/json` reads it as U+FFFD; also `foldName` key matching (`meſſages`), last-key-wins and the sticky type error of `json.Unmarshal`, and the packet / metadata JSON read through the ICA host's jsonpb ports | a `MsgRecvPacket` Go counts as 1 + n: Rust's ProcessProposal rejects the block (consensus split), CheckTx rejects the tx | `msg_count::count_json_msgs` (Go `encoding/json` semantics, cases checked against Go 1.26.6), `x_icahost::host::decode_packet_data`; `ibc-upgrades` fixture (found by its ICA invalid-UTF-8 txs) |
| 26 | v9 bank blocked addresses, `ModuleAccounts` | celestia-app v9.0.8's `maccPerms` has no `fibre` account: its address is not blocked (a `MsgSend` to it succeeds) and no module account is listed or created for it; Rust used v10's list at app version 9 | v9 consensus divergence (exotic: a send to the fibre module address) | `x_auth::module_account_perms` (found by the v9 query corpus) |
| 28 | `countExecutableMsgs` (CheckTx, PrepareProposal, ProcessProposal; v10) | `json.Unmarshal`'s `checkValid` fails a proto3json `CosmosTx` nested deeper than 10 000 levels anywhere (Go counts the packet as `MaxSDKMessages`); serde's `IgnoredAny` skipped any depth, so Rust counted 1 + n. Also a lone `\ud800` escape in a nested key failed serde | a `MsgRecvPacket` Rust counts low and Go at the limit: Rust's ProcessProposal accepts a block Go rejects | `msg_count::count_json_msgs` on `gojson` (whole-input check, `EqualFold`, last key wins, sticky type errors); unit test, regressions |
| 29 | gov `MsgSubmitProposal` metadata (both versions) | Go decodes any metadata (the length check comes later) up to 10 000 levels; serde failed at 128 levels and on `1e400`, matched keys by ASCII case only (`ſummary` is `summary` in Go), sorted and deduplicated keys (`{"title":"b","Title":"a"}` is `a` in Go) and let a later key clear a type error or `null` overwrite a title | Go fails with `invalid proposal content` where Rust went on (different gas or code): consensus split from one tx | `x_gov::msgs::metadata_title_summary` on `gojson`; unit test, regressions, `EXEC_MODE=json` exec run |
| 30 | PFM memo (`OnRecvPacket`, retry on timeout; both) | the recursive memo parser had no depth limit: a memo nested more than 10 000 levels (Go: not a forward) was forwarded, and a deep one overflowed the stack (node abort); a number outside `float64` range anywhere fails Go's map decode (not a forward); field names matched by ASCII case only (`retrieſ` is `retries` in Go); `raw` copies were quadratic in the depth | consensus split or remote crash from one relayed packet | `memo.rs` on `gojson` (spans instead of copies, `next` marshaled at decode, large stack for long memos); unit tests, regressions |
| 31 | `jsonpb` decodes (ICS-20 packet data, acks, ICA packet data and metadata; both) | serde_json rejected a lone surrogate escape (`\ud800`) that Go reads as U+FFFD; `MetadataFromVersion` parsed the UTF-8 of the version's Go form instead of its bytes (invalid UTF-8 in a channel version) | an ICS-20 packet with such a denom: Go receives, Rust acknowledged an error | `x_ibc::gojson::first_value` on `gojson`; `metadata_from_version` on the Go bytes; unit test, regressions |
| 32 | ICA host `DeserializeCosmosTx` (both encodings) | `UnpackInterfaces` after the decode fails past 10 nested `Any`s or 100 unpack calls (`MsgExec` in `MsgExec` ...); Rust had no such check for proto3json | error ack in Go, execution in Rust | `txdecode::unpack_cosmos_tx_msgs` in both branches; regressions |
| 33 | ICA host proto3json `DeserializeCosmosTx` (both versions) | Go decodes with gogoproto `jsonpb` over the Go structs (reflection), Rust decoded with the registry's serde JSON. Go: a `null` element of `[]T` is `T{}` (`"amount":[null]` is a `Coin` with an empty denom, marshaled with amount `0`), of `[]*T` / `[]*Any` a nil pointer, which gogoproto's `Marshal` of the enclosing `Any` dereferences: Go **panics** inside `UnmarshalJSON` (`MsgExec.msgs`, gov `MsgSubmitProposal.messages`, `MsgVoteWeighted.options`, `MsgSetSendEnabled.send_enabled`, ...) and the whole `MsgRecvPacket` tx fails with `ErrPanic`; a `null` message (`"messages":[null]`) is a nil `sdk.Msg`: `authenticateTx` fails it (`message type not allowed: /` or `ErrPackAny`, code 33, in the error ack). Also: `null` leaves any non-pointer value unchanged (also inside quotes: `"proposal_id":"null"` is 0), integers may be quoted (the content is decoded as JSON: `" 7 "` is 7), enums by name (raw bytes between the quotes; `"1"` fails) or by any int32, `bytes` as base64 (newlines skipped, trailing data fails) or as an array of numbers, `json_name` beats `orig_name`, the last duplicate key wins, a oneof's interface field takes only `null`, customtypes through their `UnmarshalJSON` (`math.Int` fails `""`, `LegacyDec` from a decimal string, `HexAddress` with an optional `0x`, secp256r1 `ecdsaPK` slices `data[1:len-1]` of the raw value: a one-byte value such as `1` **panics**), `stdtime` through `time.Parse(RFC3339Nano)`'s layout path (one-digit hour, `,`, any number of fraction digits, offsets up to `24:60`), `stdduration` through `time.ParseDuration`. Rust rejected every `null` element and accepted or rejected most of the rest differently. In the protobuf encoding an `Any` with an empty type URL is a nil message too (Rust failed it as unregistered), and an `UnpackInterfaces` panic (solo machine `ClientState` without a consensus state) panics in both encodings (Rust acked an error) | ack, code and gas divergence (`LastResultsHash`, app hash) from one relayed packet | `txdecode::jsonpb` (table of the Go struct shapes generated by `conformance/txdecode -jsonpb-only` from the v10 registry; v9 differences, checked against a v9 dump: `MsgForward` fields 7/8, `RemoteRouter.receiver_contract` a string), `unmarshal_cosmos_tx_json`; `deserialize_cosmos_tx` returns nil messages, `authenticate_tx` fails them like Go; the host's gogoproto helpers run with v9's schema at app version 9; `time::parse_rfc3339_nano`; unit tests, regressions, `ibc-upgrades` fixture (proto3json `null` packets) |
| 34 | `jsonpb` decodes of `channeltypes.Acknowledgement` and `InterchainAccountPacketData` (both versions) | a `[]byte` field (`result`, `data`) given as a JSON array of numbers is valid in Go (`encoding/json`) and was rejected; base64 with data after the padding (`AQ==AAAA`) was accepted; `{"result":null}` sets the `result` member (nil bytes, marshaled back as `null`, so `AcknowledgePacket`'s marshalling check passes) where Rust saw no response (and failed the check); `{"error":null}` is an empty error; `"type":-0` is 0 in Go (an `int32`), Rust failed it; a non-`null` `response` (the oneof's interface field) fails in Go | `AcknowledgePacket` with an array-form ack Go rejects (`acknowledgement marshalling error`) passed the check in Rust (the decode failed, so no check ran); ICA packet data with array `data` executed in Go, error ack in Rust | `gojson::bytes_value`, `gojson::decode_base64_std` (Go's `decodeQuantum`), `decode_acknowledgement` / `decode_packet_data` on the `gojson` tree; `json_ack_check` op; regressions |
| 35 | IBC core `RecvPacket` (both versions) | an app callback that fails (only a Go panic does: the IBC apps turn errors into acknowledgements) returned with `?` and left the callback's cache branch open: the failed tx then discarded that branch only and kept the message branch (the packet receipt, the callback's writes) | same code, gas and log as Go, different app hash (found by the `ibc-upgrades` fixture's proto3json `null` packets once finding 33 made Rust panic like Go) | the branch is discarded on the error; `run_msgs` discards every branch above its own on failure; `ibc-upgrades` fixture |
| 36 | x/auth `MsgUpdateParams` (both versions) | registered and routed in Go (`x/auth/keeper/msg_server.go`: authority check, `Params.Validate`, store), unrouted in Rust: Rust failed it in `runTxAnte` (`no message handler`, code 6, gas 1180, no fee, no sequence increment) where Go runs the ante handler and the handler (code 1, fee and sequence taken); a gov proposal carrying it could not be submitted in Rust and executes (auth params change) in Go | any user splits the network with one tx (`LastResultsHash`, app hash) | `router::handle_auth_update_params`; `app` unit test |
| 37 | ICA *controller* messages (`MsgRegisterInterchainAccount`, `MsgSendTx`, `MsgUpdateParams`; both versions) | celestia-app has no controller keeper (no msg server), but the types are in the interface registry and `runTxAnte`'s `validateBasicTxMsgs` runs their `ValidateBasic` before the handler check: Go fails with the ValidateBasic error (`host`/2 `invalid connection ID`, `ibc`/5, `interchainaccounts`/6, ...), Rust with `no message handler` (`sdk`/6). PrepareProposal's filter runs neither, so such a tx is included and the codes differ in `FinalizeBlock` | `LastResultsHash` split from one tx | `x_icahost::host::controller_validate_basic`; `no message handler found for %T` with Go's type name; `app` unit test |
| 38 | PrepareProposal `FilteredSquareBuilder.Fill` (both versions) | celestia's `HandlePanicDecorator` re-panics and `Fill` does not recover: a Go panic in the ante chain (e.g. `signerInfo.PublicKey cannot be nil`, a signer info without key) aborts the whole PrepareProposal and `BaseApp` returns the request's txs unchanged (no square); Rust dropped the tx and built the block | a Rust proposer builds a different block than a Go proposer (Go validators accept it; not a safety issue) | `proposal::ante_fill` |
| 39 | feegrant `MsgGrantAllowance` / `MsgRevokeAllowance` (both versions) | an absent allowance is a nil `*Any` whose `GetCachedValue` Go dereferences: **panic** (`ErrPanic`), Rust returned `no allowance` (code 5); `AllowedMsgAllowance.ValidateBasic`'s order (nil inner allowance, then empty message list, then `GetAllowance`) differed (`no messages` vs `no allowance`); both handlers decode addresses with the account address codec (wrong prefix: `ErrLogic`, code 35), Rust with `AccAddressFromBech32` (code 1) | code / gas split from one tx | `handle_grant_allowance`, `Allowance::check_shape`, `sdk::address::acc_codec_to_bytes` |
| 40 | bank `MsgSend` / `MsgMultiSend`, distribution `MsgFundCommunityPool` / `MsgCommunityPoolSpend` (both versions) | `Coins.IsValid` / `Validate` on the coins as decoded: a coin without amount (nil `math.Int`) that `Validate` reaches **panics** (`Sign` of a nil `big.Int`); Rust failed decoding the amount first (`invalid coins`); the error text is `Coins.String()` of the raw coins (`<nil>` amounts) | panic vs error: code split (as finding 21 for `MsgDepositValidatorRewardsPool`) | `raw_coins_is_valid` in `x_bank::msgs::valid_raw_coins` and distribution `validate_amount` |
| 41 | signers of `MsgExec` inner messages, gov proposal messages and ICA host messages (both versions) | `GetMsgV1Signers` decodes signer strings with the signing context's address codecs: a wrong prefix (`cosmos1...` with the granter's or gov's bytes) is `ErrLogic`, the validator codec for `ValidatorAddressString` signers. Rust decoded any bech32 string without a prefix check: an inner message "signed" by `cosmos1<granter>` ran under the granter's grant, a proposal message "signed" by `cosmos1<gov>` was accepted | authz / gov execution Go refuses: state split | `x_ante::txview::signer_bytes` shared by authz, gov and the ICA host |
| 42 | signal `MsgSignalVersion` / `MsgTryUpgrade` (both versions) | the msg server compares with `sdkCtx.BlockHeader().Version.App`, and `BaseApp` builds every header without `Version`: it is always 0, so a validator may signal any version (also one below the current app version) and a quorum on version 0 fails `TryUpgrade`; Rust compared with the consensus params' app version (10) and rejected lower signals | a validator's signal of an older version splits the network | `x_signal::HEADER_APP_VERSION`; `x-signal` unit test |
| 43 | zkism `MsgSubmitMessages` / `MsgUpdateInterchainSecurityModule` (both versions) | `id` is a non-nullable `HexAddress` customtype: absent, it is the zero address (`ism identifier must be non-zero`, `sdk`/18); Rust parsed the empty string (`invalid hex address length`, `sdk`/2) | code split from one tx | `decode_id` with `HexAddress::from_proto` |
| 44 | ICA host `MsgUpdateParams` (both versions) | `Params.Validate` is a plain `fmt.Errorf` (`undefined`/1, `%s` of the slice: `[a b]`); Rust used `ErrInvalidRequest` (code 18) | `ValidateBasic` (CheckTx) and gov execution code split | `x_icahost::validate_params` |
| 45 | forwarding `MsgForward.ValidateBasic` (both versions) | `MaxIgpFee` is a non-nullable `Coin`: without amount, `Coin.Validate` fails `amount is nil`; Rust read it as zero and went on (to the ante handler) | code split from one tx | `validate_coin` |
| 46 | `SIGN_MODE_LEGACY_AMINO_JSON` sign bytes (x/tx v0.13.9 `aminojson`, both versions) | (a) `GetSignBytes` runs x/tx `decode.RejectUnknownFields(body, allowUnknownNonCriticals=false)` over the interface registry's descriptors, where ibc-go's `tx.proto` resolves `UpgradeFields`, `Upgrade` and `ErrorReceipt` as *placeholders* (no fields): any content in `MsgChannelUpgradeInit.fields`, `...Try.counterparty_upgrade_fields`, `...Ack` / `...Confirm.counterparty_upgrade`, `...Cancel.error_receipt` is an unknown field, and so is any non-critical unknown field at any depth; (b) `marshalAny` unmarshals every nested `Any` with protobuf-go: `dynamicpb` types reject invalid UTF-8 in proto3 strings (the `GetSigners` rule of finding 1, now for messages inside `MsgExec`, proposals, `v1beta1` content, authorizations, ...), and every occurrence of an `Any`'s `type_url` must be UTF-8; (c) the generated (pulsar) unmarshal of the `protoregistry.GlobalTypes` types keeps the *last* occurrence of a singular message field (`dynamicpb` and gogoproto merge them); a fee coin without amount is `<nil>`. Rust encoded all of these, so it verified signatures Go rejects (and rejected signatures over Go's sign doc) | a tx signed in amino-JSON executes on one side only: state split | `aminojson::reject_unknown_fields` (placeholder fields from the new oracle op `placeholders`), `unpack_check` in `marshal_any`, `pulsar_last_wins`; oracle op `aminosign` (`fuzz-diff --ops aminosign`), 26 regressions |
| 47 | gov `v1beta1.MsgSubmitProposal` and `MsgExecLegacyContent` (both versions) | without content, `GetContent` / `LegacyContentFromMessage` call `GetCachedValue` on a nil `*Any`: **panic** (also inside `SubmitProposal`'s check of a proposal's `MsgExecLegacyContent`); Rust returned `invalid proposal content` (code 5) | code split from one tx | `go_panic` in `legacy_submit_proposal` / `exec_legacy_content` |
| 48 | vesting `MsgCreateVestingAccount` (both versions) | `validateAmount`'s `IsValid` reaches a nil amount: **panic**; Rust failed decoding the amount (`invalid coins`) | code split from one tx | `valid_raw_coins` |
| 49 | distribution `MsgDepositValidatorRewardsPool` / `MsgCommunityPoolSpend` (both versions) | the depositor, validator and recipient address-codec errors are returned unwrapped (`undefined`/1 for an empty or malformed address, `ErrLogic` for a wrong prefix); Rust used the staking decoder's `ErrInvalidAddress` (code 7) | code split from one tx | `sdk::address::{acc,val}_codec_to_bytes` |
| 50 | forwarding `MsgForward.ValidateBasic` (both versions) | `errors.Wrap` keeps the code of `AccAddressFromBech32`'s error (`ErrUnknownAddress`, code 9, for an empty or over-long address); Rust made every failure code 1 | CheckTx / FinalizeBlock code split | `SdkError::wrap` |
| 51 | multisig signatures (`ModeInfoAndSigToSignatureData`, both versions) | Go sizes a multisig's sub-signatures by the `MultiSignature` entries (`make([]SignatureData, len(sigs))`) and fills only those with a mode info: a `MultiSignature` with more entries than `mode_infos` carries nil sub-signatures, and `SetPubKeyDecorator`'s event loop fails with `got empty SignatureData` (code 1). Rust built the list from the mode infos only, so the extra entries vanished and the multisig verified: Rust accepted (`CheckTx`, `PrepareProposal`) and executed txs Go rejects | state split from one tx signed by a multisig account (anyone holding a threshold of its keys) | `SigData::Multi` sub-signatures are `Option`s (`None` past the mode infos; `improper signature data type for index n` in verification); found by the deliver harness's new multisig accounts |
| 52 | authz grants (both versions) | `SendAuthorization.ValidateBasic`'s `IsAllPositive`, `StakeAuthorization`'s `MaxTokens.IsNegative` and `TransferAuthorization`'s `SpendLimit.Validate` reach a nil amount: **panic**; Rust failed decoding (`invalid coins`, code 10). `TransferAuthorization` also wraps `Coins.Validate`'s own text (`invalid denom: u`, `coin 0utia amount is not positive`), not `invalid spend limit` | code split from one `MsgGrant` | `sdk::coins::{raw_is_positive, raw_is_negative, raw_coins_validate}` |
| 53 | feegrant allowances (both versions) | `BasicAllowance` / `PeriodicAllowance.ValidateBasic`'s `IsValid` reaches a nil amount: **panic**; the error texts print the coins as decoded (`<nil>` amounts) | code split from one `MsgGrantAllowance` | `raw_valid`, `raw_coins_string` |
| 54 | staking `MsgEditValidator` (both versions) | the nil `commission_rate` dereference comes after the `min_self_delegation` check: a non-positive minimum (`-0`) with an empty commission rate is `ErrInvalidRequest` (code 18) in Go; Rust panicked first | code split from one tx | check order as Go |
| 55 | `Coins.Add` past 256 bits (both versions) | `math.Int.Add` panics on overflow (`integer overflow`), e.g. `ValidateInputOutputs` summing a `MsgMultiSend`'s outputs (`2^256-1` and `1000`); Rust returned `ErrInvalidCoins` (code 10). Same for a balance or supply sum | code split from one tx | `sdk::coins::add_panic` in `Coins::add`, `add_coins`, `mint_coins` |
| 56 | staking `MsgCreateValidator` (both versions) | `types.NewValidator` packs the unpacked consensus key again (`NewAnyWithValue`): the stored `ConsensusPubkey` is canonical. Rust stored the tx's `Any` as received, so a key encoded with a repeated field (the same key twice) made the validator record longer: different gas and **app hash** | state split from one valid `MsgCreateValidator` | `PubKey::to_any` |
| 57 | feegrant / authz grants (both versions) | `feegrant.NewGrant` / `authz.NewGrant` re-marshal the decoded allowance / authorization (gogoproto: customtype amounts as `Int.String()`, `+1` is `1`; repeated singular messages merged), while a nested `Any` (an `AllowedMsgAllowance`'s inner allowance) keeps the bytes as received until `SetAllowance` re-packs it. Rust stored the prost re-encoding of the raw values (`+1`): different gas and **app hash** | state split from one `MsgGrantAllowance` / `MsgGrant` | `txdecode::gogo_repack` of the message's `Any` at grant time, `Allowance::to_canonical_any` after a change |
| 58 | gov proposal messages at app version 9 | `SetMsgs` / `NewLegacyContent` re-marshal with the executing binary's types: v9.0.8 (hyperlane-cosmos v1.1.0) has `RemoteRouter.receiver_contract` as a string, the v10 fork as a non-nullable `HexAddress` (66 bytes when absent). Rust re-packed with the v10 schema at app version 9: a `MsgEnrollRemoteRouter` proposal was 68 bytes longer (submit-proposal gas, proposal writes): gas split, out-of-gas at a different point | gas / result split from one proposal | `gogo_repack` / `gogo_marshaled` follow the executing app version (`sdk::version`) |
| 59 | ante logs (both versions; log only) | `SetUpContextDecorator` recovers a later decorator's out-of-gas panic as `out of gas in location: <descriptor>; gasWanted: N, gasUsed: N`; Rust logged `out of gas` (store reads) or the location only. `DeductFeeDecorator` prints the fee payer `[]byte` with `%s` (raw bytes), not bech32 | `CheckTx` / `DeliverTx` log text | `run_ante` rewrites the out-of-gas error; `gostr::from_go_bytes` |
| 60 | distribution `CalculateDelegationRewards` (both versions) | `IterateValidatorSlashEventsBetween` runs the handler (historical-rewards reads of `calculateDelegationRewardsBetween`) while the iterator stands on the event, before the `Next` that charges it; Rust collected every event first and then read. Same total gas, but a tx running out of gas inside the loop (a `MsgDelegate` to a slashed validator with a tight gas limit) stopped at a different point: `gas_used` differed (`LastResultsHash`) | result split from one tx | `store::for_each_slash_event_between` |
| 61 | message field keys past `u32::MAX` (both versions) | gogoproto truncates a field number to `int32` (`fieldNum := int32(wire >> 3)`): a key past `u32::MAX` is an unknown field to skip (or aliases a known field); prost rejects it (`invalid key value`). The tx decoder ports gogoproto, but the router decoded each message's bytes with prost again: Rust failed a message Go executes (`ValidateBasic` and handlers) | code split; a valid message with such a field executes on Go only: state split | `txdecode::prost_compatible` (gogoproto's decoding re-encoded) before `route` / `validate_basic` |
| 62 | ICA host protobuf `CosmosTx` (both versions) | the host decodes the packet's messages with gogoproto and runs them without the tx decoder's unknown-field check. gogoproto skips what prost cannot: a field key past `u32::MAX`, a group whose end tag does not match its start, a 10-byte varint whose last byte is above 1. In the `Any` of a message (kept as `XXX_unrecognized`) Rust failed the whole `CosmosTx`; inside a nested `Any` (a fee allowance, an IBC client message, an authorization) Rust failed decoding that value. At app version 9 `MsgForward` has no fields 7 and 8: Go skips them, Rust read them as v10's custom hook (or failed on another wire type) | an ICA packet executes on Go and fails on Rust (different acknowledgement: state split); `MsgGrantAllowance` is in celestia's default host allow list | `gogo::Mode::Struct` drops the unknown fields prost cannot skip and re-encodes nested `Any`s with gogoproto's decoding (except where Go stores them as received: an `AllowedMsgAllowance`'s inner allowance, the messages of `MsgExec`, of a proposal and of the `CosmosTx`); fee-grant allowances decode through `prost_compatible`; v9 `MsgForward` schema; new oracle op `json_ica_pb` (protobuf `CosmosTx` + `ValidateBasic`), regressions |
| 63 | IBC `ValidateBasic` (both versions) | (a) 07-tendermint: `PartSetHeaderFromProto` rejects a part-set total above `MaxBlockPartsCount` (2049) in a header's `LastBlockID` or its commit's `BlockID`; Rust did not check it, so a `MsgUpdateClient` / misbehaviour Go rejects in `ValidateBasic` (CheckTx, code 1) went on to the handler in Rust (another code and gas). (b) 06-solomachine: `len(publicKey.Bytes())` of a multisig key amino-encodes it, which panics on a sub-key that was never unpacked (empty type URL) or has no amino registration (secp256r1); Rust returned `ErrInvalidHeader` / `ErrInvalidConsensus` or passed | (a) code / gas split from one tx anyone can send; (b) panic vs error | `light::validate_block_id` checks the count first (the ibctesting fixtures were recorded on celestia's `test/interop` setup, which raises it to 10 000: their replay sets that); `solomachine::key_missing` |
| 64 | repeated `stdtime` / `stdduration` fields (both versions) | gogoproto's `StdTimeUnmarshal` / `StdDurationUnmarshal` overwrite the field on each occurrence; prost merges the occurrences (an empty last one leaves the first). A `MsgGrant` whose `grant.expiration` repeats with an empty last occurrence expires at Unix 0 in Go (`expiration must be after the current block time`) and was granted in Rust; the same in a nested `Any` (a fee allowance's expiration or period), and in the ante handler's parameter filter (`unbonding_time` of a gov-proposed staking `MsgUpdateParams`) | state split from one tx anyone can send | the router, `ValidateBasic` and the ante decorators decode messages through `txdecode::prost_compatible`, now always gogoproto's decoding re-encoded (nested `Any`s too); found with the new `DELIVER_SCRIPT` |
| 65 | `GetSigners` of pulsar types (x/tx `anyutil.Unpack`, both versions) | The generated (pulsar) unmarshal of a `protoregistry.GlobalTypes` type reads like gogoproto, but decodes each embedded message with `options.Unmarshal`: a well-known type (`google.protobuf.Timestamp`, `Duration`, `Any`) goes through protobuf-go's table-driven decoder, which rejects field numbers past 2^29 - 1 (`cannot parse invalid wire-format data`), 10-byte varints with a last byte above 1, mismatched groups and invalid UTF-8 in `Any.type_url`. A non-critical unknown field (bit 11 set) numbered 2^29..2^31 passes the tx decoder's `RejectUnknownFields` in the body, so `MsgGrant{grant.expiration: {..., field 2^31-1}}` (also gov `MsgUpdateParams` durations, `MsgSoftwareUpgrade` plan time, any `Any` in a pulsar message) fails `GetSigners` in Go (the ante handler rejects the tx) while Rust checked only the `dynamicpb` types and executed it; the `MsgExec`, gov and ICA paths read re-marshaled bytes, where gogoproto keeps an `Any`'s unknown fields (`XXX_unrecognized`) | state split from one `MsgGrant` anyone can sign (DIRECT mode) | `signers::pulsar_check` in `unpack_check`; found by `fuzz-diff wire` (`unknown` class, depth 3); 12 regressions |
| 66 | ICA host `authenticateTx` (both versions) | The host decodes a protobuf `CosmosTx` with gogoproto (no unknown-field check) and Go's `GetMsgV1Signers` reads the re-marshaled message; Rust checked the re-marshaled bytes but read the signers from the message as received with prost, which fails on encodings gogoproto accepts (a `MsgSend` coin denom whose length prefix is a 10-byte varint with high bits): Rust failed the whole ICA tx Go executes (different acknowledgement, balances, a fee grant stored on Go only) | state split from one ICA packet (any controller of an interchain account on a counterparty chain) | `host::msg_v1_signers` reads both from `gogo_repack`; the `message.sender` event attribute too (`tx::sender_of`); found by `DELIVER_WIRE=1 DELIVER_IBC=1 fuzz-diff deliver ibc` (app-hash split at the first mutated ICA packet) |
| 67 | messages read as received outside the router (both versions) | Go's `ValidateBlobTx`, `validatePayForFibreTxShape` and ibc-go's `RedundantRelayDecorator` read `GetMsgs()` (the gogoproto structs); Rust decoded the bytes as received with prost, which differs where the tx decoder's unknown-field check does not look: packed scalars (10-byte varints with high bits, an element read past its run), repeated `stdtime` (gogoproto overwrites: a `PaymentPromise.creation_timestamp` of `{nanos: 5}` then `{seconds: -62135596800}` is zero in Go, not in prost) and repeated oneof messages (gogoproto replaces). The blob case is unreachable (the `dynamicpb` `GetSigners` check rejects such packed runs first); the fibre shape check and `MsgUpdateClient`'s header time in the relay decorator gave other `CheckTx` codes | `CheckTx` code split (mempool); defense in depth | `Tx::gogo_msgs` (the normalized messages) in `validate_blob_tx`, `fibre_tx::validate_shape`, `redundant_relay`; `proposal::tests::blob_tx_validation_reads_the_gogoproto_message` |
| 68 | v9 msg service router (app version 9) | celestia-app v9.0.8 (no `fibre` tag, hyperlane-cosmos v1.1.0) registers no msg service for the types v10 added (fibre, valaddr, `MsgCreateNativeSyntheticToken`). The type URLs are not decodable at v9, but authz `MsgGrant` checks a `GenericAuthorization`'s `msg` string with `HandlerByTypeURL`: Go fails it (`ErrInvalidType` "/celestia.fibre.v1.MsgDepositToEscrow doesn't exist."), Rust's router (v10's handlers at every version) stored the grant | app-hash split from one `MsgGrant` anyone can send on mainnet today (app version 9) | `router::has_handler` is false for `txdecode::added_in_v10` types below app version 10; `router::tests::v10_types_have_no_handler_at_app_version_9`; found by `DELIVER_UPGRADE` (its v9 setup grants v10-only messages) |
| 69 | staking / slashing `Params.Validate` text (both versions) | the errors are plain `fmt.Errorf`s with the value (`max validators must be positive: 0`, `unbonding time must be positive: <ns>`, `downtime jail duration must be positive: <Duration>`, the commission bounds with the decimal); Rust dropped the values. A passed gov proposal whose `MsgUpdateParams` fails stores the error as the proposal's `failed_reason` | app-hash split at the end of a passed proposal's voting period | Go's texts in `x-staking` `Params::validate` and `x-slashing`; `types::tests::params_validate_errors_match_go` |
| 70 | the binary switch (`Commit` of the upgrade height) | the multiplexer stops v9.0.8 after that `Commit` and opens v10.4.0, whose `Init` sets the check state to `Header{ChainID}` (height 0, Go's zero time) until the next commit: `CheckTx` verifies signatures with account number 0 and an infinite gas meter (`gas_wanted` -1), vesting balances are all locked, periodic allowances reset from year 1; Rust kept the last block's header. In the same window (and in any historical query) the stores the upgrade adds have no version: `CacheMultiStoreWithVersion` leaves a nil store and any access panics (`runtime error: invalid memory address or nil pointer dereference`, `ErrPanic`); Rust read them as empty. The fresh-process check header also used 1970 instead of Go's zero time | `CheckTx` / query split between the upgrade's `Commit` and the next block (mempool, RPC); not consensus | `App::switch_on_commit` and `init_check_header` (Go's zero time, also on open); `sdk::MultiStore::view_at` nil stores (`StoreError::NilStore` → recovered panic); `replay_scenarios::v9_upgrade_switch_state` |
| 71 | hyperlane routers stored at app version 9, read at 10 | v1.1.0 stores `RemoteRouter.receiver_contract` as any non-empty string; v1.3.0 reads it as the `HexAddress` customtype (`Unmarshal`: exactly `0x` + 64 hex digits, canonical lowercase). At v10 Go fails `RemoteRouters` (`failed to retrieve item: invalid hex address length`), `QuoteRemoteTransfer` and the forwarding module's lookups (`collections: encoding error: value decode: ...`) on such a router and returns upper-case ones lower-cased; Rust returned the stored strings (and its forwarding errors lacked Go's `collections` text) | query split for every token with a v9-shaped router after the upgrade; tx logs | `x_warp::decode_router` / `enrolled_router` by app version (query contexts read it from `sdk::version`); `x_warp::tests::v9_routers_read_at_app_version_10` |
| 72 | `GetModuleAccount` of an address holding another account | Go panics (`account is not a module account`); Rust returned `ErrLogic`. Every module address is blocked except gov's (created at genesis), but v9 does not know fibre's: a send or a vesting account to it at app version 9 leaves an account there, and between the switch and the v10 handler (which replaces it) the v10 app's `ModuleAccounts` / `ModuleAccountByName` queries panic | query split at the upgrade height; the code split it would give in a tx is unreachable (the handler runs first) | `x_auth::get_module_account` returns Go's panic; `tests::module_address_holding_a_base_account_panics`; the `DELIVER_UPGRADE` driver funds the fibre address at v9 |
| 73 | IBC 07-tendermint light client (`MsgUpdateClient`, misbehaviour; both versions) | celestia-core's `CommitSig.FromProto` converts the protobuf `int32` `BlockIdFlag` with `BlockIDFlag(...)`, a `byte`: only the low 8 bits count, so a signature with flag `258` or `-254` is a `Commit` signature (verified and tallied) and `257` is `Absent`; the flag is not in the signed vote, so anyone relaying a valid header can rewrite it. Rust's port compared the `int32` (`unknown BlockIDFlag`) | Go accepts the client update, Rust fails the tx: app-hash divergence from one relayed header (found by `DELIVER_IBC` once its mock counterparty sent such flags: CheckTx code split, then an app-hash split) | `lightclients::light::flag` truncates like Go; `light::tests::block_id_flag_is_truncated_to_a_byte_like_go`; the mock counterparty's every third `MsgUpdateClient` carries a wide flag (`update_client_wide_flag`) |
| 74 | PFM `userRecoverableAccount` (both versions) | the fallback decodes the in-flight packet's original sender with `bech32.DecodeAndConvert` (any prefix, at most 1023 characters); Rust used the `bech32` crate, which rejects prefixes longer than 83 characters (BIP-173) and has no total limit. The original sender is the counterparty user's choice | a forward whose refund falls back to the original sender moves funds in Go and fails in Rust (or the reverse above 1023 characters): app-hash divergence | `encoding::bech32::decode_and_convert` there (and for the account / validator codecs, `x_auth::parse_bech32`, hyperlane's `AccAddressFromBech32`, the event sender); `middleware::tests::user_recoverable_account_falls_back_like_go` |
| 75 | Go panics in message handlers that Rust handled as values (both versions) | A Go panic unwinds to the nearest `recover` (runTx, gov's `safeExecuteHandler`, distribution's `addDecCoins` / deposit guard, celestia's `executeTxMsgs`); Rust returns it as an `SdkError`, which a caller can rebuild into another error. PFM's refund on an error acknowledgement mints the voucher back (`MintCoins` -> `Coin.Add` overflow panics in Go): Rust's PFM wrapped it as `cannot mint coins ...` (`undefined`/1) where Go fails the tx with `ErrPanic` (111222); through the redundant relay ante check the same in CheckTx | `LastResultsHash` split from one acknowledgement relayed for a forwarded packet of a near-max-supply IBC denom | `kvstore::unwind` + `sdk::error::recover`: inside a recover scope the first `go_panic` raised is the outcome, whatever the callers did with it, and gas stops at it (Go's meter is not touched after the panic); scopes at runTx's message loop, the ante chain (a panic there becomes `Fail::Panic`, Go's gas accounting), gov's proposal execution, both distribution recovers and `executeTxMsgs`; `sdk::error::tests::recover_*`, `kvstore::unwind::tests`; `ibc-edge` seed 3000 in `crates/fuzz-diff/deliver-regressions.txt`. Found by the nightly `fuzz-exec` rotation (`deliver ibc-edge`, `DELIVER_IBC` / `WIRE` / `SWEEP`) |
| 76 | gov `MsgVoteWeighted` error text (both versions) | `WeightedVoteOption.String()` is gogoproto's compact text: a string is quoted byte by byte with octal escapes; Rust wrote `{:?}` of the Go form (`"\u{10ffff}\u{10fffe}"` for the bytes `ff fe`). A proposal whose `MsgVoteWeighted` fails stores the text in `failed_reason` | app-hash split from a passed proposal whose vote message fails on execution (error texts of every handler a proposal can run are state) | `x_ante::gogotext::quote`; `gov::keeper::tests::weighted_option_string_quotes_like_gogoproto`; `v9-upgrade` seed 3003 in `deliver-regressions.txt` |
| 77 | the binary switch: memory stores (mempool only) | the multiplexer's next binary starts with empty memory stores: until its first BeginBlock (`x/capability` `InitMemStore`) CheckTx finds no capability, so comet's recheck after the switch drops IBC packet txs with `capability not found`; Rust (one process) still had them | mempool contents at the upgrade height (no consensus effect) | `App::commit` clears the memory stores at the switch (`MultiStore::clear_memory_stores`); `v9-upgrade` seed 3007 in `deliver-regressions.txt` |
| 78 | legacy amino JSON (`TxEncodeAmino` / `TxDecodeAmino` queries) | Go 1.26's `Time.UnmarshalJSON` runs `time.Parse(RFC3339)` (strict checks disabled): one- or two-digit hours, any skipped fraction digits, and `ParseError` texts naming the failing element with `time.quote` (`\xNN`); Rust required fixed positions and always reported `as "2006"`. Also `%#U` of an invalid hex byte (Latin-1 code point, char only if printable), `strconv.Quote` escapes of non-printable runes, the unsigned string in the dec precision error, and invalid bytes replaced per byte (Go's JSON) | query responses only | ported in `legacy-amino` (`time::go_parse_rfc3339`, `go_lossy`); 8 cases in `tests/regressions.jsonl`. Open (non-blocking nightly step): go-amino JSON syntax errors raised on JSON it builds internally, two Go `reflect.Set` panics and one input Go accepts (`282816a91a00`) |
| 79 | header hash and `ValidateBasic` with a chain id that is not UTF-8 (celestia-core) | Go hashes the header's `ChainID` string as its bytes (`cdcEncode`) and checks its byte length; Rust's Go-form string (gostr) was hashed and measured as its UTF-8 re-encoding | a header (in a light block, a light-client-attack evidence's conflicting block, a peer's proposal) with such a chain id hashed differently: evidence or block accepted by one side only | `ctypes::header` encodes `gostr::to_go_bytes` and measures `gostr::go_len`. Found by the F1.7 consensus differential (`c_header`) |
| 80 | repeated `Timestamp` / `Duration` fields (every celestia-core one is gogoproto `stdtime` / `stdduration`) | gogoproto unmarshals each occurrence into a fresh value, so the last occurrence wins whole; prost merged occurrences (a `nanos` of the first survived a second without one). A vote, proposal or commit signature whose timestamp is written twice decodes to another time, hence other sign bytes | a vote or `LastCommit` signature Go verifies and Rust rejects (or the reverse); a block whose `LastCommit` re-encodes differently: Rust validators reject a block Go accepts (halt / fork of Rust nodes) from one crafted message | `prost-gostr-std` (celestia-core-proto's `prost`): `encoding::message::merge` replaces a `Timestamp` / `Duration` per occurrence; `celestia_core_proto::tests::repeated_timestamp_takes_the_last_occurrence`. F1.7 (`c_vote`, `c_commit`) |
| 81 | out-of-range timestamps and durations (celestia-core) | `StdTimeUnmarshal` / `StdDurationUnmarshal` reject a `Timestamp` before year 1 or from year 10000 on, nanos outside [0, 1e9), and durations outside the protobuf range or `time.Duration`; prost accepted them | a vote, proposal, header or evidence Go fails to decode was valid for Rust | validated in `prost-gostr-std` (`validate_timestamp`, `validate_duration`). F1.7 (`c_vote`, `c_header`) |
| 82 | unknown fields with group wire types (celestia-core) | the gogoproto generated `skip*` closes the innermost open group at any end-group tag (its field number is not checked; field number 0 is skipped too); prost requires the matching field number and rejects the message | Rust nodes failed to decode a block part, vote, proposal or evidence Go accepts: a crafted message from one peer (a byzantine proposer's block) halts Rust validators | `prost-gostr-std::encoding::skip_field` follows gogoproto. F1.7 (`c_block`, `c_vote`) |
| 83 | varints, keys and lengths (celestia-core) | gogoproto reads up to ten varint bytes and drops the bits past 64 (prost rejects such a varint), takes a key's field number as `int32(key >> 3)` (a key past `u32` is a valid, possibly known, field in Go; prost rejects it) and a length the same way | as 82: a message Go accepts fails to decode in Rust | `prost-gostr-std`: `gogo_varint` for every varint scalar, packed runs, `bytes` / `string` lengths and message lengths; `celestia_core_proto::gogo_decode` (Go's `Unmarshal` loop) at every place untrusted celestia-core bytes are decoded (p2p packets and handshakes, consensus / block-sync / propagation / mempool / evidence / state-sync messages, blocks, the WAL, the block and state stores, the light store); `tests::ten_byte_varints_decode_like_gogoproto`, `keys_and_lengths_decode_like_gogoproto`. F1.7 |
| 84 | repeated message fields (celestia-core) | the fixes of 80-83 applied only to singular message fields: repeated ones (validator sets, commit signatures, evidence lists) went through prost's own decoder | as 80-83 inside every repeated field | `prost-gostr-std::encoding::message::merge_repeated`. F1.7 (`c_valset`, `c_commit`) |
| 85 | oneof fields (celestia-core and the SDK, IBC and celestia protos) | gogoproto's generated `Unmarshal` replaces a oneof with a fresh value at each occurrence of one of its fields; prost merges an occurrence into a value of the same variant. Celestia-core: `Evidence` (`DuplicateVoteEvidence` / `LightClientAttackEvidence`), consensus / p2p / block-sync / mempool / state-sync / propagation message envelopes, public keys. SDK side: `ModeInfo` (tx signing mode), `StakeAuthorization`'s validator lists, IBC `Acknowledgement` | a block whose evidence list repeats the evidence field decodes to other evidence in Rust (Go: `one or both of the votes are empty`): accepted by Rust and rejected by Go, or the reverse, a consensus split from one byzantine proposer; for txs, a `ModeInfo` written twice is another signing mode | `prost_gostr::GogoOneof` / `gogo_oneof!`: `cargo xtask gen-proto` routes every oneof field through it (both generated crates); `celestia_core_proto::tests::oneof_occurrences_replace_like_gogoproto`. F1.7 (`c_evidence`) |
| 86 | `Data.hash` present but empty in a block (celestia-core) | `BlockFromProto` keeps an empty `Data.hash` (so `Data.Hash()` returns it and `ValidateBasic` compares it with the header's `DataHash`); Rust detected the field with a hand-written scan of the encoded block that did not decode like gogoproto (a second `Data` occurrence, which merges into the first, groups, wide keys) | Rust accepted a block whose `Data` repeats with an empty `hash` that Go rejects (`wrong Header.DataHash`): a consensus split from one proposal | `celestia_core_proto::block_data_hash_present` decodes a shadow message (`Data.hash` as an optional field) with gogoproto's rules; `tests::block_data_hash_presence`. F1.7 (`c_block`, found by a long run after 80-85 were fixed) |
| 87 | IBC packet events (`packet_data`, `packet_ack`, transfer's ack `success`; both versions) | Go sets the attributes to `string(bytes)`, the bytes as they are; Rust converted them with `String::from_utf8_lossy` (U+FFFD for invalid bytes) and, for a valid U+10FFFF, kept a char that the Go-form encoding writes as other bytes | events only (indexer, RPC, relayers reading `packet_data`); not consensus | `gostr::from_go_bytes` / `from_go_vec`; `channel::tests::packet_data_attribute_keeps_the_bytes`. Found by `exec` in `strings` mode (`ibc`, `v9-ibc`, seed 3) during the F1.3 coverage run |
| 88 | propagation `WantParts` bit arrays (celestia-core) | `WantPartsFromProto` builds the array with `NewBitArray(w.Parts.Size())`, the proto's *encoded length*, then `FromProto` takes the proto's `bits` and its `elems` only when there are any: a request with `bits` and no `elems` passes `ValidateBasic` (zeroed elements), an all-default one is `nil parts`. Rust required `elems` to match `bits` | Rust dropped a peer (and its request) that Go serves; peer behaviour only | `WantParts::from_proto` follows Go; `types::tests::want_parts_bit_array_like_go`. Found by the F1.7 `c_prop` op. Rust's array cannot keep bits past `bits` in the last element (Go keeps and re-encodes them; `GetTrueIndices` ignores them) |
| 89 | IBC proofs (`MsgRecvPacket`, `MsgAcknowledgement`, `MsgTimeout`, connection / channel handshakes, client upgrades, solo-machine proofs; both versions) | the keepers decode the proof bytes with gogoproto (`cdc.Unmarshal` into `MerkleProof`, whose `CommitmentProof`s are gogoproto too); Rust decoded them with prost into the `ics23` crate's types. A `CommitmentProof` whose `exist` (oneof) field occurs twice is the last occurrence in Go and the merge of both in Rust (the `path` steps concatenated): another root. Seen both ways, e.g. a valid proof with its `exist` repeated verifies in Go and fails in Rust | **any relayer** turns a valid packet proof into one that Go accepts and Rust rejects (or the reverse): the tx result, then the app hash, splits Rust validators from Go ones | `x_ibc::gogo_decode` (txdecode's schema-driven gogoproto port, then prost on its canonical re-encoding) for every proof the keepers decode; five inputs pinned in `tests/regressions.jsonl`. Found by the F1.7 `c_ics23` op (ibc-go v8.7.2 `MerkleProof.Verify(Non)Membership` with the SDK specs) on its first run |
| 90 | `validateBlock` height check (celestia-core) | `state.LastBlockHeight + 1` wraps in Go; Rust's checked arithmetic panicked on a state at `MaxInt64` (also `height - 1` for the last commit) | none reachable (the state is the node's own); a panic instead of an error | `wrapping_add` / `wrapping_sub` in `nodestore::execution::validate_block`; pinned in `tests/regressions.jsonl`. Found by the F1.7 `c_vblock` op (`validateBlock` against a state built from consecutive real blocks) |
| 91 | IBC 07-tendermint `VerifyMembership` / `VerifyNonMembership` error text (both versions) | ibc-go formats `client state height < proof height (%d < %d)` with `%d`, for which `fmt` does not call `Height.String()`: the heights print as structs (`{1 20} < {720 1}`). Rust printed `1-20 < 720-1`. The error is only a log for a tx, but a passed gov proposal whose message fails (here a `MsgChannelCloseConfirm` with a proof above the client's height) stores it as the proposal's `failed_reason` | app-hash split at the end of the proposal's voting period (any proposer of a proposal carrying an IBC message that verifies a proof) | `height::go_fmt_d` in `lightclients::tendermint::verify_membership` (and the `TrustedHeight` header check, which already printed the struct form); `height::tests::compare_and_parse`; `v9-upgrade` seed 3334 pinned in `crates/fuzz-diff/deliver-regressions.txt`. Found by the nightly `fuzz-exec` `upgrade-0of2` shard (`upgrade-s3334-IBC+HL+SWEEP+RECHECK`, celestia-rs#59) |
| 92 | a tx decoding panic in `PrepareProposal` (both versions) | the tx decoder panics in `UnpackInterfaces` (a solo machine `ClientState` without a consensus state: `cs.ConsensusState.UnpackInterfaces` through a nil pointer), and `separateTxs` / `Fill` decode without a recover, so the whole handler panics and `BaseApp` returns the request's txs unchanged (no square size, no data root); Rust skipped the tx like one that fails to decode | a proposal Go builds (and rejects in its own ProcessProposal) differs from Rust's; proposal content and liveness, not the committed state | `proposal::fill_decode` turns a decoder panic into the handler's abort at every decode in `fill` / `fill_v9`; `proposal::tests::decoder_panic_aborts_fill`; `ibc-upgrades` seed 3059 in `deliver-regressions.txt`. Found by the nightly `fuzz-exec` (`deliver-1of6`, 14 runs) ([#67](https://github.com/celestiaorg/celestia-rs/issues/67)) |
| 93 | solo machine public keys (both versions) | Go keeps an `Any`'s `Value` as received next to its cached value: `Initialize` compares the client's consensus state with the message's by `reflect.DeepEqual` (raw bytes included), and the client state is stored and signed (`HeaderData`) with those bytes. Rust's tx decoder replaced each nested `Any`'s value by the re-encoded cached message, so a key whose sub-key carries an unknown field compared equal and was stored without it | `MsgCreateClient` that Go fails (`consensus state in initial client does not equal initial consensus state`) succeeded in Rust: tx result and app hash split, from any sender | the decoder keeps the `Any`s of `solomachine.v3.ConsensusState` / `Header` as received (`gogo::keeps_raw_anys`); the light client reads the key's gogoproto decoding (`solomachine::cached`); `txdecode/tests/solomachine.rs`; `ibc-upgrades` seed 3080 in `deliver-regressions.txt`. Found by the nightly `fuzz-exec` (`deliver-1of6`, 4 runs, 2 app-hash splits) ([#65](https://github.com/celestiaorg/celestia-rs/issues/65)) |
| 94 | the SDK message count (`countExecutableMsgs`, CheckTx and both proposal handlers) | Go counts the decoded messages (`GetMsgs`); Rust decoded each message's bytes as received with prost, which rejects what gogoproto reads (a key past `u32`, see 83), and counted such a `MsgRecvPacket` as one message instead of reading its ICA packet (600 when its channel cannot be read) | a tx over the limit passed CheckTx in Rust; a block over the limit was accepted by Rust and rejected by Go in ProcessProposal (a split on block validity from one proposer) | `count_executable_msgs` takes `Tx::gogo_msgs`, and a `MsgExec`'s inner messages are counted from their gogoproto decoding; `msg_count::tests::counts_the_gogoproto_decoding`; `ibc-edge` seed 3020 in `deliver-regressions.txt`. Found by the nightly `fuzz-exec` (`deliver-0of6`, `deliver-1of6`, 5 runs) ([#66](https://github.com/celestiaorg/celestia-rs/issues/66)) |
| 95 | `SetPubKeyDecorator` on an interchain account | ICS-27 `InterchainAccount.SetPubKey` always fails (`cannot set public key for interchain account`); Rust set the key. Reachable only in simulation (otherwise no key matches the account's address) | `Simulate` of a tx signed by an interchain account succeeded in Rust and failed in Go (gas estimates; not consensus) | `sigverify::set_pub_key` fails for `Account::Interchain`. Found by the nightly `fuzz-exec` (`DELIVER_SIM`, 18 runs across the deliver shards) ([#68](https://github.com/celestiaorg/celestia-rs/issues/68)) |
| 96 | ante handler and wire-format limits (both versions) | the decoders after the tx decoder still read tx bytes with prost where Go holds gogoproto's decoding (the 83 gap, SDK side): `ParamFilterDecorator` / `NestedMsgDecorator` unpacked `MsgExec` / `MsgSubmitProposal`, and `GetPubKeys` / `SetPubKeyDecorator` the signer keys (`LegacyAminoPubKey` included) with prost (the message count's decoding is 94), which rejects a key past `u32::MAX` (Go: `int32(key >> 3)`, an unknown or a known field); `decodeMultisignatures` compared the whole key instead of `int32(key >> 3)`; `countProtoMsgs` (ICA `CosmosTx`) and `dynamicpb`'s group skipping limited field numbers to 2^29 - 1 where protowire's `ConsumeTag` allows 2^31 - 1. Separately, `SetPubKeyDecorator` stored a multisig key re-encoded by prost, dropping the unknown fields gogoproto's `Any` keeps in its sub-keys | a `MsgExec` holding a key past `u32::MAX` and no `msgs`: Go fails `must include at least one message` (code 18), Rust failed decoding (code 2); the tx result, and with it `LastResultsHash`, splits Rust validators from Go ones from one tx (issue #60); the other paths: a tx or block Go accepts that Rust rejects (or the reverse), and another stored account key | `txview::unpack_go` and `PubKey::from_any` read through `txdecode::prost_compatible` (a multisig keeps its sub-keys as received, `gogo::keeps_raw_anys`, and is stored as gogoproto's `Marshal`), `int32` field numbers in `decode_multisignatures`, `ConsumeTag`'s limit in `count_proto_msgs` and `signers::skip_group`; `txview::tests::issue_60_msg_exec` (the tx's `MsgExec`), `nested_msgs_are_read_as_gogoproto_decodes_them`, `multisignature_keys_truncate_to_int32`, `pubkey::tests`, `msg_count::tests::proto_and_json_counts`, `signers::tests::group_inner_fields_up_to_max_int32`. Found by `deliver-diff` (`ibc`, `DELIVER_WIRE`, seed 3022) |
| 97 | gov `MsgUpdateParams` (`Params.ValidateBasic`, both versions: cosmos-sdk v0.52.8 and v0.52.12 are identical) | Rust's texts dropped Go's values and merged checks (`voting period must be positive` for `voting period must be positive: 0s`, `invalid quorum` for `quorom cannot be negative: %s` / `quorom too large: %s`, ...) and the initial-deposit-ratio and cancel-ratio parse errors had other prefixes. The error is a log for a tx, but a passed gov proposal whose `MsgUpdateParams` fails stores it as the proposal's `failed_reason` | app-hash split at the end of the proposal's voting period (needs a passed proposal with invalid gov params) | `x_gov::msgs::validate_params` uses Go's texts (durations in Go's `String()`, decs in `LegacyDec.String()`); `msgs::tests::validate_params_errors_are_go_s`; `v9-gov` seed 4018 pinned in `crates/fuzz-diff/deliver-regressions.txt`. Go formats `maximum deposit period must be positive: %d` with a `*time.Duration`, which prints the pointer's address (see Open); Rust prints the nanoseconds. Found by the nightly `fuzz-exec` (`deliver-3of6`) ([#71](https://github.com/celestiaorg/celestia-rs/issues/71)) |
| 98 | IBC connection version negotiation error text (both versions) | ibc-go v8.7.2 `PickVersion` returns `failed to find a matching counterparty version (%v) from the supported version list (%v)` of two `[]*Version`, each element printed as gogoproto's `CompactTextString` (`[identifier:"1" features:"ORDER_ORDERED" ]`), and `ConnOpenAck` `the counterparty selected version %s is not supported by versions selected on INIT`; Rust omitted the versions. The error is a log for a tx, but a passed gov proposal whose `MsgConnectionOpenTry` / `MsgConnectionOpenAck` fails stores it as the proposal's `failed_reason` | app-hash split at the end of the proposal's voting period (any proposer of a proposal carrying a connection handshake message) | `connection::go_version_string` / `go_versions_string` in `pick_version` and `conn_open_ack`; `connection::tests::pick_version_error_is_go_s`; `zkism-forwarding` seed 5083 pinned in `crates/fuzz-diff/deliver-regressions.txt`. Found by the nightly `fuzz-exec` (`deliver-0of6`) ([#72](https://github.com/celestiaorg/celestia-rs/issues/72)) |
| 99 | staking `BeginRedelegation` from an unbonded validator (both versions: cosmos-sdk v0.52.8 and v0.52.12) | when the redelegated shares are the source validator's last, `Unbond` removes the unbonded validator (`RemoveValidator`); `getBeginInfo` then looks it up again and, on `ErrNoValidatorFound`, returns early with its named `err` still set, so the redelegation fails with `validator does not exist` (code 3) and the tx reverts. Rust treated the missing validator as bonded (a redelegation entry maturing after the unbonding period) and succeeded | **any delegator** holding all remaining shares of an unbonded validator (e.g. the operator of a jailed or out-of-set validator with only its self-delegation) redelegates them: the tx result, and with it `LastResultsHash` and the app hash, splits Rust validators from Go ones | `delegation::get_begin_info` fails with `no_validator_found`; `tests::redelegating_last_shares_of_unbonded_validator_fails`; `staking-lifecycle` seed 5017 pinned in `crates/fuzz-diff/deliver-regressions.txt` (seen through `DELIVER_SIM`: the generated tx was never delivered). Found by the nightly `fuzz-exec` (`deliver-1of6`) ([#74](https://github.com/celestiaorg/celestia-rs/issues/74)) |
| 100 | ICS-20 `MsgTransfer.ValidateBasic` length limits (both versions) | `len(msg.Receiver) > 2048` and `len(msg.Memo) > 32768` count Go bytes; Rust used `str::len` of the Go-form string, in which each invalid UTF-8 byte is a multi-byte char, so a receiver of 683 to 2048 invalid bytes failed `recipient address must not exceed 2048 bytes` (code 5) where Go passes the check and fails GetSigners' decoding (`... contains invalid UTF-8`, code 1) | `CheckTx` code, gas and log (RPC); not consensus: both ante handlers fail the tx, so both `ProcessProposal`s reject a block holding it | `gostr::go_len` for both limits in `relay::validate_msg_transfer` (as `x_icahost` does for the owner); `tests::transfer_length_limits_count_go_bytes`; `ibc-edge` seed 6091 pinned in `crates/fuzz-diff/deliver-regressions.txt`. Found by `fuzz-exec` (`deliver-3of6`) ([#78](https://github.com/celestiaorg/celestia-rs/issues/78)) |
| 101 | hyperlane `sdk.NewCoins` panic text (both versions) | Go's `Coins.Validate` prints its single-coin case with `%s` of `coins[0]` (`coin -1utia amount is not positive`) and later coins by denom; the hyperlane module's `Coins::validate` printed the denom for the first coin too. The panic is a log for a tx, but a passed gov proposal whose message panics (here a `MsgPayForGas` with amount `-1`) stores `PANICKED: invalid coin set ...` as the proposal's `failed_reason` | app-hash split at the end of the proposal's voting period (any proposer of a proposal carrying such a hyperlane message) | `x_hyperlane::coins::Coins::validate` prints the first coin whole (as `sdk::coins::raw_coins_validate` already does); `coins::tests::validate_texts_are_go_s`; `v9-upgrade` seed 7431 pinned in `crates/fuzz-diff/deliver-regressions.txt`. Found by `fuzz-exec` (`upgrade-1of2`) ([#79](https://github.com/celestiaorg/celestia-rs/issues/79)) |
| 102 | authz `StakeAuthorization.MsgTypeURL` panic text (both versions) | an unknown `authorization_type` panics with `normalizeAuthzType`'s `errorsmod.Wrapf(ErrUnknownAuthorizationType, "cannot normalize authz type with %T", ...)`; gov's `PANICKED: %v` prints it with the `cosmossdk.io/errors` frame of the wrap (`[celestiaorg/cosmos-sdk@v0.52.8/x/staking/types/authz.go:193]`, v0.52.12 at app version 10). Rust printed `cannot normalize authz type: unknown authorization type` | app-hash split at the end of the voting period of a passed proposal whose `MsgGrant` carries such an authorization | `stake_msg_type` uses Go's text and `sdk::version::cosmos_sdk_frame` (shared with `acc_from_bech32_go_v`); `authorization::tests::unknown_stake_type_panics_with_go_s_frame`; `v9-upgrade` seeds 8513 (app version 9) and 8142 (10) pinned in `crates/fuzz-diff/deliver-regressions.txt`. Found by `fuzz-exec` (`upgrade-1of2`, `upgrade-0of2`) ([#80](https://github.com/celestiaorg/celestia-rs/issues/80)) |
| 103 | `x_ibc::gojson::escape_string` (ICS-20 `GetBytes`, every IBC JSON Rust emits as Go's `encoding/json`; both versions) | Go 1.22+ `encoding/json` writes U+0008 and U+000C as `\b` and `\f`; Rust wrote `\u0008` / `\u000c`. `GetBytes` (`MustSortJSON` re-encodes with `encoding/json`) is the packet data hashed into the packet commitment | **any account** sends a `MsgTransfer` whose receiver, memo, denom or sender holds either byte: the stored packet commitment, and the app hash, split Rust validators from Go ones | the two short escapes in `escape_string` (the hyperlane and amino JSON encoders already had them); `gojson::tests::go_compatible_output`, `tests::packet_data_bytes_use_go_s_short_escapes` (Go 1.26.9 / ibc-go v8.7.2 output). Found by `fuzz-exec` (`exec`, `EXEC_MODE=strings`, `v9-ibc` seed 9016) ([#81](https://github.com/celestiaorg/celestia-rs/issues/81)) |
| 104 | authz `SendAuthorization` / ICS-20 `TransferAuthorization` spend limit (`SpendLimit.SafeSub`, both versions) | `ValidateBasic` only checks `IsAllPositive`, so a grant can store a spend limit out of denom order or with a denom twice; `Coins.safeAdd` panics on an unsorted receiver (`Coins (self) must be sorted`) before the allow list is read, and sums repeated denoms through a map. Rust merged the lists without either: an unsorted limit reached the allow list (code 4 instead of Go's recovered panic, 111222), a repeated denom was paired once | **any granter / grantee pair**: a `MsgExec` through such a grant splits the tx result, `LastResultsHash` and the app hash | `authorization::safe_add` ports `safeAdd` (both panics, per-denom sums, zeros dropped, sorted); `authorization::tests::spend_limit_safe_sub_like_go`; `v9-upgrade` seed 10498 pinned in `crates/fuzz-diff/deliver-regressions.txt`. Found by `fuzz-exec` (`upgrade-0of2`) ([#85](https://github.com/celestiaorg/celestia-rs/issues/85)) |
| 105 | staking, slashing and distribution `MsgUpdateParams` (both versions) | gogoproto decodes an omitted LegacyDec as nil and Go rejects nil minimum commission, minimum signing / slash fractions and community tax. Rust's math type does not model nil and decoded each omission as zero, which is valid | a passed governance proposal writes zero parameters and succeeds in Rust while Go records a failed proposal: app-hash split | the handlers retain the raw wire field through validation (`validate_wire` / `validate_basic_wire`), distinguish omission from explicit zero and preserve Go's validation order; focused nil/zero/order tests and `DELIVER_GOVPASS` generation for all three modules. Found by independent v10.5.0 source review |
| 106 | staking `MsgCreateValidator` unsupported consensus-key error (both versions) | Go's `%s` formatting of `[]string` prints `[ed25519]`; Rust Debug printed `["ed25519"]` | a passed governance proposal carrying a create-validator message that reaches this rejection stores a different `failed_reason`: app-hash split | join allowed key types with spaces inside brackets, matching Go. Found by independent v10.5.0 source review |
| 107 | IBC Tendermint client update, misbehaviour, upgrade and recovery errors (both versions) | Rust omitted Go's trusted validator-set text and expected/actual hashes, the missing consensus state's trusted height, elapsed/trusting-period nanoseconds, concrete expected/actual client types and upgrade proof path. The handlers otherwise rejected the same inputs | a passed governance proposal carrying `MsgUpdateClient`, `MsgUpgradeClient` or `MsgRecoverClient` records another `failed_reason`: app-hash split at the end of the voting period | preserve gogoproto `ValidatorSet.String`, upper-hex hashes, height/duration values, Go concrete type names and space-separated Merkle paths; focused recovery, trusted-state, validator-text/type and path tests. Found by independent v10.5.0 / ibc-go v8.7.2 source review |
| 108 | IBC channel opening against a connection version that does not support the requested ordering (both versions) | ibc-go reports `connection version %s does not support channel ordering: %s` with gogoproto's compact version text; Rust omitted the version. The malformed/imported-state branch for a connection with zero or multiple versions also omitted Go's `got: %v` version list | a passed governance proposal carrying `MsgChannelOpenInit` or `MsgChannelOpenTry` records another `failed_reason`: app-hash split at the end of the voting period | use the connection module's gogoproto version/list formatter in the shared channel handshake check; focused supported-feature and version-count tests. Found by independent v10.5.0 / ibc-go v8.7.2 source review |
| 109 | IBC `ChanUpgradeTimeout` already-upgraded predicate (both versions) | ibc-go v8.7.2 compares the proposed local connection hop with that connection's counterparty ID; Rust compared the relayed counterparty channel's hop with the counterparty ID. With different local/remote IDs, equal proposed version/order and an OPEN counterparty channel, Go proceeds to proof verification and timeout restoration while Rust returns `counterparty channel is already upgraded` | a valid channel-upgrade timeout succeeds on one implementation and fails on the other, changing channel/upgrade/error-receipt state and the app hash | match ibc-go's exact operands in `counterparty_already_upgraded`; a focused regression distinguishes proposed local and relayed remote hops. Found by independent ibc-go v8.7.2 channel-upgrade review |
| 110 | IBC channel-upgrade and packet error details (both versions) | `UpgradeFields` is passed by value to Go `%s` and prints as `{ORDER [hops] version}`, while Rust used pointer-style protobuf text; connection versions need pointer compact text. Packet acknowledgement and timeout commitment mismatches omitted calculated/stored byte arrays (whose argument order differs), and ordered timeout errors omitted sequence values, including Go's intentionally unmatched parenthesis in timeout-on-close | passed governance proposals carrying upgrade, acknowledgement or timeout messages can persist another `failed_reason`: app-hash split at the end of the voting period | exact Go value/pointer formatters, decimal byte slices and ordered sequence details; focused formatter and operand tests. Found by independent ibc-go v8.7.2 upgrade/packet review |
| 111 | IBC packet JSON / packet-forward sender Go-string boundary (both versions) | the shared JSON decoder returned ordinary Unicode rather than gostr's Go-string form; a reserved-range scalar could be re-marshalled or persisted as its internal UTF-8 representation. Normalizing the decoder also requires `GetReceiver` to hash the original Go bytes | a foreign ICS-20 sender can produce different forwarded packet/in-flight state and later refund behavior | normalize JSON strings centrally; hash the normalized PFM preimage through `gostr::to_go_bytes`; full decode/remarshal and Go 1.26.9 / packet-forward-middleware v8.2.0 U+10FFFF vectors pin the path |
| 112 | ICA host metadata aliases (both versions) | Rust iterated a sorted JSON map, so snake-case aliases could overwrite camel-case protobuf JSON names; gogoproto jsonpb gives camel-case names field-wise precedence, including null and type errors | a handshake version with both aliases can be accepted on one implementation and rejected on the other, or negotiate/store different metadata | validate unknown fields, then decode each field with camel-case precedence; tests cover order, null, type errors and connection IDs. Found by independent ibc-go v8.7.2 application review |
| 113 | IBC acknowledgement error JSON Go-string boundary (both versions) | a canonical reserved-range Unicode error decoded as ordinary Rust Unicode, so Go-form re-marshalling changed its bytes and Rust's acknowledgement guard rejected it | Go accepts the acknowledgement and executes the application refund while Rust rejects the packet acknowledgement: transaction result and state split | normalize the decoded error to gostr form before constructing `AckResponse::Error`; a canonical U+10FFFF decode/remarshal test pins the bytes |

Open:

- Go itself: `jsonpb` decodes oneof members in map iteration order, which
  Go randomizes, so a proto3json ICA message with two members of one oneof
  (`StakeAuthorization` with both `allow_list` and `deny_list`, an ICS-23
  proof with two variants, `Acknowledgement` with `result` and `error`)
  decodes nondeterministically in Go (a consensus risk of Go nodes among
  themselves). Rust decodes them by name (the last one sorted wins).
  **Reachability on mainnet (F1.11, checked 2026-10-19):** gogoproto
  v1.7.2 `jsonpb` ranges over `sprops.OneofTypes` (a map) and sets each
  member present, so the last one in Go's random order wins. Mainnet's ICA
  host `allow_messages` (queried from two public REST endpoints) holds
  `MsgTransfer`, `MsgSend`, the staking and distribution messages,
  `MsgVote`, `MsgGrantAllowance` and `MsgRevokeAllowance`: none has a oneof
  in its proto3json form (allowances are `Any`s), so the
  `StakeAuthorization` case cannot be sent there. Acknowledgements: core
  `AcknowledgePacket` (ibc-go v8.7.2 `packet.go`) re-marshals a decoded
  `Acknowledgement` and fails unless the bytes are equal, which an ack with
  two members never is, so ICS-20's and PFM's decodes (PFM checks no bytes
  itself) only see single-member acks. ICS-23 proofs are protobuf. No
  path that reaches Go's nondeterminism is known on mainnet today.
  **Policy:** the Rust decoding stays deterministic; if Go validators ever
  diverge among themselves, the Rust validator follows the chain only
  while its own execution matches the committed app hash, and otherwise
  halts (the app-hash-halt response in `MILESTONES.md` C1.5): it never
  signs a block on a state it did not compute. Upstream report: drafted in
  [`docs/upstream/gogoproto-jsonpb-oneof.md`](upstream/gogoproto-jsonpb-oneof.md),
  for the owner to file (re-check the ICA allow list whenever mainnet
  governance changes it).

- Error text (logs; not hashed, code and gas match) that still differs
  from Go, judged infeasible or not worth emulating: Go panics log a
  stack (goroutine ids, addresses); protobuf-go randomizes a space in its
  errors (`proto:` followed by a space or U+00A0, chosen per binary);
  `Simulate`'s error carries `cosmossdk.io/errors`' `%v` source frame
  (`[celestiaorg/cosmos-sdk@v0.52.12/x/auth/ante/sigverify.go:290]`, see
  "Log parity" above); gogoproto's `wrong wireType` names the Go field.
  gRPC
  status messages carry Go forms (tonic cannot send Go's invalid bytes);
  ABCI, REST and RPC carry Go's bytes. Fixed since: gogoproto's text form
  in `failed to execute message; message %v` and gov's
  `PANICKED: %v` (`x_ante::gogotext`), nil amounts printed `<nil>`,
  durations and times in Go's `String()` forms, ICS-23 proof errors,
  the fee-grant error's raw addresses, `unknownproto`'s Go type and wire
  type names (`errUnknownField "*types.MsgForward": {TagNum: 7,
  WireType:"bytes"}`, `txdecode::go_types` from the protos' `go_package`). Logs and events of every devnet
  scenario match Go byte for byte (`KNOWN_EVENT_LOG_GAPS` is empty).
- Go itself (both versions, Rust matches it): x/signal compares versions
  with `ctx.BlockHeader().Version.App`, but `FinalizeBlock` builds the
  header without a version (0). `MsgSignalVersion` therefore accepts any
  version, and `MsgTryUpgrade` with a 5/6 quorum on the current version
  (or a lower one) succeeds instead of failing with
  `ErrInvalidUpgradeVersion`: it stores an upgrade the `EndBlocker` never
  applies (not increasing) and never clears, so every later signal and
  try-upgrade fails with `ErrUpgradePending`. Once 5/6 of the power has
  re-signalled the running version (e.g. automation signalling again
  after an upgrade), anyone's `MsgTryUpgrade` disables upgrades by
  signalling for good. Reproduced by the `DELIVER_UPGRADE` late phase.
- Go itself (both versions): gov `Params.ValidateBasic` returns
  `maximum deposit period must be positive: %d` of a `*time.Duration`;
  `%d` of a pointer to an integer prints the pointer's address, so a
  passed `MsgUpdateParams` proposal with a non-positive
  `max_deposit_period` stores a different `failed_reason` on every Go
  node (an app-hash split among Go validators). Rust prints the
  nanoseconds (`... must be positive: 0`) and, per the policy above,
  halts rather than follow a state it did not compute. Reaching it needs
  governance to pass such a proposal. `deliver-diff` therefore gives a
  synthesized proposal's gov `MsgUpdateParams` a positive period
  (`go_deterministic`; `ibc` seed 8083 found the address in Go's log).
- An event attribute of ibc-go's callbacks (`ibccallbackerror-error`,
  ICS-20 `ValidateBasic`'s `amount must be strictly positive: got %d`)
  prints a `math.Int` by its pointer: `%d` of the struct reaches the
  unexported `*big.Int` field, which `fmt` prints as an address. Go's
  output is nondeterministic (`{53814906916832}` and `{26591903563872}`
  for the same packet in two runs), so Rust prints the value (`got 0`);
  events are not hashed.
- Go itself: `TxDecodeAmino` of a 2 MB `StdTx` with ~20 000 nested
  `MsgExec` grew the Go process past 10 GB (quadratic copying in
  go-amino); the oracle caps amino inputs at 256 KB.

## Re-running the triage

```bash
# Rust (fetch the advisory DB with git if cargo-deny cannot reach GitHub)
cargo deny check advisories
cargo audit

# Dependabot's view (GHSA / OSV data), one manifest at a time
osv-scanner --format json -L Cargo.lock
osv-scanner --format json -L conformance/<module>/go.mod

# Go reachability: govulncheck needs vuln.go.dev; without it, build a local
# database from https://osv-vulnerabilities.storage.googleapis.com/Go/all.zip
# (index/db.json, index/modules.json, ID/<id>.json) and pass -db file://...
cd conformance/<module> && GOFLAGS=-mod=mod govulncheck ./...
cd conformance/<module> && GOFLAGS=-mod=mod govulncheck -scan module

# Bump a Go test tool: `go get <module>@<fixed>` (no `go mod tidy` in the
# pinned oracles), rebuild, and compare the tool's output with the old
# binary's / the committed fixtures (antereplay: replay the cases of
# conformance/fixtures/ante/mainnet-tx-corpus.json.zst; fixturegen: every
# `xtask refresh-fixtures` scenario into a temp dir; fuzzoracle: the same
# request lines to both binaries).
```
