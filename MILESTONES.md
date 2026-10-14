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
| M2  | completed*  | State & storage (IAVL, kvstore, multistore, ICS-23) | DB-backed replay shard; *non-empty historical import pending |
| M3  | completed   | Tx & ante machinery | Tx-validation outcome matches Go for 1,062 mainnet txs |
| M4  | in progress | Core SDK modules (auth, bank, staking) | Keeper state matches Go vectors |
| D1  | completed   | Devnet differential oracle: Go `celestia-appd` v10 devnet in-container, exported blocks + per-block app hashes + genesis as fixtures | `cargo xtask devnet-fixtures` regenerates a committed corpus |
| R1  | pending     | Retarget implemented crates from app v6 to v10 (shares/square/ante/modules diffs) | Existing conformance suites pass against v10-generated fixtures |
| S1  | completed   | **Vertical slice:** ABCI++ server + `App` (InitChain/FinalizeBlock/Commit/Info/Query) running behind Go celestia-core | Rust app replays devnet blocks with matching app hash for an empty-tx chain |
| S2  | in progress | Grow the slice until every devnet scenario matches: remaining SDK modules (M5), celestia modules incl. blob/fibre/minfee/signal/mint (M6), IBC (M7), PrepareProposal/ProcessProposal | App hash matches Go for every block of the devnet scenario corpus |
| S3  | in progress | State sync client + snapshot import (devnet Go↔Rust done); follow Arabica/Mocha as a full node behind celestia-core | Full node follows the testnet tip with matching app hashes |
| M9  | in progress | Rust consensus engine (Tendermint BFT, celestia-core semantics) — `privval`, `nodestore`, `consensus` + single-validator `celestia-rs start` done (Go verifies and replays its chain); reactor wiring to `p2p` pending | Mixed Rust+Go private devnet reaches consensus |
| M10 | in progress | Rust P2P (secret connection, MConn, PEX), mempool, block sync — `p2p`, CAT `mempool`, `blocksync`, `evidence`, `statesync` done; `celestia-rs start` block-syncs / state-syncs from Go and serves Go (`blocksync-interop.sh`) | Rust node peers with Go nodes and gossips blocks/txs |
| M11 | completed   | RPC & gRPC: ABCI `Query` (store proofs, historical heights, gRPC routing), every query service, gRPC server and REST gateway in `celestia-rs abci` | Responses match Go for a canned request corpus (13 scenarios, 46.8k ABCI + 30.6k REST requests, `devnet-conformance/tests/query_corpus.rs`) |
| M12 | pending     | Node binary, CLI, keys, double-sign guard, pruning | `celestia-rs start --network mocha` works end-to-end |
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
| IBC differential | `devnet/ibc` (two Go chains + cosmos/relayer) and `devnet/ibc-edge` (ibc-go testing package, `conformance/ibcvectors`) at full parity incl. CheckTx |
| Deferred (IBC) | 06-solomachine verification, channel upgrades, IBCSoftwareUpgrade scheduling, ICA `proto3json` txs and `MsgModuleQuerySafe` |
| Light client & state sync (ADR 0003) | `light` (verifier, bisection, detector, store, RPC provider; Go vectors 720 verifier + 196 client scenarios at parity) and `statesync` (reactor, syncer, state provider); a Rust node state-syncs from Go peers and serves snapshots to Go (`conformance/devnet/statesync-rust.sh`) |
| Queries (M11) | ABCI `Query` (store/ICS-23 proofs, app, p2p, custom inclusion proofs, gRPC routing at historical heights) and every query service incl. hyperlane/warp, zkism, forwarding, IBC, reflection, cmtservice/tx status/gas estimation over celestia-core RPC; gRPC server (`--grpc-address`) and REST gateway (`--api-address`) in `celestia-rs abci`; byte-identical to Go on the 13-scenario query corpus |
| Deferred (M11) | Legacy amino tx codec (`TxDecodeAmino`/`TxEncodeAmino` beyond empty requests), reflection `v2alpha1`/`cosmos.reflection.v1`, the validator-local fibre promise cache, Go build info in `GetNodeInfo` |
| Not started | testnet state sync (S3), Rust consensus/P2P (M9/M10) |

M2's disk-backed IAVL and non-empty historical import landed with ADR 0002
(snapshot import of Go state, verified against Go app hashes); S3 now only
needs a public testnet to follow.
