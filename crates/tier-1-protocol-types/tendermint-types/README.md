# tendermint-types

Tier-1 CometBFT protocol types for celestia-rs.

## Purpose

Provides CometBFT block IDs, headers, commits, validators, votes, proposals,
canonical sign bytes, and the trimmed protobuf surface needed by later M1
crates.

## Depends on

- `hash`
- `encoding`
- `crypto`
- `merkle`

The crate currently keeps generated CometBFT v0.38.12 prost modules behind its
own API via `tendermint-proto`.

## Consumed by

- `cosmos-types`
- `abci-types`
- `shares`
- `square`
- `m1-integration`
- downstream consensus and runtime crates

## Public API

```rust
pub mod block;
pub mod canonical;
pub mod commit;
pub mod error;
pub mod hash;
pub mod proposal;
pub mod proto;
pub mod validator;
pub mod vote;
```

See
[`docs/superpowers/specs/m1/tendermint-types.md`](../../../docs/superpowers/specs/m1/tendermint-types.md)
for the full API contract.

## Conformance

Fixture files live at
[`conformance/fixtures/tendermint-types/`](../../../conformance/fixtures/tendermint-types/):

| File | Scenario | Description |
|---|---|---|
| `headers.json` | `tendermint-headers` | CometBFT header protobuf and header-hash vectors |
| `validators.json` | `tendermint-validators` | CometBFT validator bytes, ordering, proposer, and validator-set hash vectors |
| `commits.json` | `tendermint-commits` | CometBFT commit protobuf, commit-hash, and vote sign-byte vectors |
| `canonical-sign-bytes.json` | `tendermint-canonical-sign-bytes` | CometBFT vote, vote-extension, and proposal sign-byte vectors |

Regenerate fixtures:

```bash
cargo xtask refresh-fixtures --crate-name tendermint-types
```

Run conformance tests:

```bash
cargo test -p tendermint-types --test conformance
```

## Out of scope

- ABCI request/response types
- Cosmos SDK transactions
- Evidence verification
- Consensus state machines
- Networking and RPC
