# Crate: `bank`

**Tier:** 3 - App modules

**Depends on:** `ante`, `cosmos-types`, `kvstore`

**Consumed by:** M4 `staking`, M4 integration, later SDK modules and runtime

## Purpose

Implement the consensus-critical `x/bank` balance and supply store used by
Celestia app-v6, including the fee deduction adapter consumed by M3 ante.

## Public API

- `Keeper::balance`, `all_balances`, and `set_balance`.
- `Keeper::supply` and `set_supply`.
- `Keeper::send_coins`, `mint_coins`, and `burn_coins`.
- `ante::FeeKeeper` implementation and fee collector address derivation.

## Behavioral Requirements

### BANK-REQ-1: Collection encoding

- The mounted store name is `bank`.
- Balance keys use prefix `0x02` followed by the SDK pair encoding of raw
  account address and denomination: address length, address bytes, then denom.
- Balance values are canonical decimal `math.Int` bytes.
- Every non-zero balance maintains the reverse index at prefix `0x03`, encoded
  as denomination, zero delimiter, address length, and address bytes; its value
  is empty.
- Supply keys use prefix `0x00` followed by denomination; values use the same
  canonical decimal integer encoding.
- Zero balances and supply are deleted rather than persisted.

### BANK-REQ-2: Integer and coin rules

- Amount arithmetic supports the SDK's full unsigned 256-bit positive range.
- Stored values reject signs, leading zeroes, invalid UTF-8, and values above
  256 bits.
- Multi-coin operations require sorted, unique denominations and positive
  amounts.

### BANK-REQ-3: State transitions

- Sends debit and credit every denomination without changing supply.
- Mint and burn update balances and supply together.
- Preflight validation prevents partial writes on insufficient funds or
  arithmetic overflow.

### BANK-REQ-4: Ante integration

- Base accounts have no locked vesting coins, so spendable balance equals bank
  balance for this checkpoint.
- Fee deduction transfers the fee coins from the payer to the legacy
  `fee_collector` module address.
- Insufficient bank funds map to `ante::Error::InsufficientFunds`; other keeper
  failures cross the trait boundary as `ante::Error::State`.

## Conformance Fixtures

`fixturegen bank-balances` uses Cosmos SDK v0.50.10 collection, bank, and
`math.Int` codecs to emit balance, reverse-index, and supply bytes for native,
IBC, and maximum-width amounts. Regenerate with:

```bash
cargo xtask refresh-fixtures --crate-name bank
```

## Definition Of Done

- [x] Balance, reverse-index, and supply bytes match Go vectors.
- [x] Transfers, mint, burn, zero deletion, and 256-bit arithmetic are tested.
- [x] Ante fee deduction credits the fee collector.
- [ ] `MsgSend` and `MsgMultiSend` routing emits Go-compatible events.
- [ ] Bank subtree roots match Go during M4 block replay.
- [x] Focused line coverage is at least 90% (97.00%).
- [x] Repository health is green.

## Out Of Scope For This Checkpoint

- Vesting locked-coin calculations.
- Automatic auth-account creation for previously unseen send recipients.
- Denomination metadata, send-enabled parameters, and blocked addresses.
- Module permission checks around mint and burn.
- gRPC/query services.
