# Conformance

This directory holds **golden vectors** generated from Go celestia-app, plus the Go program that generates them. Every Rust crate that has byte-level Go compatibility requirements asserts against fixtures here.

## Layout

```
conformance/
├── fixtures/        # committed golden vectors (JSON or zstd-compressed binary)
│   └── <crate>/<scenario>/<file>.json
│   └── <crate>/<scenario>/<file>.meta.json   # source chain, height, app version, generating commit
└── fixturegen/      # Go program that emits fixtures; the only Go in the repo
```

## Regenerating fixtures

```bash
cargo xtask refresh-fixtures                       # all scenarios
cargo xtask refresh-fixtures --scenario bech32-addresses
cargo xtask refresh-fixtures --crate-name hash
```

The Rust runner shells out to `go run ./conformance/fixturegen ...`. Go 1.24.11 is required for regeneration only; consumers of fixtures (Rust crates) need no Go toolchain.

## Expanding cached mainnet blocks

The `square-mainnet-blocks` fixture is generated from cached CometBFT block
JSON files under `conformance/fixturegen/cmd/testdata/`. To add more source
blocks, run the fetch utility from the fixturegen module, then regenerate the
derived golden fixture:

```bash
cd conformance/fixturegen
go run . square-fetch-mainnet-blocks --rpc <cometbft-rpc-url> --start <height> --count <n>
cd ../..
cargo xtask refresh-fixtures --scenario square-mainnet-blocks
```

## When to regenerate

- After mainnet upgrades to a new app version.
- After adding a new fixture scenario.
- Never as part of normal feature work — regeneration is a deliberate step that may invalidate test expectations.

Each fixture has a sibling `<file>.meta.json` recording: source chain, height (if applicable), app version, celestia-app commit SHA at generation time, timestamp.
