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

`conformance/fixtures/ante/mainnet-tx-corpus.json.zst` and its metadata sidecar
contain 1,062 replay-accepted transactions sampled from app-v6 mainnet heights
10,126,900 through 10,127,119. The generator samples deterministically,
hydrates each case from its own pre-block RPC state (with a same-block-created
account fallback), retries transient archive failures, and obtains the expected
envelope from the real celestia-app Go ante handler. The runner decodes and
validates the corpus. Rust now byte-matches the ordered SDK events emitted by
the fee, bank, and signer decorators for all 1,057 direct-sign cases. The full
comparison remains a red implementation gate: Rust does not yet model the
storage gas emitted by circuit, minfee, auth, and bank keeper access, and five
cases use legacy Amino JSON signing.

The Go `antereplay` input accepts diagnostic `gas_profile` values that isolate
one persistent or transient KV gas coefficient at a time. This makes the
remaining storage-gas work measurable as operation counts and byte totals. For
the first corpus case the profiles account exactly for the 44,018-gas gap: 3
has calls, 1 delete, 13 reads over 1,546 bytes, and 6 writes over 346 bytes.

## Definition Of Done

- [x] Add DB-backed pre-block account and fee-balance hydration to
      `fixturegen ante-mainnet-tx-corpus`.
- [x] Add deterministic corpus case assembly from sampled transactions and
      hydrated account/balance state.
- [x] Write `mainnet-tx-corpus` fixture and metadata when replay supplies
      expected Go ante outcomes.
- [x] Add per-transaction RPC state hydration and Go ante replay in
      `fixturegen ante-mainnet-tx-corpus`.
- [x] Commit `conformance/fixtures/ante/mainnet-tx-corpus.json.zst` plus
      `.meta.json`.
- [x] Implement the full fixture runner in `m3-integration`.
- [x] Match ordered Go ante events for at least 1000 direct-sign transactions.
- [ ] Match Go storage gas in the Rust runner.
- [ ] Support the five legacy Amino JSON cases in the committed corpus.
- [ ] Remove the ignore from `mainnet_tx_validation_matches_go`.
- [x] Run `cargo xtask repo-health --offline`.
- [ ] Mark M3 complete once the acceptance test passes for at least 1000
      mainnet transactions.
