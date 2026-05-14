# celestia-rs

A Rust port of [celestia-app](https://github.com/celestiaorg/celestia-app) — a wire-compatible Celestia consensus node implementation.

**Status:** Pre-M1. Active scaffolding. Not usable for anything yet.

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

## Regenerating conformance fixtures

Requires Go 1.22+:

```
cargo xtask refresh-fixtures
```

## License

Dual-licensed under [Apache 2.0](LICENSE-APACHE) and [MIT](LICENSE-MIT).
