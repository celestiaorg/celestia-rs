# ante

Cosmos SDK transaction ante machinery for `celestia-rs`.

## Purpose

This crate is the M3 transaction-validation layer. It owns stateless
transaction checks, ante gas accounting, signer extraction scaffolding, and the
trait boundary that later `x/auth` and `x/bank` keepers will implement.

The current checkpoint provides the crate skeleton, gas-meter behavior,
stateless `TxRaw` validation, direct secp256k1 deliver execution, and fee-path
coverage. It also emits the Cosmos SDK fee-transfer, fee, account-sequence, and
signature events in Go order. The stateless and signature/account-state fixture
groups are Go-generated, as are the fee deduction, gas-accounting, malformed
transaction, and 1,062-case mainnet transaction fixtures.

In addition to direct signing, the ante handler reproduces canonical legacy
Amino JSON sign bytes for the delegate and withdraw-delegator-reward message
shapes exercised by the mainnet corpus.

The mainnet runner supplies measured fixture-backed keeper gas as pre-consumed
context gas. `AnteHandler::run` preserves that consumption when it applies the
transaction gas limit, allowing complete Go gas parity without leaking fixture
or store implementation details into the account and fee traits.

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
| `malformed.json` | `ante-malformed` | Malformed protobuf bytes and invalid signer-info, public-key, sign-mode, and fee fields |
