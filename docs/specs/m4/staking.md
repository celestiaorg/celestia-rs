# Crate: `staking`

**Tier:** 3 - App modules

**Depends on:** `abci-types`, `bank`, `cosmos-types`, `kvstore`

**Consumed by:** M4 integration, later distribution and slashing modules

## Purpose

Implement the consensus-critical `x/staking` state machine used by Celestia
app-v6. The current checkpoint adds exact `MsgDelegate` execution to validator
and delegation storage.

## Public API

- `Keeper::validator`, `set_validator`, and `remove_validator`.
- `Keeper::delegation`, `set_delegation`, and `remove_delegation`.
- Ordered `delegator_delegations` and `validator_delegations` queries.
- `Keeper::execute_message` routing for protobuf `MsgDelegate`.
- Validator power-index get, set, and remove operations.

## Behavioral Requirements

### STAKING-REQ-1: Validator primary storage

- The mounted store name is `staking`.
- Validator keys use prefix `0x21`, a one-byte operator-address length, and the
  raw validator operator address.
- Values use the SDK v0.50 `cosmos.staking.v1beta1.Validator` protobuf shape.
- A decoded record's operator address must match its storage key.

### STAKING-REQ-2: Delegation storage

- Primary keys use prefix `0x31`, length-prefixed delegator and validator
  addresses, and protobuf `Delegation` values.
- Every delegation maintains an empty reverse-index value under prefix `0x71`,
  a length-prefixed validator address, and raw delegator bytes.
- A decoded record's addresses must match its storage key. Protobuf shares use
  the canonical non-negative integer atomics string for SDK `LegacyDec` values
  (the human representation has 18 decimal places).
- Removing a delegation removes both primary and reverse-index records.

### STAKING-REQ-3: Delegation execution

- `MsgDelegate` accepts a positive `utia` amount and an existing validator.
- Delegator funds move to `bonded_tokens_pool` for bonded validators and to
  `not_bonded_tokens_pool` for unbonded or unbonding validators.
- Issued shares use SDK `LegacyDec` atomics and exact integer division. The
  first delegation establishes a 1:1 token/share exchange rate.
- The transition updates validator tokens and shares, creates or updates the
  delegation and reverse index, and replaces the validator's `0x23` power key.
- Jailed validators are omitted from the power index.
- `coin_spent`, `coin_received`, and `delegate` events preserve SDK ordering and
  attribute values.

## Conformance Fixtures

`fixturegen staking-state` uses Cosmos SDK v0.50.10 staking key helpers and
protobuf implementations to emit a full validator plus two delegations.
`fixturegen staking-delegate` additionally emits a fractional-share delegation
transition, both power keys, bank balances, and events. Regenerate with:

```bash
cargo xtask refresh-fixtures --crate-name staking
```

## Definition Of Done

- [x] Validator and delegation primary bytes match Go vectors.
- [x] Delegation reverse-index bytes and ordered queries match Go.
- [x] `MsgDelegate` moves bond-denom funds and updates shares and power index.
- [x] Focused line coverage is at least 90% (97.32%).
- [x] Repository health is green.

## Out Of Scope For This Checkpoint

- Consensus-address index.
- Validator creation, editing, and commission updates.
- Undelegation, redelegation, queues, and unbonding IDs.
- End-block validator-set updates and staking hooks.
- Distribution reward accounting.
