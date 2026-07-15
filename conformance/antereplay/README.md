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

Scaffold: the module builds against celestia-app v6 and constructs a full
in-memory app (with its wired ante handler). Snapshot injection, ante
execution, and the fixturegen subprocess interface are added on top of this.

## Build

Requires Go 1.25+.

```
cd conformance/antereplay
go build ./...
go test ./...
```
