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

Regenerate with a fresh read-only mainnet observation:

```sh
MAINNET_OBSERVATION=/tmp/current-mainnet-observation.json \
  conformance/mainnet-rpc/live-tx/record.sh
```
