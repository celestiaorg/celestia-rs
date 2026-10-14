# math

Consensus-compatible Rust port of Go's [`cosmossdk.io/math`](https://pkg.go.dev/cosmossdk.io/math@v1.5.3)
**v1.5.3**, the numeric types the Cosmos SDK state machine uses for amounts,
shares and rates:

- `Int`: signed integer bounded to `±(2^256 - 1)` (`int.go`)
- `Uint`: unsigned integer bounded to `2^256 - 1` (`uint.go`)
- `LegacyDec`: 18-decimal fixed point over a `10^18`-scaled integer bounded to
  `±(2^256 * 10^18 - 1)` (`legacy_dec.go`)

Rounding (banker's rounding in `Mul`/`Quo`/`Power`/`RoundInt`, the round-up and
truncating variants, `ApproxRoot`'s Newton iteration), parsing (Go
`big.Int.SetString` literal syntax for `Int`/`Uint`/binary forms,
`LegacyNewDecFromStr` rules for decimals), `String()` formatting, JSON and the
gogoproto `customtype` binary encoding (`Marshal`/`Unmarshal`) follow the Go
source line by line.

The only intended difference is error handling: where Go panics (overflow,
division by zero, out-of-range conversion) the Rust API returns
`Result<_, MathError>`. Go "nil" values are not modelled; unmarshalling an
empty byte slice yields zero, which marshals to the same `"0"` bytes.

Helpers `legacy_dec_json_to_proto_bytes` and `int_json_to_proto_bytes` turn a
JSON string value into the proto binary form.

## Conformance

`tests/conformance.rs` replays the Go-generated golden vectors in
[`conformance/fixtures/math/vectors.json`](../../../conformance/fixtures/math/vectors.json)
(~12k deterministic random and edge-case operations, including `.5` ties,
negatives, values at the bit limits, `Power` and `ApproxRoot`). Regenerate with:

```bash
cd conformance/mathvectors
GOFLAGS=-mod=mod go run . -out ../fixtures/math/vectors.json
```
