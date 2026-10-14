# ADR 0002 — Persistent state storage

**Status:** Accepted (2026-10-14).

## Context

The `iavl` crate is an in-memory versioned AVL tree whose hashes match Go
IAVL v1 (verified for all 27 v10 stores). A mainnet node needs:

- state that survives restarts and does not need to fit in RAM (target:
  Mac Mini M4, 16 GB — `docs/hardware.md` budgets ≤ 8 GB resident),
- historical versions for queries and snapshots, with pruning,
- atomic commit of all stores per height (crash consistency),
- state-sync snapshot export/import (S3).

The on-disk format does **not** need to match Go's (only hashes, proofs and
ABCI-visible behavior are consensus-relevant), which leaves us free to pick
a layout that suits Rust.

## Decision

1. **Node store.** Persist IAVL nodes keyed by `(version, nonce)` — the IAVL
   v1 node-key scheme — so a tree version is reachable from its root key and
   unchanged subtrees are shared across versions. Nodes are loaded lazily
   and cached in a bounded LRU; the working tree keeps only dirty nodes in
   memory.
2. **Backend.** A small `KvBackend` trait (get, ordered range, atomic write
   batch) with two production implementations — **RocksDB** (default) and
   **`redb`** (pure Rust, ACID, copy-on-write B-tree, no C/C++ toolchain) —
   plus an in-memory implementation for tests. All stores of a height plus
   the multistore commit info are written in one atomic batch →
   crash-consistent commits. *Amended 2026-10-14:* the original decision
   made redb the only backend; the long-run measurements below showed redb's
   file at 2.7–5× live data with rising write amplification, so RocksDB
   became the default and redb the opt-in pure-Rust alternative.
3. **Pruning.** Keep-recent + keep-every (SDK semantics). Orphaned nodes are
   recorded per version and deleted when their version falls out of the
   retention window. Validator default: keep recent 100 versions plus
   snapshot heights, pruned after every commit (`interval = 1`).
4. **Snapshots.** Export/import uses the SDK snapshot format (IAVL export
   nodes in post-order) so Rust and Go nodes can state-sync from each other.
5. **Concurrency.** Commit holds the write lock; queries read committed
   versions through immutable snapshots of the tree (redb read
   transactions), never the working tree.

## Consequences

- `iavl::Tree` gains a storage-backed mode; hashing code is shared so the
  existing conformance tests keep guarding hash parity.
- `sdk::MultiStore` commits through the backend; `devnet-conformance`
  replays run against the in-memory backend and every persistent backend
  (RocksDB, redb).
- A restart test (commit N blocks, reopen, continue replay, compare app
  hashes) becomes part of the S-series acceptance.

## Implementation (2026-10-14)

| Piece | Where |
|---|---|
| `KvBackend` (get, ordered/reverse range, atomic `WriteBatch`, sync, `stats`); `RocksBackend` (`rocksdb =0.24.0` / RocksDB 10.4, `rocksdb` feature, default column family, one `WriteBatch` per batch); `RedbBackend` (redb `=2.6.4`, one table, one write txn per batch); `MemBackend`; `FaultyBackend` (crash simulation); `BackendKind` + `open_backend` (refuses to start on a data dir written by the other backend) | `crates/tier-2-state-and-store/kvdb` |
| Storage-backed IAVL: `NodeDb` (per-store prefix), shared LRU `NodeCache`, `ImmutableTree` per version, orphans, `prune`, `Exporter`/`Importer` | `crates/tier-2-state-and-store/iavl` |
| Atomic commit, `load_latest_version`, pruning policy, metadata, snapshot `export`/`Restorer` | `crates/tier-2-state-and-store/multistore` |
| SDK snapshot format 3 (delimited `SnapshotItem`s, zlib 7, 10 MB chunks) + `Manager` | `crates/tier-2-state-and-store/snapshots` |
| `App::open(home)` (`AppOptions.db: DbOptions`), ABCI snapshot calls; `celestia-rs abci --home` with `--db-backend`, `--db-cache-mib`, `--node-cache-size`, `--pruning-*` | `crates/tier-4-runtime/app`, `crates/tier-5-node/celestia-rs` |
| Long-run storage benchmark (`store_bench_mainnet`) | `crates/tier-2-state-and-store/multistore/examples` |

Layout in the single key space (one redb table / the RocksDB default column
family; Go-like prefixes; the format is ours):
`s/k:<store>/n‖nodekey` → node, `s/k:<store>/r‖version` → root reference,
`s/k:<store>/o‖to-version‖nodekey` → orphan record, `s/<version>` →
`CommitInfo` proto, `s/latest` → version, `m/<key>` → app metadata (chain
ID). Node keys are `version‖nonce` big-endian. A parent stores each child's
node key **and hash, height and size**, so re-hashing a dirty path and AVL
rebalancing never load an unchanged sibling; a node's own hash comes from the
reference used to reach it. One `KvBackend::write` per height carries every
store's new nodes, root and orphan records plus the commit info and
`s/latest`; pruning runs in a separate batch afterwards and is idempotent.

An orphan record `o‖t‖nk` says node `nk` is live in versions `nk.version..=t`.
Deleting version `t` moves the record to the newest retained version
`≥ nk.version` and `< t`, or deletes the node if there is none — so a retained
version never loses a node (tests check exact hashes/contents of retained
versions and that every stored node is reachable from a retained root).

Snapshots are taken synchronously after Commit when `height % interval == 0`
and served from `<home>/data/snapshots/<h>/3/<chunk>`. Restores verify each
chunk hash, stage chunks on disk, then import; roots and commit info are
written in one batch at the end, so an interrupted restore leaves an empty
store (leftovers are wiped by the next restore). `conformance/devnet/statesync.sh`
proves Go → Rust and Rust → Go state sync plus restart on a live devnet.

## Backend measurements (2026-10-14)

`cargo run --release -p multistore --features rocksdb --example
store_bench_mainnet -- backend=<redb|rocksdb> ...` simulates a validator's
write mix: genesis of 200k accounts + balances, 100 validators and 20k
delegations; then per block 600 txs (sender account + sender/recipient
balances, 1% hot senders carry half the load, 10% new recipients; 3%
delegations with distribution period churn, 1% unbondings that mature after
1,000 blocks), BeginBlock distribution updates for 100 validators, two IBC
client updates (consensus states expire after 2,000 blocks) and a 32 KB
governance entry every 200 blocks (deleted 500 blocks later): ~2,200 IAVL
writes and ~35k new IAVL nodes per block. Keep-recent 100, fsync on every
commit, Linux, 4 cores. *Live* is the sum of key + value bytes in the store
(every retained version's nodes, orphan records, roots, commit infos); with
keep-recent 100 the 100 retained versions dominate it, not the latest tree.

### Disk vs live data over 2,000 (RocksDB) / 1,100 (redb) blocks, pruning interval 10

| block | live MiB | redb file MiB | redb allocated MiB | redb file/live | redb WA | RocksDB MiB | RocksDB/live | RocksDB WA |
|---|---|---|---|---|---|---|---|---|
| 101 | 363 | 1,028 | 815 | 2.84 | 2.3 | 272 | 0.75 | 2.4 |
| 301 | 379 | 1,028 | 820 | 2.71 | 4.0 | 410 | 1.08 | 3.1 |
| 501 | 391 | 1,028 | 842 | 2.63 | 4.4 | 542 | 1.39 | 3.4 |
| 701 | 401 | **2,056** | 861 | **5.12** | 4.6 | 526 | 1.31 | 3.5 |
| 1,001 | 417 | 2,056 | 895 | 4.93 | 4.8 | 514 | 1.23 | 3.7 |
| 1,101 | 423 | 2,056 | 907 | 4.86 | 4.9 | 464 | 1.10 | 3.7 |
| 1,501 | 442 | | | | | 497 | 1.12 | 3.8 |
| 1,801 | 456 | | | | | 845 | 1.85 | 3.8 |
| 2,001 | 466 | | | | | 648 | 1.39 | 3.8 |

WA = bytes the process wrote to the device (`/proc/self/io`) ÷ key + value
bytes handed to the backend. RocksDB oscillates between 1.03× and 1.85× live
(compaction output exists next to its inputs until they are deleted; mean
1.26× over blocks 300–2,000) with no upward trend.

**Why redb's file is several times live data — and whether it plateaus.**
Freed pages *are* reused: allocated pages track live data at a constant
~2.15× (815 → 907 MiB while live grew 363 → 423 MiB), so there is no
unbounded leak. Read transactions do not pin pages (the cached reader is
dropped before every write and range iterators are short-lived), and
`Durability::Immediate` lets each commit free the pages of the previous one
(`Durability::None` would defer every free to the next durable commit, so it
is not an option). The factor comes from:

1. **File growth policy.** Below 4 GiB redb doubles the file (1 → 2 GiB at
   block ~700 with 861 MiB allocated) and only trims a free tail, so file
   size is a step function: up to 2× allocated on top of the 2.15×. Above
   4 GiB it grows by 4 GiB regions. The earlier "129 MB for 17 MB live" and
   "257 MB for 46 MB live" readings were exactly such doubling steps.
2. **Page fill factor.** IAVL node keys are `version‖nonce`, so each store's
   node key space is appended at the right edge (redb splits full leaves in
   half → ~50% fill) and pruned unevenly (nodes still referenced by the
   latest version survive scattered among deleted ones; redb merges a leaf
   only below 1/3 full). Leaves therefore sit at ~40–50% → ~2.15× live.
3. **Copy-on-write churn.** Every commit rewrites each touched 4 KiB leaf and
   its path; pruning's scattered deletes touch a different leaf per few
   entries. Write amplification rises with database size (2.3 → 4.9), and to
   7.3 with per-commit pruning.

`Database::compact()` (exposed as `RedbOptions::compact_on_open`; it needs
exclusive access and no live read transaction, so only at open) moves pages
down and truncates the file, but cannot go below the ~2.15× allocated
footprint, and redb only trims when at least half of the last region is
free. Measured: compacting at blocks 150 and 300 took 31 s and 43 s and left
the file at 1,028 MiB (820 MiB allocated) — a maintenance tool, not a fix. Table/key layout changes cannot fix the fill
factor without giving up IAVL's version-ordered node keys, which pruning and
export rely on.

### Commit latency and RSS (400 blocks, run alone; node cache 1M, backend cache 512 MiB)

| backend | pruning interval | txs/block | commit p50 | commit p99 | commit max | WA | peak RSS |
|---|---|---|---|---|---|---|---|
| redb | 10 | 600 | 356 ms | 3,618 ms | 4,467 ms | 4.2 | 1,055 MiB |
| redb | 1 | 600 | 860 ms | 1,256 ms | 1,320 ms | 7.3 | 1,015 MiB |
| RocksDB | 10 | 600 | 280 ms | 1,948 ms | 2,631 ms | 3.3 | 1,286 MiB |
| **RocksDB** | **1** | 600 | 296 ms | 483 ms | 639 ms | 3.2 | 1,252 MiB |
| **RocksDB** | **1** | 100 | 76 ms | 319 ms | 599 ms | 2.4 | 1,059 MiB |

Commit includes IAVL hashing and node encoding (~110 ms of the 600-tx
commit on an in-memory backend) and pruning. With `interval = 10` every
tenth commit deletes ten versions at once (~700k deletes), which is where
the multi-second spikes come from; per-commit pruning does the same total
work in small steps.

### Decision

**RocksDB is the default backend**; redb stays available
(`--db-backend redb`, and the only backend in a `--no-default-features`
build, which needs no C/C++ toolchain).

| criterion | target | RocksDB | redb |
|---|---|---|---|
| steady-state disk / live | ≤ ~2–3× | 1.1–1.4× (peaks 1.85×) | 2.7–5.1× (allocated alone 2.15×) |
| commit latency, heavy load | well under the 6 s block | p99 0.48 s, max 0.64 s | p99 1.3–3.6 s, max 4.5 s |
| write amplification | | 3.2–3.8, flat | 4.2–7.3, rising |
| RSS | ≤ 8 GB | 1.05–1.3 GiB | ~1.0 GiB |

RocksDB settings (`kvdb::RocksOptions`): one LRU block cache holding data,
index and filter blocks (`--db-cache-mib`, default 512), 10-bit bloom
filters, 16 KiB blocks, 2 × 64 MiB memtables, WAL capped at 128 MiB,
dynamic level sizing, no compression above the bottommost level and zstd
there (IAVL nodes are mostly hashes), `max_open_files = 512`, fsync of the
WAL on every commit. `rocksdb` crate pinned `=0.24.0` (RocksDB 10.4.2; zstd
only, libclang loaded at build time by bindgen). In dev builds
`librocksdb-sys` is compiled with `-O2` and no debug info
(`[profile.dev.package.librocksdb-sys]`). The RocksDB backend's range
iterator owns an `Arc<DB>` and refills 1,024 entries at a time; every
caller iterates from the single committing thread, so chunk boundaries are
not observable.

Node defaults (`celestia-rs abci --home`): `--db-backend rocksdb`,
`--db-cache-mib 512`, `--node-cache-size 1000000`,
`--pruning-keep-recent 100 --pruning-keep-every 0 --pruning-interval 1`.
State lives in `<home>/data/application.rocksdb/` (RocksDB) or
`application.redb` (the extension is the backend name, so it never collides
with Go's goleveldb `application.db` in a shared home); `celestia-rs start`
keeps its block and state stores the same way (`blockstore.*`, `state.*`,
64 MiB cache each). Opening a database written by the other backend is
refused instead of silently starting from an empty state.
