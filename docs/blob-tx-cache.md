# Blob transaction validation cache

Successful **New CheckTx** admission records the SHA-256 of the complete outer
BlobTx bytes together with the executing app version. ProcessProposal can then
skip only share-commitment recomputation for that exact transaction. Blob shape,
namespace, size, signer, SDK/ante checks, proposal limits and data-root checks
still run. New CheckTx itself always performs full validation. Failed requests
and Recheck never add entries; proposals do not populate the cache.

The app-local cache stores at most 10,000 digests and versions. FIFO eviction
bounds both its index and queue, and no blob payloads are retained. Eviction,
a restart or a different wire encoding causes full validation. A version change
cannot reuse a prior version's entry. PrepareProposal retains its existing
validation behavior. Set `CELESTIA_RS_BLOB_TX_CACHE=off` before starting a process
to disable admission and hits for diagnosis or differential checks.

## Validation on 2026-11-12

Recorded Go v9 and v10 fixtures exercise successful admission, failed signatures,
Recheck and warm ProcessProposal calls. Altering a cached blob is rejected even
when the proposal's square size and data root are recomputed. Unit tests also
cover version/wire-byte binding, duplicates, eviction and disabled operation.
All 29 offline app tests and app clippy checks pass.

[Retained comparison](evidence/blob-cache-2026-11-12/comparison.json): `blob`,
`v9-blob` and `ibc` each ran 300 generated blocks with 12 transactions per block
and seed 20261112, with the cache on and off. All six runs had zero consensus
mismatches against their v10.6.0/v9.0.8 Go oracle; log-only differences remain
counted separately. Each scenario's complete Rust response trace matched
byte-for-byte between settings. The same three scenarios compared 100 mutated
proposals per recorded height under both settings: 42,200 decisions, zero
Go/Rust mismatches.

A local recorded-proposal microbenchmark repeated an accepted warm proposal
3,000 times per measurement, alternating on/off twice at each version. The
proposal contains only 4,183 transaction bytes. The retained
[driver](evidence/blob-cache-2026-11-12/benchmark-driver.rs) can be built as an
`app` example; it accepts the fixture name and repetition count. Recorded times are in
[benchmark.jsonl](evidence/blob-cache-2026-11-12/benchmark.jsonl): approximately
0.66–0.71 seconds with caching and 0.68–0.71 seconds without. This small fixture
shows modest differences; it does not establish large-blob or production
throughput. No live validator was restarted or deployed for these checks.
