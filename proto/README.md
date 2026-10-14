# Vendored protobuf sources

Exact `.proto` sources for the versions celestia-app v10.4.0 pins, compiled
into prost bindings by `cargo xtask gen-proto` (pure-Rust `protox`, no
`protoc` needed). Generated code is committed under
`crates/tier-1-protocol-types/{celestia-core-proto,celestia-proto}/src/gen`.

| Dir | Source |
|---|---|
| `celestia-core/` | `celestiaorg/celestia-core` v0.42.3 |
| `cosmos-sdk/` | `celestiaorg/cosmos-sdk` v0.52.12 (minus orm/nft/group/app/autocli/reflection and depinject `module.proto`s) |
| `ibc-go/` | `celestiaorg/ibc-go/v8` v8.7.2 |
| `celestia-app/` | `celestiaorg/celestia-app/v10` v10.4.0 |
| `hyperlane-cosmos/` | `celestiaorg/hyperlane-cosmos` v1.3.0 |
| `packet-forward-middleware/` | `cosmos/ibc-apps/middleware/packet-forward-middleware/v8` v8.2.0 |
| `third_party/` | gogoproto, cosmos_proto, google/api, and a name-only `cosmos/ics23` stub (mapped to the `ics23` crate) |

Local edits (wire-format neutral) are marked `celestia-rs:` in the source:

- `cosmos/staking/v1beta1/authz.proto`: oneof `validators` → `policy`
  (prost name collision with the nested `Validators` message).
