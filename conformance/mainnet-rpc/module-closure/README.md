# Frozen populated module comparisons

This package completes the bounded current-release module investigation in
[#188](https://github.com/celestiaorg/celestia-rs/issues/188). Ten fresh isolated
Go v10.5.0 campaigns populate staking/rewards, governance, IBC, authz/feegrant,
vesting, Hyperlane, fibre/validator addresses, forwarding/zkism, blob state and
invalid UTF-8 state. The retained signed public transactions and ABCI inputs
replay identically in Rust at **686 heights**. Every recorded app hash matches;
the primary query roots also match the retained block archives.

The comparison covers 259 method paths and **40,271 direct ABCI, 40,271 Comet
JSON-RPC, 36,467 Cosmos gRPC and 33,982 REST responses**. These are actual socket
listeners on both implementations. Missing REST bindings and ABCI-only methods
are not counted as transported requests. Rust's raw protobuf responses, Comet
JSON and REST bytes are retained beside the Go captures. The final runtime test
checks codes, diagnostics, response heights, protobuf payloads, keys and proof
operations, HTTP statuses, REST bodies and gRPC content type/height metadata.
The persistent API campaign separately checks grpc-go header/trailer placement.

## Oracle and reproducibility

The source-built oracle is celestia-app v10.5.0 at
`6013e758d5dceb75827ad191c8d739d7c43cfa04`, celestia-core v0.42.3 at
`a5537d84a008d566abc503d2ea41e30f2301403d`, SDK v0.52.12, with ledger
registration. The actual compiler is **Go 1.26.9**, differing from issue #188's
original Go 1.26.6 baseline. Each capture retains its fresh mainnet selection,
module/build receipts, exact helper binary hashes, compiler information, request
inventory, frozen tip, genesis, recorded inputs, blocks and state dumps.

The official remote signer is stopped while the application and core RPC remain
live. Capture starts only after three stable matched core/app-height samples;
ABCI Info must remain unchanged through the last request. This avoids block-time
and state drift across transports. The same inputs are replayed through each
Rust listener, with every intervening committed hash checked.

`evidence/collector-source.tar.gz` archives signed source commit `5695440` used
to build the capture helpers. The per-campaign `source_files` map additionally
records the capture workspace, including unrelated candidate edits made during
the batch; it is not a claim that every listed file was compiled into every
helper. Critical generator/recorder sources are checked against that archive.
`candidate-source.tar.gz` archives the final signed native test source at
`f51ce12`; the native-run receipt binds the exact test executable and toolchain. The current
test adds only lint annotations after that recorded run; the archived function
bodies are unchanged.
Neither archive requires an unmerged source commit to remain fetchable.

Genesis time and the first two public test accounts are fixed. Additional
workload keys and live block timestamps are retained as exact signed public
inputs. Replay is deterministic; regenerating a live chain need not reproduce
those random keys or timestamps. No runtime keyrings or private validator homes
are included. The existing checked-in devnet-only test keys are public fixtures.

Build the pinned oracles with `conformance/devnet/build-bins.sh`, the two Go
commands `local-signer` and `populated-matrix` in `conformance/rpccorpus/recorder`,
and `proofcheck` in this directory's independent Go module. Build the
`populated_requests` example with `cargo build -p devnet-conformance
--no-default-features --example populated_requests`. IBC additionally uses
`build-closure-relayer.sh`; UTF-8 uses `fuzz-diff craft`. Then take a fresh
`observe-mainnet.py` observation and run `record.sh <workload>` with
`DEVNET_BIN`, `MAINNET_OBSERVATION`, `POPULATED_REQUESTS_BIN` and isolated
`CELESTIA_APP_HOME`/`DEVNET_PORT_OFFSET` settings. `FUZZ_DIFF` selects the built
UTF-8 generator. Fibre's copied workload only adds `-buildvcs=false` to its Go
helper build; the relayer deliberately uses fixed gas to isolate IBC from the
separate Simulate contract.

## Historical, malformed and proof cases

The inventory includes key/offset pagination, totals, reverse order, terminal
and empty pages, malformed and invalid cursors, current and historical query
heights, future/missing-height failures, unknown methods, protobuf/Any edges and
invalid UTF-8 values in persisted module state. The exact cases, omissions and
height selections remain in each `requests.json`; this is sampled coverage,
not an exhaustive enumeration of every possible request or lifecycle history.

All **640 successful proof-bearing queries** have byte-identical Go/Rust IAVL
and multistore operations. The independent official `cosmos/ics23/go v0.11.0`
verifier accepts **616** and their roots match the correct committed height.
It rejects **24** involving empty-valued bank leaves or their non-membership
neighbors; Rust also rejects those same bytes. These are retained negative
validity results, not counted as valid proofs. No proof bytes or Go verdicts
were rewritten to obtain a passing comparison.

## Explicit normalization and exclusions

- Go diagnostic source annotations (` [file.go:line]`) are removed only for
  comparison; raw diagnostics remain in both artifacts.
- Go map order is canonicalized only for total delegation rewards, parameter
  subspaces, reflection v1 interface/implementer lists, and reflection v2 authn,
  codec, query-service and transaction descriptors. The manifest enumerates
  every case whose raw protobuf payload differs under these policies.
- JSON object key order/whitespace and equivalent empty protobuf-byte JSON
  encodings are immaterial. REST array order remains significant except the
  same explicitly listed map-order collections.
- Date, server timing, derived Content-Length and unrelated HTTP headers are
  outside this suite's comparison. Raw headers remain recorded. Error metadata
  phase placement is checked by the separate persistent grpc-go client.
- Consensus-backed node, transaction lookup/gas and block API methods, the empty
  ABCI envelope and internal P2P filter routes are inventoried exclusions. They
  belong to the runtime-identity, transaction, empty-query and persistent Comet
  campaigns. They are not silently counted as application-query matches.
- The oracle here is standalone. Official multiplexer pre-v10 history is covered
  by the separate current-release upgrade campaign. This suite does not claim
  TLS/proxy equivalence, production-duration resource bounds or universal
  provider equivalence.

The expanded check found and fixes two native transport differences: explicit
protobuf content subtypes now survive on success/error/reflection responses,
and SDK error height metadata precedes its status trailers as in grpc-go.

## Verification

```sh
python3 conformance/mainnet-rpc/module-closure/verify.py
cargo test -p devnet-conformance --no-default-features --test populated_module_matrix
cargo test -p grpc-transport --test query_headers
```

The Python verifier checks frozen artifacts, exact input/source/build bindings,
all case/transport inventories, recorded matched heights, block roots, proof
verdicts and the explicit payload-normalization inventory. The Rust test performs
the actual replay, cryptographic proof checks and socket comparisons. Set
`MODULE_CLOSURE_CAPTURE` to a new output directory to retain its raw responses;
ordinary CI does not rewrite the frozen artifacts.
