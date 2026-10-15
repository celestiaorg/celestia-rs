# Legacy amino schema and vectors

go-amino (de)serializes Go values by reflection, so the Rust port of the
legacy `StdTx` codec behind `TxDecodeAmino` / `TxEncodeAmino`
(`crates/tier-1-protocol-types/legacy-amino`) runs over a description of the
Go types it reflects over:

- `schema_test.go` (`TestSchema`) dumps, from a celestia-app v10.4.0 app's
  `LegacyAmino` codec, every type reachable from `legacytx.StdTx` and from
  the registered concrete types: struct fields with their json/amino tags,
  registered names / prefix / disambiguation bytes, interface implementers,
  custom (amino) JSON marshalers, and which `Any` fields each type's
  `UnpackInterfaces` visits with which interface. The output is
  `crates/tier-1-protocol-types/legacy-amino/src/schema.json`.
- `vectors_test.go` (`TestVectors`) calls celestia-appd's
  `TxDecodeAmino` / `TxEncodeAmino` handlers on txs of each message family
  (bank, staking incl. a multisig signer, gov v1/v1beta1, authz, feegrant,
  upgrade, blob, warp) and on malformed binary and JSON, recording outputs,
  errors and panics into `vectors.json`. The crate's tests replay them, and
  the empty-scenario query corpus (`conformance/querycorpus/record.sh empty`)
  sends every vector to a Go node over ABCI `Query`.

Both run inside a celestia-app v10.4.0 checkout:

```bash
mkdir -p <celestia-app>/test/aminodump
cp conformance/aminoschema/*_test.go <celestia-app>/test/aminodump/
cd <celestia-app>
AMINO_SCHEMA_OUT=$OLDPWD/crates/tier-1-protocol-types/legacy-amino/src/schema.json \
  go test -count=1 -run TestSchema ./test/aminodump
AMINO_VECTORS_OUT=$OLDPWD/conformance/aminoschema/vectors.json \
  go test -count=1 -run TestVectors ./test/aminodump
```
