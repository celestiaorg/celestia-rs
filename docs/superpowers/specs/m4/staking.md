# Crate: `staking`

**Tier:** 3 - App modules

**Depends on:** `cosmos-types`, `kvstore`; later `auth` and `bank`

**Consumed by:** M4 integration, later distribution and slashing modules

## Purpose

Implement the consensus-critical `x/staking` state machine used by Celestia
app-v6. This first checkpoint establishes exact validator and delegation
storage before delegation execution is added.

## Public API

- `Keeper::validator`, `set_validator`, and `remove_validator`.
- `Keeper::delegation`, `set_delegation`, and `remove_delegation`.
- Ordered `delegator_delegations` and `validator_delegations` queries.

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

## Conformance Fixtures

`fixturegen staking-state` uses Cosmos SDK v0.50.10 staking key helpers and
protobuf implementations to emit a full validator plus two delegations.
Regenerate with:

```bash
cargo xtask refresh-fixtures --crate-name staking
```

## Definition Of Done

- [x] Validator and delegation primary bytes match Go vectors.
- [x] Delegation reverse-index bytes and ordered queries match Go.
- [ ] `MsgDelegate` moves bond-denom funds and updates shares and power index.
- [x] Focused line coverage is at least 90% (98.01%).
- [x] Repository health is green.

## Out Of Scope For This Checkpoint

- Consensus-address and validator-power indexes.
- Validator creation, editing, and commission updates.
- Undelegation, redelegation, queues, and unbonding IDs.
- End-block validator-set updates and staking hooks.
- Distribution reward accounting.
