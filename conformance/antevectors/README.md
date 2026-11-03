# antevectors

Go generator for the `x-ante` conformance vectors. It builds a celestia-app
v10.5.0 test app (`testutil.SetupTestAppWithGenesisValSet`, real keepers over
an in-memory store), funds a fixed set of accounts (secp256k1 with and
without stored pubkeys, secp256r1, ed25519, 2-of-3 / nested / ed25519-member
multisigs, a "validator operator" account, poor and unfunded accounts) and
runs generated transactions through `app.AnteHandler()` in FinalizeBlock,
CheckTx, ReCheckTx and simulation modes (and at genesis height 0).

Each vector records the pre-state delta on top of the scenario base state,
the tx bytes, and the ante result as `runTxAnte` reports it: code/codespace,
gas wanted/used (tx meter, or the pre-ante meter for `sdk.Context{}` returns,
`GasWanted = 0` after a recovered panic), priority, events and the state
writes (acc, bank, circuit, minfee, blob, feegrant stores).

Regenerate (fixtures are committed, zstd-compressed):

```
cd conformance/antevectors
GOTOOLCHAIN=go1.26.9 GOFLAGS=-mod=readonly go run . ../fixtures/antevectors
cargo test -p x-ante --test antevectors
```

`ANTE_JSON=1` also writes uncompressed JSON; `ANTE_VECTOR_FILTER=<substr>`,
`ANTE_SHOW=<n>` and `ANTE_VERBOSE=1` help when debugging the Rust replay.
secp256r1 keys are random per run, so regenerated files differ in bytes but
not in coverage.

Scenarios: `basic` (signatures, sequences, account numbers, chain id, memo,
timeouts, fees, payer/granter, signer/signature count mismatches, circuit
breaker, module-account and validator-address signers, param changes,
block max gas), `modes` (the basic set in CheckTx with several node min gas
prices, ReCheckTx and simulation), `gas` (out-of-gas at every consumption
point of eight base txs), `multisig`, `amino` (30 message types signed with
`SIGN_MODE_LEGACY_AMINO_JSON` and DIRECT), `keys` (ed25519, secp256r1),
`celestia` (nested messages, param filters, PFB min gas / blob shares,
MsgPayForFibre gas, IndexWrapper tx bytes) and `fuzz` (1200 seeded random
combinations).

`feegrant-revoke-gas` creates valid Basic and Periodic allowances at an
earlier block time, then runs after expiration without pruning. It sweeps
gas limits through fee deduction in FinalizeBlock, CheckTx and ReCheckTx,
including the revocation reads and deletes. A high-gas control returns the
ordinary expiry error; gas exhausted during revocation must instead unwind
to the ante handler's out-of-gas recovery. Vectors retain block time and
compare code/codespace, gas wanted/used, events and rollback of all ante writes.
Older vectors without time fields retain their original replay time of zero.
Regenerate just this scenario by appending `feegrant-revoke-gas` to the
generator command above. Use the pinned Go compiler: compiler-dependent
encoding behavior makes newer Go versions unsuitable substitutes.

`feegrant-boundaries` adds 225 pinned Go v10.5.0 ante vectors: Basic and
Periodic expiration at one nanosecond before, equal to and after block time;
fee budgets below/equal/above the fee; period reset equality, exactly one
period late, strictly later and zero duration; and outer, inner and later-message
filter denials. Each boundary runs raw, with one filter and with two nested
filters in FinalizeBlock, CheckTx and ReCheckTx. The signed transactions have
two messages, so filtering cannot be credited from the first message alone.
The generator checks valid grant setup, expected codes, exact-fee removal,
persisted reset timestamps and rollback before writing a fixture. The replay
compares code/codespace, gas, priority, events and state writes. Regenerate with
`feegrant-boundaries` as the final generator argument; this scenario rejects a
compiler other than Go 1.26.9. Its provenance receipt records module versions,
source commits and input/fixture hashes. These vectors exercise ante handling
of existing grants, not message execution, historical-v9 replay, block app hashes
or saturation/observation-window qualification.

Not covered by this vector suite: the full range of existing fee grants,
fibre stateful/signature checks in
CheckTx, and IBC redundant-relay checks beyond the nil client message.
Extension options cannot be covered: no `TxExtensionOptionI` implementation
is registered, so such txs fail decoding before the ante handler.

The M3 mainnet corpus (`conformance/fixtures/ante/mainnet-tx-corpus.json.zst`,
1,062 mainnet txs with Go app-v6 outcomes) is replayed through `x-ante` by
`crates/tier-3-app-modules/x/ante/tests/mainnet_corpus.rs`: signature
verification (direct and legacy amino-JSON), sequences and sig/decoding
error codes must match; fee and gas outcomes are not compared because v10
changed them (tx-size cost, utia-only fees, network minimum gas price,
circuit breaker, nested-message, fibre and param-filter decorators).
