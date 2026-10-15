# celestia-rs Milestones

Authoritative source for "what is next." Updated when a milestone starts or
completes. Direction and scope decisions: [`docs/adr/0001-project-direction-2026-10.md`](docs/adr/0001-project-direction-2026-10.md).
Blockers needing the owner: [`OPEN_QUESTIONS.md`](OPEN_QUESTIONS.md).

**End state:** a Rust Celestia mainnet validator that is consensus-compatible
with Go celestia-app/celestia-core and runs on a Mac Mini M4 (16 GB / 512 GB).

**Go parity target:** celestia-app **v10.4.0** / celestia-core v0.42.x (see
open question 2).

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
| S3  | in progress | State sync client + snapshot import (devnet Go↔Rust done); follow Arabica/Mocha as a full node behind celestia-core | Full node follows the testnet tip with matching app hashes |
| M9  | completed   | Rust consensus engine (Tendermint BFT, celestia-core semantics) — `privval`, `nodestore`, `consensus` (+ reactor on `p2p`), `propagation` (RECOVERY), `celestia-rs start` as a full network participant | Mixed Rust+Go private devnet reaches consensus (`conformance/devnet/mixed-validators.sh`) |
| M10 | completed   | Rust P2P (secret connection, MConn, PEX), mempool, block sync — `p2p`, CAT `mempool`, `blocksync`, `evidence`, `statesync` done; `celestia-rs start` block-syncs / state-syncs from Go and serves Go (`blocksync-interop.sh`); propagation + consensus gossip with Go validators (`mixed-validators.sh`), txs both ways (`mempool-interop.sh`) | Rust node peers with Go nodes and gossips blocks/txs |
| M11 | completed   | RPC & gRPC: ABCI `Query` (store proofs, historical heights, gRPC routing), every query service, gRPC server and REST gateway in `celestia-rs abci` | Responses match Go for a canned request corpus (13 scenarios, 46.8k ABCI + 30.6k REST requests, `devnet-conformance/tests/query_corpus.rs`) |
| M12 | in progress | Node binary, CLI, keys, double-sign guard, pruning — operator CLI at Go parity (`init`, `keys` on Go test/file keyrings, `genesis` add-account/gentx/collect/validate, `tx` (incl. staking create/edit-validator, unbond, redelegate, `--gas auto`, generic `tx <module> <msg> --json`; `--generate-only` identical to Go's, `devnet/cli`)/`query`/`status`, `config`, `rollback`, `export` = Go's export on 15 devnet scenarios), `--network mainnet\|mocha\|arabica\|local`, SIGTERM shutdown, `halt-height`/`halt-time`, upgrade halt with `upgrade-info.json`, pruning + `min-retain-blocks`, Prometheus metrics, ulimit check; `conformance/devnet/operator.sh`, `docs/operator-guide.md` | `celestia-rs start --network mocha` works end-to-end (public network access pending, OPEN_QUESTIONS #1) |
| M13 | pending     | Mocha validator soak; mainnet full node | Signs on Mocha without misses/double-signs; follows mainnet |
| M14 | pending     | Mainnet validator | Signs mainnet blocks; no double-signs; no evidence |
| B1  | later       | Benchmarks vs Go on the Mac Mini M4 | Report committed under `docs/benchmarks/` |

## Current focus

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
| Not started | testnet state sync (S3) |

M2's disk-backed IAVL and non-empty historical import landed with ADR 0002
(snapshot import of Go state, verified against Go app hashes); S3 now only
needs a public testnet to follow.
