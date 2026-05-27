# ante

Cosmos SDK transaction ante machinery for `celestia-rs`.

## Purpose

This crate is the M3 transaction-validation layer. It owns stateless
transaction checks, ante gas accounting, signer extraction scaffolding, and the
trait boundary that later `x/auth` and `x/bank` keepers will implement.

The current checkpoint provides the crate skeleton, gas-meter behavior,
stateless `TxRaw` validation, direct secp256k1 deliver execution, and fee-path
coverage. The stateless and signature/account-state fixture groups are
Go-generated, as are the first fee deduction and gas-accounting fixtures; the
mainnet transaction corpus is next.

## Depends On

- `abci-types`
- `cosmos-types`
- `crypto`
- `hash`
- `kvstore`

## Consumers

- M3 integration tests
- M4 account and bank module keepers
- Later ABCI app/runtime crates

## Conformance

The implementation spec is `docs/superpowers/specs/m3/ante.md`.
Go-generated fixtures live under `conformance/fixtures/ante/` and are
regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name ante
```

| File | Scenario | Description |
|---|---|---|
| `stateless.json` | `ante-stateless` | `TxRaw` stateless validation, unsupported sign modes, memo, extension, and signature-count cases |
| `signatures.json` | `ante-signatures` | SIGN_MODE_DIRECT sign bytes, signer extraction, sequence drift, stored public-key mismatch, invalid public keys, and bad signatures |
| `fees.json` | `ante-fees` | Minimum gas prices, implicit and explicit fee payers, insufficient fees/funds, and balance deductions |
| `gas.json` | `ante-gas` | Tx-size gas, signature-verification gas, simulate mode, and out-of-gas failures |
