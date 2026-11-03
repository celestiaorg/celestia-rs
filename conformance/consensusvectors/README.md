# consensus vectors and Go verification

Go programs (celestia-core v0.42.3) that check the Rust consensus node
crates (`privval`, `nodestore`, `consensus`, `celestia-rs start`).

| Program | What |
|---|---|
| `privval` | Records Go `FilePV` behavior for a scripted sign sequence (fresh signatures, same-HRS re-signs, timestamp-only differences, regressions, conflicts, vote extensions): every step's error string, signatures, timestamp and the exact `priv_validator_state.json` bytes → `../fixtures/consensus/privval.json` (checked by `cargo test -p privval`). |
| `statejson` | Runs the official JSON and HexBytes decoders against the exported `FilePVLastSignState` field schema. Records acceptance, nil slices and canonical state JSON for byte-field types, base64/hex boundaries and duplicate fields → `../fixtures/consensus/privval-state-json.json` (checked by `cargo test -p privval`). This constructs no signer and does not prove persistence or signing behavior. |
| `wal` | Records Go's consensus WAL group (`libs/autofile.Group` + `consensus/wal.go`): a record stream written with a 3000-byte head limit (`wal.000` … `wal.005` + `wal`), corrupted variants (checksum, length, torn tails, a fresh head) and a total-size pruning run, with every `SearchForEndHeight` result, the records `catchupReplay` decodes after it and the error that stops it → `../fixtures/consensus/wal.json`. With `-rust <dir>` it reads a group written by the Rust `FileWal` (`WAL_VECTORS_RUST_DIR=<dir> cargo test -p consensus wal_vectors_write_rust_group -- --ignored`) and records Go's results as the `rust_written` case (checked by `cargo test -p consensus wal_`). |
| `verify` | Checks blocks dumped by `celestia-rs export-blocks` like a Go node would: decoding, `ValidateBasic`, block ids and part sets, header chaining, `VerifyCommit` of every last/seen commit, median block time, validator / params / results / app hash chaining. With `-app` it replays the chain through a fresh Go `celestia-appd` over ABCI (`InitChain`, `ProcessProposal` must accept every Rust-proposed block, `FinalizeBlock` app hashes, tx codes / gas and results hashes must match); with `-wal` it decodes the Rust consensus WAL with Go's `WALDecoder`. |

```bash
cd conformance/consensusvectors
GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run ./privval -out ../fixtures/consensus/privval.json
GOPATH=/tmp/claude-0/gopath GOFLAGS=-mod=mod GOTOOLCHAIN=go1.26.6 go run ./wal -out ../fixtures/consensus/wal.json
# end to end: Go genesis + Go-signed txs -> Rust node (restarted halfway) -> Go verification and replay
DEVNET_BIN=<dir with celestia-appd> DEVNET_PORT_OFFSET=12000 verify/run.sh 12 6
```

The state-JSON fixture uses Go **1.26.9** and core **v0.42.3**
(`a5537d84a008d566abc503d2ea41e30f2301403d`). Reproduce from this module:

```bash
GOTOOLCHAIN=go1.26.9 go run ./statejson -out ../fixtures/consensus/privval-state-json.json
```

Its provenance receipt records the generator and fixture hashes and the exact
official source-file hashes. The recorded run used an isolated, dependency-free
source subset: `libs/json/{decoder,encoder,structs,types}.go`, `libs/bytes/bytes.go`
and `libs/sync/sync.go`, copied unchanged from that commit into a minimal
`github.com/cometbft/cometbft` module. The generator was placed at `cmd/statejson`
in that module. Its local field schema matches the exported fields and tags of
`privval.FilePVLastSignState`; the private file path is outside JSON decoding.
The 39 cases include 15 accepts and 24 rejects, not a general decoder audit.
