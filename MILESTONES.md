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
| M9  | pending     | Rust consensus engine (Tendermint BFT, celestia-core semantics) | Mixed Rust+Go private devnet reaches consensus |
| M10 | pending     | Rust P2P (secret connection, MConn, PEX), mempool, block sync | Rust node peers with Go nodes and gossips blocks/txs |
| M11 | pending     | RPC & gRPC | Responses match Go for a canned request corpus |
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
| Modules at parity | auth (+ vesting), bank, staking, distribution, slashing, mint, gov (all msgs, tally, execution), upgrade (plans, v10 handler, signalled upgrades, halt), consensus (MsgUpdateParams), authz, feegrant, evidence, params, circuit, blob (msgs), minfee, signal, fibre (blocks), valaddr, zkism, forwarding (genesis), capability, ibc, transfer, icahost, pfm (genesis/begin), hyperlane + warp (all msgs) |
| At parity (blocks) | PrepareProposal / ProcessProposal (go-square v4 builder, DAH) and CheckTx, replayed for every scenario incl. `devnet/blob`; Rust-proposed blocks accepted by a Go full node (`rust-proposer.sh`) |
| Deferred | Pay-for-fibre settlement in proposals (x/fibre Msg server not routed); ProcessProposal blob-tx cache (behavior-neutral) |
| Storage (ADR 0002) | Persistent IAVL on redb (lazy nodes, LRU cache, history, pruning), atomic multistore commits, restart/handshake (`celestia-rs abci --home`), SDK snapshots + ABCI state sync; every scenario replays on redb, incl. restart-halfway and snapshot-restore replays; Go↔Rust state sync both ways (`conformance/devnet/statesync.sh`) |
| In progress | IBC msgs + relaying |
| Not started | queries/gRPC/REST (M11), testnet state sync (S3), Rust consensus/P2P (M9/M10) |

M2's disk-backed IAVL and non-empty historical import landed with ADR 0002
(snapshot import of Go state, verified against Go app hashes); S3 now only
needs a public testnet to follow.
