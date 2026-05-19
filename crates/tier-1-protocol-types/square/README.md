# square

Celestia data-square layout primitives for `celestia-rs`.

This crate is the M1 layer above `shares`. It will own original data square
construction, blob share placement, blob commitment rules, erasure extension,
and data availability headers. The current implementation checkpoint covers
the low-level blob transaction wrapper, inclusion math, square sizing, and
`write_square` layout foundation, builder/export mechanics, blob commitments,
data availability header value helpers, and Leopard erasure-coded EDS helpers.

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
builder/export behavior.
