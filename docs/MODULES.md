# Porting a state-machine module

How celestia-app v10 modules are ported to Rust. Read this before touching
`crates/tier-3-app-modules/x/` or `crates/tier-4-runtime/app/`.

## Sources of truth

Go sources for every pinned dependency are in the Go module cache
(`GOPATH=/tmp/claude-0/gopath`; populate with
`GOFLAGS=-mod=mod go mod download -json <module>@<version>` from any Go module
dir). Pins (from celestia-app v10.4.0 `go.mod` + `replace`):

| Code | Module path in cache |
|---|---|
| celestia-app | `github.com/celestiaorg/celestia-app/v10@v10.4.0` |
| cosmos-sdk fork | `github.com/celestiaorg/cosmos-sdk@v0.52.12` |
| x/evidence, x/upgrade, x/tx, store forks | `github.com/celestiaorg/cosmos-sdk/{x/evidence@v0.1.2-celestia,x/upgrade@v0.2.0,store@v1.1.3-celestia.3}` |
| collections, math, x/feegrant, x/circuit | `cosmossdk.io/...` (versions in celestia-app `go.mod`) |
| ibc-go fork | `github.com/celestiaorg/ibc-go/v8@v8.7.2` |
| hyperlane | `github.com/celestiaorg/hyperlane-cosmos@v1.3.0` |
| PFM | `github.com/cosmos/ibc-apps/middleware/packet-forward-middleware/v8@v8.2.0` |
| capability | `github.com/cosmos/ibc-go/modules/capability@v1.0.1` |

Port behavior exactly: store keys and value encodings, write order within a
call, gas consumption (every KV access goes through `sdk::Context` metering),
error codespace/code, and events (type, attribute keys/values, order).

## Layout

- One crate per module: `crates/tier-3-app-modules/x/<name>/`, package
  `x-<name>`, with `[lints] workspace = true`. Keepers are free functions
  taking `&mut sdk::Context`; modules hold no in-memory state.
- Protobuf types: `celestia_proto::<package path>` (generated, with serde
  proto3-JSON). `Any` is `celestia_proto::Any` (`pack`/`unpack`).
- Math: `math` crate (`Int`, `LegacyDec`) — exact port of cosmossdk.io/math.
- Collections key encodings: `sdk::codec`.
- Cross-module calls: depend on the other `x-*` crate. Hooks that would form
  a cycle (staking → distribution/slashing) are traits defined by the caller
  and implemented in `app`.

## Wiring

`crates/tier-4-runtime/app/src/modules.rs` holds Go's order lists and the
dispatch `match` for `init_genesis`, `pre_block`, `begin_block`, `end_block`.
Add your module's arms there; do not reorder the lists.

Genesis JSON: `init_genesis` receives the module's `app_state[<name>]`
section. Deserialize with the generated serde types. `bytes` fields that Go
annotates with a math customtype (`LegacyDec`/`Int`) carry decimal strings in
JSON: normalize them before decoding (see `math` helpers).

## Testing (mandatory)

1. Unit tests in the crate.
2. Differential: `cargo run -p devnet-conformance --example replay -- empty`
   shows, per store, the keys Rust is missing/extra/different versus Go at
   the first diverging height. Your module is done for a scenario when its
   store disappears from that list. Then add the store name to
   `EXPECTED_MATCHING_STORES` in `crates/devnet-conformance/tests/replay_empty.rs`.
3. New behavior that the existing scenarios don't exercise needs a new devnet
   scenario (`conformance/devnet/devnet.sh <name> <blocks> <workload.sh>`) or
   Go-generated vectors.

## Gate before pushing

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p <your crates> -p app -p devnet-conformance
```

Push straight to `main` (`git fetch origin main && git rebase origin/main`,
re-run the gate, `git push origin HEAD:main`). Never force-push.
