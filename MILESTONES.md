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
| D1  | in progress | Devnet differential oracle: Go `celestia-appd` v10 devnet in-container, exported blocks + per-block app hashes + genesis as fixtures | `cargo xtask devnet-fixtures` regenerates a committed corpus |
| R1  | pending     | Retarget implemented crates from app v6 to v10 (shares/square/ante/modules diffs) | Existing conformance suites pass against v10-generated fixtures |
| S1  | pending     | **Vertical slice:** ABCI++ server + `App` (InitChain/FinalizeBlock/Commit/Info/Query) running behind Go celestia-core | Rust app replays devnet blocks with matching app hash for an empty-tx chain |
| S2  | pending     | Grow the slice until every devnet scenario matches: remaining SDK modules (M5), celestia modules incl. blob/fibre/minfee/signal/mint (M6), IBC (M7), PrepareProposal/ProcessProposal | App hash matches Go for every block of the devnet scenario corpus |
| S3  | pending     | State sync client + snapshot import; follow Arabica/Mocha as a full node behind celestia-core | Full node follows the testnet tip with matching app hashes |
| M9  | pending     | Rust consensus engine (Tendermint BFT, celestia-core semantics) | Mixed Rust+Go private devnet reaches consensus |
| M10 | pending     | Rust P2P (secret connection, MConn, PEX), mempool, block sync | Rust node peers with Go nodes and gossips blocks/txs |
| M11 | pending     | RPC & gRPC | Responses match Go for a canned request corpus |
| M12 | pending     | Node binary, CLI, keys, double-sign guard, pruning | `celestia-rs start --network mocha` works end-to-end |
| M13 | pending     | Mocha validator soak; mainnet full node | Signs on Mocha without misses/double-signs; follows mainnet |
| M14 | pending     | Mainnet validator | Signs mainnet blocks; no double-signs; no evidence |
| B1  | later       | Benchmarks vs Go on the Mac Mini M4 | Report committed under `docs/benchmarks/` |

## Current focus

**D1 + S1.** Build the Go devnet oracle (Go is available through the module
proxy; public chain RPC is blocked — open question 1) and stand up the ABCI
vertical slice so app-hash mismatches drive the remaining module work. M4
keeper work continues as the slice needs it.

M2's non-empty historical-state import hardening remains pending because
arbitrary IAVL snapshots need preserved node-version metadata; S3 (state sync)
will revisit it.
