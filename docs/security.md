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
when it has none), and ownership changes are followed.

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

Open:

- Go itself: `jsonpb` decodes oneof members in map iteration order, which
  Go randomizes, so a proto3json ICA message with two members of one oneof
  (`StakeAuthorization` with both `allow_list` and `deny_list`, an ICS-23
  proof with two variants, `Acknowledgement` with `result` and `error`)
  decodes nondeterministically in Go (a consensus risk of Go nodes among
  themselves). Rust decodes them by name (the last one sorted wins).

- Error text (logs; not hashed, code and gas match) that still differs
  from Go, judged infeasible or not worth emulating: Go panics log a
  stack (goroutine ids, addresses); protobuf-go randomizes a space in its errors;
  address-codec errors are worded differently in a few modules. gRPC
  status messages carry Go forms (tonic cannot send Go's invalid bytes);
  ABCI, REST and RPC carry Go's bytes. Fixed since: gogoproto's text form
  in `failed to execute message; message %v` and gov's
  `PANICKED: %v` (`x_ante::gogotext`), nil amounts printed `<nil>`,
  durations and times in Go's `String()` forms, ICS-23 proof errors,
  the fee-grant error's raw addresses, `unknownproto`'s Go type and wire
  type names (`errUnknownField "*types.MsgForward": {TagNum: 7,
  WireType:"bytes"}`, `txdecode::go_types` from the protos' `go_package`). Logs and events of every devnet
  scenario match Go byte for byte (`KNOWN_EVENT_LOG_GAPS` is empty).
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
