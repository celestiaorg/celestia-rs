# Query corpus capture integrity

The query recorders replay a scenario into an isolated Go application and then
capture ABCI and REST requests. Existing corpora identify their original release;
replaying an old workload against a newer release does not update the provenance
of the old answers.

`record_queries` and `record_comet` fail if a transport, protobuf, exception or
Flush response fails, including while discovering objects for follow-up requests.
A failed discovery request invalidates the entire recording rather than silently
reducing its coverage. No recording failure is replaced by a successful default
`ResponseQuery`.

A decoded ABCI response envelope can contain no Query value. New recordings
preserve this as `missing_response: true`; the accompanying `response` placeholder
has no response semantics. An actual present, default-valued `ResponseQuery` is
still a successful Query and does not have that marker. App replay currently
rejects the missing-response case until it has an explicit transport comparison;
it must not count it as parity with a successful default Query.

The older recorders used `unwrap_or_default()` around ABCI queries. Their empty
P2P-filter results, especially `/p2p/filter/other/x`, therefore cannot establish
success or transport compatibility. Keep those historical artifacts as background
regression inputs and collect fresh responses with the hardened recorder. See
[issue 186](https://github.com/celestiaorg/celestia-rs/issues/186) for the separate
public HTTP/JSON-RPC/WebSocket behavior investigation.

To build the recorders without the native RocksDB dependency, use
`cargo build --locked -p devnet-conformance --no-default-features --examples`.
The default features continue to enable RocksDB.
