# celestia-rs Milestones

Authoritative source for "what is next." Updated when a milestone starts or completes.

See `docs/superpowers/specs/2026-05-14-celestia-rs-design.md` for full descriptions.

| #   | Status      | Milestone                            | Acceptance test |
|-----|-------------|--------------------------------------|-----------------|
| M0  | completed   | Bootstrap (scaffolding + fixturegen) | `cargo xtask refresh-fixtures --scenario bech32-addresses` produces a valid fixture |
| M1  | pending     | Foundational libraries               | `m1-integration::parse_real_mainnet_block_produces_matching_dah` passes |
| M2  | pending     | State & storage                      | `m2-integration::apply_mainnet_block_diff_produces_matching_kv_root` passes |
| M3  | pending     | Tx & ante machinery                  | Tx-validation outcome matches Go for ≥ 1000 mainnet txs |
| M4  | pending     | Core SDK modules (auth, bank, staking) | Per-module subtree roots match Go after replaying mainnet blocks |
| M5  | pending     | Remaining SDK modules                | Per-module state roots match Go |
| M6  | pending     | Celestia-specific modules            | App hash matches Go for blob-tx mainnet blocks |
| M7  | pending     | IBC                                  | IBC-touching mainnet blocks replay; LC verifies headers |
| M8  | pending     | ABCI++ app integration               | App hash matches Go for 1000 consecutive mainnet blocks |
| M9  | pending     | Consensus engine                     | Two celestia-rs nodes form a private network |
| M10 | pending     | P2P networking                       | Connects to Go peers and gossips a block |
| M11 | pending     | RPC & gRPC                           | RPC responses match Go for canned request corpus |
| M12 | pending     | Node binary, CLI, ops                | `celestia-rs start --network mainnet` works end-to-end |
| M13 | pending     | Mainnet sync from genesis            | Final app hash equals current mainnet's |
| M14 | pending     | Mainnet validator                    | Signs blocks on mainnet without double-signs |

## Current focus
**M1** — see `docs/superpowers/specs/m1/_index.md`. Tier-0 crates 1-4 (`hash`,
`encoding`, `crypto`, and `merkle`) and crate 5 (`tendermint-types`) are
implemented. Crate 6 (`cosmos-types`) now has a spec and is the next strict
dependency step; the remaining tier-1 specs are still to be written.
