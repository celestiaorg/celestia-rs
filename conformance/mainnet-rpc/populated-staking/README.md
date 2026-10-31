# Current-release populated staking slice

This capture is the first bounded current-release slice for issue
[#188](https://github.com/celestiaorg/celestia-rs/issues/188). It runs the
fixed staking/distribution transaction sequence against a
fresh, isolated celestia-app v10.5.0 and celestia-core v0.42.3 devnet. While
that same node is live, it records identical protobuf query inputs through
the direct ABCI socket, Comet JSON-RPC `abci_query`, Cosmos gRPC and REST.

The slice covers populated validators, delegations and rewards. Validator
pagination includes key and offset pages, totals, reverse order, an empty
page, an invalid cursor and a successful historical page. Every case is
pinned to a committed height. The committing block records the app hash for
that height; `genesis.json`, `abci.rec` and `blocks.bin.gz` retain the inputs
needed to replay and validate the same chain state.

The two account mnemonics, consensus/node keys, second validator public key
and genesis time are fixed test inputs. They are public devnet-only material
and must never be used for a funded network. Block times remain live inputs;
the capture binds their resulting genesis, ABCI stream and blocks by hash.

`capture/metadata.json` binds all capture sources and outputs to SHA-256
hashes. It also records the binary hash, complete build receipt and exact
source set for each recorder and state generator. The offline verifier parses
the block archive and state snapshots instead of treating them as opaque
hashed files. The app and core build manifests pin source commits, module
sums, compiler target and flags.

The read-only mainnet observation must precede capture by no more than 24
hours. Its raw `status` and `abci_info` bodies are hashed and parsed again to
check their chain, height and application-version agreement. The source-built
app uses the `ledger` tag; it is not a downloaded release binary or the
historical-version multiplexer.

This is a partial #188 artifact. Its explicit omissions include proof
validity, non-staking modules, invalid UTF-8, Any/JSON edge cases and later
staking lifecycle completion. Those require separate populated captures and
must not be inferred from this slice.

`crates/devnet-conformance/tests/populated_staking_transports.rs` treats this
Go capture as admitted, immutable provenance. It replays the recorded genesis
and ABCI stream into Rust, checks the recorded chain ID and the committed app
hash at the primary capture height 39, and submits each of the eleven exact
request byte strings at its recorded height through direct ABCI, Comet
JSON-RPC, Cosmos gRPC and REST. The
delegation-rewards validator list is the sole normalization, matching the
recorded Go map-order policy. The test also verifies a Rust-generated IAVL and
multistore proof against the admitted committed root. The capture contains no
Go proof-bearing request, so this establishes Rust proof validity but does not
claim Go/Rust proof-byte parity. The comparison covers ABCI response fields,
protobuf values, gRPC height metadata, REST status/JSON, and stable REST
content-type and height metadata. Live `Date` and `X-Server-Time` values and
transport-derived content length are excluded.

Regenerate with an unused port offset and a fresh observation produced by
`conformance/mainnet-rpc/observe-mainnet.py`:

```sh
MAINNET_OBSERVATION=/tmp/current-mainnet-observation.json \
DEVNET_PORT_OFFSET=6000 \
  conformance/mainnet-rpc/populated-staking/record.sh
```
