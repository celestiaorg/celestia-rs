# celestia-rs

A Rust port of [celestia-app](https://github.com/celestiaorg/celestia-app) — a wire-compatible Celestia consensus node implementation.

**Status:** M2 state-and-storage hardening. M1 is complete; M2 crates are
implemented and the current work is replacing the synthetic M2 acceptance
fixture with real replay-state mainnet state diffs.

## Design

- Top-level design: [`docs/superpowers/specs/2026-05-14-celestia-rs-design.md`](docs/superpowers/specs/2026-05-14-celestia-rs-design.md)
- Current focus: [`MILESTONES.md`](MILESTONES.md)
- Per-milestone specs: [`docs/superpowers/specs/m1/`](docs/superpowers/specs/m1/) (and `m2/`, `m3/`, … as we go)

## Layout

```
crates/       # Rust workspace members, organized by dependency tier
conformance/  # Golden vectors + Go fixture generator
xtask/        # cargo-xtask runner (fixture regen, vendor bumps)
docs/         # Specs, plans, ADRs
```

## Build

```
cargo build --workspace
cargo test --workspace
cargo xtask --help
```

## Repository health

Run the local gate before moving between milestone sections:

```
cargo xtask repo-health --offline
```

Use `--dry-run` to print the command sequence and fixture inventory without
running the gates.

## Regenerating conformance fixtures

Requires Go 1.22+:

```
cargo xtask refresh-fixtures
```

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
