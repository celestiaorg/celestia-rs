# antevectors

Go generator for the `x-ante` conformance vectors. It builds a celestia-app
v10.4.0 test app (`testutil.SetupTestAppWithGenesisValSet`, real keepers over
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
GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod go run . ../fixtures/antevectors
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

Not covered (the Go keepers are not ported; `x_ante::StoreDeps` returns
`ErrNotSupported`): existing fee grants, fibre stateful/signature checks in
CheckTx, and IBC redundant-relay checks beyond the nil client message.
Extension options cannot be covered: no `TxExtensionOptionI` implementation
is registered, so such txs fail decoding before the ante handler.

The M3 mainnet corpus (`conformance/fixtures/ante/`, replayed by the
v6-era `crates/tier-3-app-modules/ante` prototype) is unchanged. It targets
app v6 and differs from v10 in: the tx-size cost (auth param vs. the
versioned constant), the fee checker (no utia-only rule or minfee network
minimum in v6's prototype), the absent circuit-breaker, nested-message,
fibre and param-filter decorators, and the v0.52 x/tx sign-mode handlers.
