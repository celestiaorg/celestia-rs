# antereplay

Go ante **replay oracle** for the M3 mainnet transaction corpus. It builds an
in-memory [celestia-app](https://github.com/celestiaorg/celestia-app) v6
instance and runs its app-v6 ante handler against fixture-provided account and
balance snapshots to produce the `expected` Go ante outcome for each sampled
transaction.

## Why a separate module

celestia-app pins a celestia-specific cosmos-sdk fork (and celestia-core) via
`replace` directives. Placing celestia-app in the `conformance/fixturegen`
module would override the upstream `github.com/cosmos/cosmos-sdk` that the other
fixture generators build against. To keep those generators on upstream
cosmos-sdk, the ante replay is isolated in this module and is invoked by
`fixturegen ante-mainnet-tx-corpus` as a subprocess.

## Status

The module builds against celestia-app v6, constructs a full in-memory app,
injects per-case account and balance snapshots, executes the production ante
handler, and returns gas, events, signer sequences, and fee balances through
the fixturegen subprocess interface.

For storage-gas analysis, an input case may set `gas_profile`. `default` (or an
omitted field) uses the SDK schedules, while `zero` disables KV gas. The
one-coefficient profiles below set the named cost to one and every other
persistent and transient KV cost to zero:

- `kv_has`, `kv_delete`, `kv_read_flat`, `kv_read_per_byte`,
  `kv_write_flat`, `kv_write_per_byte`, `kv_iter_next_flat`
- `transient_has`, `transient_delete`, `transient_read_flat`,
  `transient_read_per_byte`, `transient_write_flat`,
  `transient_write_per_byte`, `transient_iter_next_flat`

Subtracting a case's `zero` result from a one-coefficient result yields the
operation count or byte total charged by that coefficient. For the first
committed mainnet case this decomposes the 44,018 storage gas into 3 has calls,
1 delete, 13 reads covering 1,546 bytes, and 6 writes covering 346 bytes.

For a default-profile accepted case, the JSON response performs those
diagnostic replays automatically and includes the resulting `storage_gas`
trace. Rejected cases omit the trace because changing the gas schedule may move
their rejection point.

## Build

Requires Go 1.25+.

```
cd conformance/antereplay
go build ./...
go test ./...
```
