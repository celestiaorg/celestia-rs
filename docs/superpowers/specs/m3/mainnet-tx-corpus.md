# M3 Mainnet Transaction Corpus

## Purpose

Generate the committed transaction corpus that turns the current `ante`
implementation and `m3-integration::mainnet_tx_validation_matches_go` scaffold
into the M3 acceptance gate. The fixture must compare Rust ante validation
against Go celestia-app for at least 1000 real mainnet transactions without
requiring a full Rust `x/auth` or `x/bank` implementation.

## Generator Contract

`conformance/fixturegen` should expose:

```bash
go run . ante-mainnet-tx-corpus \
  --rpc <cometbft-rpc-url> \
  --db-dir <celestia-data-dir> \
  --start <height> \
  --count <blocks> \
  --min-txs 1000 \
  --out conformance/fixtures/ante/mainnet-tx-corpus.json.zst
```

The command samples committed mainnet transactions from app-v6 blocks,
hydrates the minimal pre-ante account and bank state needed for each signer and
fee payer, runs the Go ante handler in every emitted run mode, and records the
Go result next to the raw transaction bytes. The fixture must be deterministic:
cases are ordered by block height, transaction index, then run mode.

Required inputs:

1. A CometBFT RPC endpoint for block transaction bytes.
2. A local Celestia app application DB, exported state, or equivalent account
   and bank snapshot source for the sampled heights.
3. `--start`, `--count`, and `--min-txs` identifying the mainnet sampling
   window.

## Fixture Schema

The top-level JSON object is:

- `scenario`: `ante-mainnet-tx-corpus`.
- `source`: human-readable source description.
- `chain_id`: expected Celestia chain ID, for example `celestia`.
- `app_version`: Celestia app version used by the sampled blocks.
- `cases`: at least 1000 transaction validation cases.

Each case records:

- `label`: stable identifier such as `height-10126900-tx-3-deliver`.
- `network`: `mainnet`.
- `height`: block height containing the transaction.
- `tx_index`: zero-based transaction index in that block.
- `mode`: `check`, `recheck`, `deliver`, or `simulate`.
- `tx_raw_hex`: encoded `cosmos.tx.v1beta1.TxRaw` bytes.
- `min_gas_prices`: configured minimum gas prices for the run.
- `accounts`: minimal account snapshot before ante, with `address_hex`,
  `account_number`, `sequence`, and optional `public_key_any_hex`.
- `balances`: spendable fee balances before ante, with `address_hex`, `denom`,
  and `amount`.
- `expected`: Go result envelope.

The `expected` envelope records:

- `ok`: whether Go accepted the transaction.
- `error`: stable Rust-facing error code for rejected transactions.
- `code`, `codespace`, and `log`: Go ABCI result fields.
- `gas_wanted` and `gas_used`.
- `events`: ordered ABCI events with ordered attributes.
- `signer_sequences`: final signer sequence summary.
- `balances`: final fee-balance summary.

The sidecar `mainnet-tx-corpus.meta.json` must record source chain, sampled
height range, number of source transactions, number of emitted cases, app
version, celestia-app commit, Cosmos SDK version, Go version, RPC endpoint
label, state source label, and the exact regeneration command.

## Rust Acceptance Behavior

`m3-integration::mainnet_tx_validation_matches_go` prefers
`conformance/fixtures/ante/mainnet-tx-corpus.json.zst`, falls back to
`mainnet-tx-corpus.json`, loads `mainnet-tx-corpus.meta.json`, checks that the
metadata matches the fixture provenance and emitted case count, checks that the
fixture contains at least 1000 cases, runs each case through
`ante::AnteHandler` with fixture-backed account and fee adapters, and compares
the Rust outcome or error against the `expected` envelope.

The first implementation may compare the fields already emitted by the current
`ante` public API. If Go exposes more result detail than Rust can yet report,
the test should make that gap explicit with a typed comparison TODO in the
fixture runner rather than silently ignoring the field.

## Current Status

`crates/m3-integration/` contains the ignored acceptance entry point, but
`conformance/fixtures/ante/mainnet-tx-corpus.json.zst` is not committed yet.
The runner can decode the compressed corpus and validate its metadata sidecar
once generated. `fixturegen ante-mainnet-tx-corpus` has validated
configuration, deterministic RPC transaction sampling, sampled `TxRaw`
signer/fee decoding, SDK auth/bank hydration-key planning, DB-backed pre-block
account and fee-balance hydration, deterministic corpus case assembly, and
metadata scaffolding. Per-transaction state replay through earlier
transactions in the same block and the real Go ante replay adapter are still
pending. The current `ante` fixtures are still explicit unit-scale vectors, not
a broad mainnet corpus.

## Definition Of Done

- [x] Add DB-backed pre-block account and fee-balance hydration to
      `fixturegen ante-mainnet-tx-corpus`.
- [x] Add deterministic corpus case assembly from sampled transactions and
      hydrated account/balance state.
- [ ] Complete per-transaction same-block state replay and Go ante replay in
      `fixturegen ante-mainnet-tx-corpus`.
- [ ] Commit `conformance/fixtures/ante/mainnet-tx-corpus.json.zst` plus
      `.meta.json`.
- [ ] Implement the full fixture runner in `m3-integration`.
- [ ] Remove the ignore from `mainnet_tx_validation_matches_go`.
- [ ] Run `cargo xtask repo-health --offline`.
- [ ] Mark M3 complete once the acceptance test passes for at least 1000
      mainnet transactions.
