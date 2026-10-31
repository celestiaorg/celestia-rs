# Current-release live transaction submission batch

This bounded Go v10.5.0 oracle batch records transaction submission only on an
isolated loopback devnet. It covers Cosmos gRPC and REST, celestia-core gRPC,
and Comet JSON-RPC async, sync and commit modes. Inputs include malformed and
ante-rejected transactions, a duplicate, an admitted execution failure,
successful bank sends and a pay-for-blob transaction. A deliberately canceled
commit request is followed through later inclusion, retry and another successful
commit to demonstrate continued capacity.

The malformed transport case is invalid base64 at the REST gateway, and the
semantic malformed case is a valid transaction envelope with no messages. Raw
malformed transaction protobuf is excluded: on this v10.5 standalone ABCI
topology a decoder exception closes the ABCI socket and ends the multi-case
batch. This batch records client timeout and disconnect behavior for Comet
commit, but does not record gRPC cancellation or slow-reader resource stress.
The message-family scope is bank sends plus one pay-for-blob transaction.

The capture preserves each raw transaction, exact HTTP bodies and headers,
protobuf request/response bytes and gRPC metadata, transaction queries, heights,
mempool state, blocks, ABCI frames and state snapshots. Metadata binds the
tagged app/core builds and every capture/state generator source, build receipt
and binary hash. This remains Go oracle evidence until the same inputs are
replayed through the Rust listeners.

The Rust production-listener replay is stateful rather than a collection of
standalone request vectors. The valid bank transactions are signed for account
number 1 on `rs-devnet-1` at sequences 0 through 9; the execution failure still
consumes sequence 9 during ante handling, and the PFB transaction is signed at
sequence 10. Those signed bytes cannot be replayed independently or reordered
without intentionally producing a different sequence error. The bad-signature,
future-sequence, low-gas and message-free inputs are independently replayable
rejection controls. The invalid-base64 case has no transaction bytes because it
is a REST transport-decoding control.

Celestia-core indexes the inner signed PFB transaction after unwrapping its
BlobTx envelope, so `capture/inputs/blob.tx` contains those indexed inner bytes,
not the original envelope sent by the CLI. The replay reconstructs the canonical
v0 BlobTx envelope from the frozen CLI command's namespace and data arguments
and asserts that it unwraps to the captured signed transaction and the expected
blob. The exact original envelope bytes were not retained and are excluded from
byte-for-byte input claims.

`crates/tier-5-node/celestia-rs/tests/live_tx_transports.rs` applies the first
ten admitted blocks with their recorded timestamps. It checks the height-10
status/header root (the post-height-9 application state) against block 10 and
checks the post-height-10 application root against the admitted successor
block 11. A freshly opened Go or Rust app intentionally checks transactions
with a height-zero header until its first live commit, so the replay waits for
one empty successor block before submitting the exact ordered bytes through
the production Comet JSON-RPC, celestia-core gRPC, Cosmos gRPC and REST
listeners. It compares
deterministic response bytes or fields, transaction hashes, CheckTx and
execution results, inclusion, cancellation/retry cleanup, the empty mempool
and final account sequence. One narrow normalization is explicit: after the
canceled-commit/retry sequence, repeated Rust runs have observed either the
recorded 50,319 CheckTx gas or 50,307 for the final cleanup bank send. Its
DeliverTx gas and all other recorded result fields match. The timing-dependent
12-gas cause remains unresolved. Live consensus block times and resulting
inclusion heights are not byte-parity inputs. The test does not check final
Go/Rust app-root or balance equality; its state claims are limited to the two
admitted pre-submission roots, recorded transaction outcomes, inclusion
ordering, empty final mempool and account sequence 11.

Regenerate with a fresh read-only mainnet observation:

```sh
MAINNET_OBSERVATION=/tmp/current-mainnet-observation.json \
  conformance/mainnet-rpc/live-tx/record.sh
```
