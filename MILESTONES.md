# celestia-rs Milestones

Authoritative source for "what is next." Updated when a milestone starts or
completes. Direction and scope decisions: [`docs/adr/0001-project-direction-2026-10.md`](docs/adr/0001-project-direction-2026-10.md).
Blockers needing the owner: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md).

**End state:** a Rust Celestia mainnet validator that is consensus-compatible
with Go celestia-app/celestia-core and runs on a Mac Mini M4 (16 GB / 512 GB).

**Go parity target:** celestia-app **v10.4.0** / celestia-core v0.42.x, and
app version 9 at celestia-app **v9.0.8** (mainnet today; ADR 0004; see open
question 2).

| #   | Status      | Milestone | Acceptance test |
|-----|-------------|-----------|-----------------|
| M0  | completed   | Bootstrap (scaffolding + fixturegen) | `cargo xtask refresh-fixtures --scenario bech32-addresses` produces a valid fixture |
| M1  | completed   | Foundational libraries (hash, encoding, crypto, merkle, NMT, shares, square, …) | `m1-integration::parse_real_mainnet_block_produces_matching_dah` |
| M2  | completed   | State & storage (IAVL, kvstore, multistore, ICS-23) | DB-backed replay shard (`m2-integration`); non-empty historical import by snapshot restore of Go state (ADR 0002) |
| M3  | completed   | Tx & ante machinery | Tx-validation outcome matches Go for 1,062 mainnet txs (the v6 prototype `ante` / `m3-integration` were removed; the corpus runs through the v10 `x-ante`: `x/ante/tests/mainnet_corpus.rs`) |
| M4  | completed   | Core SDK modules (auth, bank, staking) | Superseded by the v10 `x/auth`, `x/bank`, `x/staking` at app-hash parity on every devnet scenario; the v6 prototype crates and their storage fixtures were removed |
| D1  | completed   | Devnet differential oracle: Go `celestia-appd` v10 devnet in-container, exported blocks + per-block app hashes + genesis as fixtures | `cargo xtask devnet-fixtures` regenerates a committed corpus |
| R1  | completed   | Retarget implemented crates from app v6 to v10 (shares/square/ante/modules diffs) | Done by the v10 parity work: go-square v4 builder, `x-ante` (Go v10 vectors), every module at v10 app-hash parity; the remaining v6-era fixtures (hash, NMT, shares, DAH of mainnet blocks, IAVL / multistore) cover algorithms unchanged in v10 |
| S1  | completed   | **Vertical slice:** ABCI++ server + `App` (InitChain/FinalizeBlock/Commit/Info/Query) running behind Go celestia-core | Rust app replays devnet blocks with matching app hash for an empty-tx chain |
| S2  | in progress | Grow the slice until every devnet scenario matches: remaining SDK modules (M5), celestia modules incl. blob/fibre/minfee/signal/mint (M6), IBC (M7), PrepareProposal/ProcessProposal | App hash matches Go for every block of the devnet scenario corpus |
| V9  | in progress | **App version 9** (mainnet's celestia-app v9.0.8) at Go parity inside the same app, gated on the block's app version, and the v9→v10 upgrade exactly as the multiplexer runs it ([ADR 0004](docs/adr/0004-app-version-9.md)) | Every `v9-*` devnet scenario (recorded from Go v9.0.8; `v9-upgrade` switches binaries inside `Commit` like the multiplexer) replays at full parity (`devnet-conformance/tests/replay_scenarios.rs`) |
| S3  | completed   | State sync client + snapshot import (devnet Go↔Rust done); follow Mocha / mainnet — `celestia-rs follow` (HTTPS RPC, light-client verified, Go snapshot bootstrap) follows the Mocha tip and mainnet (app v9) with matching app hashes (P2P full node: OPEN_QUESTIONS #1) | Full node follows the testnet tip with matching app hashes |
| M9  | completed   | Rust consensus engine (Tendermint BFT, celestia-core semantics) — `privval`, `nodestore`, `consensus` (+ reactor on `p2p`), `propagation` (RECOVERY), `celestia-rs start` as a full network participant | Mixed Rust+Go private devnet reaches consensus (`conformance/devnet/mixed-validators.sh`) |
| M10 | completed   | Rust P2P (secret connection, MConn, PEX), mempool, block sync — `p2p`, CAT `mempool`, `blocksync`, `evidence`, `statesync` done; `celestia-rs start` block-syncs / state-syncs from Go and serves Go (`blocksync-interop.sh`); propagation + consensus gossip with Go validators (`mixed-validators.sh`), txs both ways (`mempool-interop.sh`) | Rust node peers with Go nodes and gossips blocks/txs |
| M11 | completed   | RPC & gRPC: ABCI `Query` (store proofs, historical heights, gRPC routing), every query service, gRPC server and REST gateway in `celestia-rs abci` | Responses match Go for a canned request corpus (13 scenarios, 46.8k ABCI + 30.6k REST requests, `devnet-conformance/tests/query_corpus.rs`) |
| M12 | completed   | Node binary, CLI, keys, double-sign guard, pruning — operator CLI at Go parity (`init`, `keys` on Go test/file keyrings, `genesis` add-account/gentx/collect/validate, `tx` (incl. staking create/edit-validator, unbond, redelegate, `--gas auto`, generic `tx <module> <msg> --json`; `--generate-only` identical to Go's, `devnet/cli`)/`query`/`status`, `config`, `rollback`, `export` = Go's export on 15 devnet scenarios), `--network mainnet\|mocha\|arabica\|local`, SIGTERM shutdown, `halt-height`/`halt-time`, upgrade halt with `upgrade-info.json`, pruning + `min-retain-blocks`, Prometheus metrics, ulimit check; `conformance/devnet/operator.sh`, `docs/operator-guide.md` | Release-ready for the Mac Mini: `release` / `dist` profiles, `macos-14` CI job (release build + smoke tests), tag-triggered release workflow (aarch64-apple-darwin, x86_64 / aarch64 Linux + SHA-256), launchd plist / systemd unit (`packaging/`), Mocha + mainnet validator runbooks (`docs/operator-guide.md` §11-12). `start --network mocha` over P2P is the first step of M13 on the Mac (OPEN_QUESTIONS #1) |
| M13 | in progress | Mocha validator soak; mainnet full node. 2026-10-15 on the Mac Mini M4: route A (P2P state sync from Mocha's Go peers) restored snapshot 1,716,000 and reached consensus 47 s after start, outbound-only with 13 peers; `check-apphash.sh` OK against two providers. Validator `celestia-rs validator` bonded at 1,720,896, signing from 1,720,898, under launchd since 1,721,185; the switch-over exposed a delayed-precommit-on-SIGTERM bug (one `conflicting data` refusal, 3 missed blocks), fixed with a sim test | 24 h signing on Mocha with no app-hash mismatch, no double-sign refusal, missed blocks < 1% (owner, 2026-10-16; shortened from 72 h the same day: a longer soak adds little signal); then a mainnet full node on the Mac |
| M14 | pending     | Mainnet validator | Signs mainnet blocks; no double-signs; no evidence |
| F1  | in progress | **Fuzzing**: differential fuzzing of tx decoding, signers, square construction and ProcessProposal against Go v10.4.0 / v9.0.8 (`crates/fuzz-diff`, `conformance/fuzzoracle`), crash-only libFuzzer targets for p2p / ABCI / RPC / light-client input (`fuzz/`); findings fixed with regressions (`docs/security.md`, "Fuzzing") | Nightly `fuzz-diff` and `cargo-fuzz` jobs find no mismatch or crash |
| B1  | later       | Benchmarks vs Go on the Mac Mini M4 | Report committed under `docs/benchmarks/` |

## Current focus

**M13 — Mocha validator on the owner's Mac Mini (next).** Everything that
can be done from the development container is done (M12): the runbook is
`docs/operator-guide.md` §11 ("Running a Mocha validator on a Mac"),
mainnet in §12. Steps on the Mac, in order: (1) install (release archive
or `cargo build --profile dist`), `init --network mocha`, operator key in
the `file` keyring, faucet; (2) bootstrap — route A, P2P state sync from
Mocha peers (never run against Mocha yet: the first real test of the Rust
P2P stack on a public network), else route B, Polkachu snapshot +
`follow` over HTTPS, then `start`; (3) catch up over P2P block sync and
`packaging/check-apphash.sh` OK; (4) `tx staking create-validator`
(commission >= 0.20, Mocha's minimum); (5) soak under launchd with
Prometheus alerts on height, missed blocks and app hash. Report back:
which bootstrap route worked, peers / block sync rate, missed blocks per
day, RSS and disk per day, any `failed signing vote` line. Owner choices:
OPEN_QUESTIONS #4.

**S3 — following public networks over HTTPS (2026-10-15).** P2P is
blocked from the development container, so `celestia-rs follow`
(`docs/operator-guide.md` §10) follows a network over celestia-core RPC:
per height it light-client verifies the header (`light::verify_adjacent`),
checks that the rebuilt block's id (header + part set, so every tx) is the
one 2/3+ of our validator set signed, compares `app_hash`,
`last_results_hash`, validator and consensus-param hashes with what our
own execution produced, runs `ValidateBlock` + `ProcessProposal`, and
executes the block through the `start` block executor; the first
divergence stops it with a diagnostic bundle (per-tx results vs the
network's `/block_results`, every store's root hash from the network's
ICS-23 proofs vs ours). Bootstrap: the Go SDK state-sync snapshot inside
Polkachu's pruned tarballs (`data/snapshots/<h>/3`, streamed out of the
`.tar.lz4`: 167 MB for Mocha, 644 MB for mainnet), restored through
`OfferSnapshot` / `ApplySnapshotChunk` with the light-client-trusted app
hash.

| Network | Range followed with matching app hash, results and validator sets | Notes |
|---|---|---|
| Mocha (`mocha-5`, app v10) | 1,624,501 → tip (1,711,100+ at 15:32 UTC), ~86,600 heights, at the tip since 11:09 UTC | PayForBlobs (~96k), PayForFibre, fibre escrow, bank, staking, authz, feegrant, unjail. No mismatch. Mocha's v10 era has no IBC or hyperlane traffic (hyperlane txs only at v9 heights 799,940–973,254) |
| Mainnet (`celestia`, app v9) | 14,894,001 → 14,914,200+ (still running), ~20,200 heights | PayForBlobs, IBC (update client, recv / ack, transfer), ICA host txs, hyperlane, staking, distribution, authz. Four bugs found and fixed (below) |

Bugs found by following real networks (each with a regression test):
v9 snapshots could not be state-synced (`unknown store: fibre`);
mainnet's last completed upgrade `v9` had no handler at app version 9
(halt at the first block); a redelegation from an unbonded validator
returned the Unix epoch instead of Go's zero time (`LastResultsHash`);
out of gas inside an ICA host tx was turned into an error ack and kept
consuming gas instead of failing the tx at the first exceeding
consumption like Go's panic (`LastResultsHash`). Mainnet headers are also
verified light-client-only (`follow --light-only`, sequential + skipping
to the tip, with witnesses).

Throughput (release build, shared 4-core container under load): 7–14
blocks/s on Mocha (execution-bound; debug build 1.5–2.5), 2–3 blocks/s on
mainnet (heavier blocks; RPC fetch ~1/3 of the time). Open: the Mocha
v9 era and mainnet before 14.89M need a v9 snapshot (or genesis replay);
`LightClientAttackEvidence` JSON decoding; a release-mode long soak at the
mainnet tip.

**V9 (app version 9).** Mainnet runs celestia-app v9.0.8 and Mocha started
at app version 9, so the state machine also executes app-version-9 blocks
(ADR 0004: no fibre / valaddr stores, cosmos-sdk v0.52.8 sequential
`deliverTx` with the block gas meter, v9's decoder registry and lenient
BlobTx / IndexWrapper, `MsgExecDecorator`, multi-denom fee priority,
duplicate-TxRaw-field CheckTx rejection, v9 PrepareProposal /
ProcessProposal) and the v9→v10 upgrade (store loader adds fibre / valaddr
at `H+1`, the v10 handler runs their default genesis). Recorded from Go
v9.0.8 (`APP_V9=1 devnet.sh`, `UPGRADE_TO_V10=1` for the binary switch);
at full parity (app hash, tx results, CheckTx, PrepareProposal /
ProcessProposal): `v9-empty`, `v9-staking`, `v9-gov`, `v9-authz-feegrant`,
`v9-vesting`, `v9-blob`, `v9-hyperlane`, `v9-ibc`, `v9-zkism-forwarding`
(SP1 v5 proofs), `v9-exec` (sequential execution,
multi-denom fees, `MsgExec` of `MsgSubmitProposal`) and `v9-upgrade`
(v9 txs → signal + try-upgrade → binary switch → v10 handler → v10 txs).
Real blocks: square size, data root and tx decoding match for 12 recent
mainnet blocks and 10 Mocha blocks below the v10 height (`real_dah`), and
Mocha replayed from its real genesis (InitChain of the 56 MB mocha-5
genesis, incl. ibc-go `TransferAuthorization` grants, now ported) through
height 40,000 with every app hash and tx result matching (`genesis_replay`).
The legacy amino codec runs v9.0.8's type graph at app version 9
(2026-10-16: schema and 77 vectors dumped from v9.0.8, the v9 query
corpora with no exception left). Open: state sync / snapshots of a v9 chain, and following Mocha from genesis
over HTTPS (with the follower work). Ten v9 query corpora are at parity
(2026-10-16).

**Go strings (ADR 0005, 2026-10-15).** Protobuf `string` fields carry
any bytes, as gogoproto decodes them: Go executes and stores invalid UTF-8
(validator descriptions, proposal titles, vote metadata, memos, grants,
`MsgExec` / gov / ICA inner messages), which the Rust router rejected — a
one-tx network split. Strings now hold Go's bytes losslessly (`gostr`
Go forms, `prost-gostr` codec), with Go's semantics wherever bytes matter
(lengths, store keys, JSON in / out, amino-JSON sign bytes). Evidence: the
`utf8` / `v9-utf8` devnet scenarios (Go executes the txs, incl. two
amino-JSON signed ones; Rust replays at full parity with logs and events
byte for byte), the `utf8` query corpus, and `fuzz-diff exec` (Simulate
differential: ~8 000 mutated txs over 12 scenarios, no code / gas / result
mismatch after fixing what it found: nested-signer checks, gov deposit
validation order and nil amounts, nil commission rates, unrouted bank
messages, proposal message re-packing). Also fixed: v9 `x/warp`
`receiver_contract` as a plain string. ICA host txs with invalid UTF-8
(2026-10-16, `ibc-upgrades` regenerated from Go: a transfer with it in
receiver and memo, a fee allowance holding it, the packet memo, proto3json
payloads) found the per-block message count reading such a proto3json
payload as uncountable (Rust's ProcessProposal would reject a block Go
accepts; security.md finding 27); the replay is at full parity with
CheckTx. Open: error-text-only log
differences (security.md).

**Nil customtypes, logs and events (2026-10-16).** `fuzz-diff exec`
gained two modes: `EXEC_MODE=nil` leaves every `math.Int` / `LegacyDec`
of the scenario's messages nil (dropped or empty, nested ones too) and
`EXEC_MODE=synth` builds every `sdk.Msg` that reaches a math customtype
from scratch. Against Go v10.4.0 and v9.0.8 (12 scenarios, ~15 000
simulated txs) they found Go's nil-pointer panics Rust did not raise
(`MsgEditValidator`'s present-but-empty rates, warp transfers, IGP
payments, reward-pool deposits) and the in-place zeroing of nil `Int`s
by gogoproto's `Marshal` in authz / ICA signer checks (security.md
findings 17–24); after the fixes no code / gas / result mismatch. Logs and
events of every devnet scenario now match Go byte for byte (the
`events_and_logs_parity` ratchet is empty): gogoproto's text form of
messages in authz / gov errors, consensus-param and fibre events, ICS-23
proof errors. ADR 0004's remaining v9 differences are ported (hyperlane
v1.1.0 multisig / IGP / amino names, the distribution overflow panic,
zkism's paginated export, the v9 query surface with ten v9 query corpora
recorded from Go v9.0.8) and, since 2026-10-16, the legacy amino codec's v9 type graph. Light-client-attack
evidence decodes from `/block` JSON (Go vectors with cmtjson and `ABCI()`
forms).

**S2.** S1 is done: the Rust state machine reproduces Go celestia-app v10.4.0
exactly — app hash, every tx result's code/data/gas, validator updates — for
every height of the `empty` (20), `staking` (40), `staking-lifecycle` (60) and
`hyperlane` (60) devnet scenarios (plus `gov`, `authz-feegrant`, `vesting`,
`upgrade` (ends in Go's halt), `consensus` and `evidence` (misbehavior
injected into a re-driven Go app, `inject-evidence.sh`)), and runs live: Go celestia-core over
`celestia-rs abci` produces blocks, and a Rust follower tracks a Go validator
through a staking workload (`conformance/devnet/mixed.sh`).

| Area | State |
|---|---|
| Runtime & tx pipeline | sdk runtime, math (12,446 vectors), txdecode (4,968 vectors), x-ante (4,144 vectors), phased runTx, router, genutil |
| Modules at parity | auth (+ vesting), bank, staking, distribution, slashing, mint, gov (all msgs, tally, execution), upgrade (plans, v10 handler, signalled upgrades, halt), consensus (MsgUpdateParams), authz, feegrant, evidence, params, circuit, blob (msgs), minfee, signal, fibre (all msgs, PFF settlement, ante), valaddr (all msgs), zkism (all msgs, SP1 Groth16, ISM type 42), forwarding (MsgForward), capability, ibc (all msgs: 07-tendermint light client incl. misbehaviour, connection / channel handshakes and close, packets, timeouts, redundant-relay ante), transfer, icahost (handshake, tx execution), pfm (forwarding, refunds, retries), hyperlane + warp (all msgs) |
| At parity (blocks) | PrepareProposal / ProcessProposal (go-square v4 builder, DAH) and CheckTx, replayed for every scenario incl. `devnet/blob`; Rust-proposed blocks accepted by a Go full node (`rust-proposer.sh`) |
| At parity (fibre/zk) | `devnet/fibre` (escrow, withdrawals, PayForFibre with validator signatures and PFF system blobs, unsettleable promises dropped in PrepareProposal, timeouts, gov params update) and `devnet/zkism-forwarding` (celestia-app's SP1 Groth16 vectors verified on chain, zk ISM on the hyperlane router, MsgForward with IGP fees); both also pass `mixed.sh` and `rust-proposer.sh` |
| Deferred | ProcessProposal blob-tx cache (behavior-neutral) |
| Storage (ADR 0002) | Persistent IAVL on redb (lazy nodes, LRU cache, history, pruning), atomic multistore commits, restart/handshake (`celestia-rs abci --home`), SDK snapshots + ABCI state sync; every scenario replays on redb, incl. restart-halfway and snapshot-restore replays; Go↔Rust state sync both ways (`conformance/devnet/statesync.sh`) |
| IBC differential | `devnet/ibc` (two Go chains + cosmos/relayer), `devnet/ibc-edge` and `devnet/ibc-upgrades` (ibc-go testing package, `conformance/ibcvectors`) at full parity incl. CheckTx |
| IBC (closed deferrals) | `devnet/ibc-upgrades` (`conformance/ibcvectors/upgrade_test.go`, incl. CheckTx) at full parity: 06-solomachine (headers, multisig, membership / non-membership proofs over handshakes and packets, misbehaviour, recovery), channel upgrades of the ICA host channel (Try / Confirm / Open with flushing, cancel by receipt and authority, timeout, FAILURE results; transfer is not upgradable behind PFM, like Go), gov-executed `MsgIBCSoftwareUpgrade` (circuit breaker reset, plan + upgraded client / consensus state), ICA `proto3json` txs and `MsgModuleQuerySafe` (gas-metered module queries) |
| Light client & state sync (ADR 0003) | `light` (verifier, bisection, detector, store, RPC provider; Go vectors 720 verifier + 196 client scenarios at parity) and `statesync` (reactor, syncer, state provider); a Rust node state-syncs from Go peers and serves snapshots to Go (`conformance/devnet/statesync-rust.sh`) |
| Queries (M11) | ABCI `Query` (store/ICS-23 proofs, app, p2p, custom inclusion proofs, gRPC routing at historical heights) and every query service incl. hyperlane/warp, zkism, forwarding, IBC, reflection (incl. `cosmos.reflection.v1` FileDescriptors on the empty corpus and the gRPC-only `v2alpha1` service, `reflection-v2alpha1.json` recorded from Go), the legacy amino tx codec (`TxDecodeAmino`/`TxEncodeAmino`: go-amino ported over a type schema dumped from celestia-appd, `conformance/aminoschema`; 77 Go vectors incl. errors and panics, all in the empty corpus), cmtservice/tx status/gas estimation over celestia-core RPC; gRPC server (`--grpc-address`) and REST gateway (`--api-address`) in `celestia-rs abci`; byte-identical to Go on the 13-scenario query corpus |
| Deferred (M11) | The validator-local fibre promise cache, Go build info in `GetNodeInfo` |

M2's disk-backed IAVL and non-empty historical import landed with ADR 0002
(snapshot import of Go state, verified against Go app hashes); S3 now only
needs a public testnet to follow.
