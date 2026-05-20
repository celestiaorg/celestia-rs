# square

Celestia data-square layout primitives for `celestia-rs`.

This crate is the M1 layer above `shares`. It owns original data square
construction, blob share placement, blob commitment rules, erasure extension,
and data availability headers. The current implementation covers blob
transaction wrappers, inclusion math, square sizing, `write_square` layout,
builder/export mechanics, share ranges, deconstruction, blob commitments, data
availability header helpers, Leopard erasure-coded EDS helpers, and cached
mainnet DAH validation.

## Depends On

- `shares`
- `encoding`
- `hash`
- `leopard-codec`
- `merkle`

## Consumers

- M1 integration tests
- Later DAH, proof, consensus, and RPC crates

## Conformance

The implementation spec is
`docs/superpowers/specs/m1/square.md`. Go-generated conformance fixtures will
be committed under `conformance/fixtures/square/` and regenerated with:

```bash
cargo xtask refresh-fixtures --crate-name square
```

The current fixture checkpoint covers power-of-two sizing helpers, blob
inclusion-rule placement, blob transaction wrappers, blob commitments, and
builder/export behavior, direct square layout, share ranges, and square
deconstruction, plus Leopard EDS extension, import/export, and row/column NMT
roots, DAH roots, hashes, and protobuf encoding, 100 cached mainnet app-v6
block DAH vectors, and aggregated malformed input/error vectors. Additional
cached mainnet blocks can be added as
`conformance/fixturegen/cmd/testdata/mainnet_block_<height>.json`; the
`square-mainnet-blocks` generator includes every matching embedded file. The
source testdata can be fetched from a CometBFT RPC endpoint with:

```bash
cd conformance/fixturegen
go run . square-fetch-mainnet-blocks --rpc <cometbft-rpc-url> --start <height> --count <n>
```
