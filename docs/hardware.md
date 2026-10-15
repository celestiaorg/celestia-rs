# Target hardware

The reference machine for running a celestia-rs mainnet validator is a
**Mac Mini M4 (base): 10-core CPU, 16 GB unified memory, 512 GB SSD**,
macOS on `aarch64-apple-darwin`. Linux `x86_64`/`aarch64` are supported for
development and CI.

## Budgets

| Resource | Budget | Notes |
|---|---|---|
| Resident memory | ≤ 8 GB steady state, ≤ 12 GB peak | Leaves headroom for the OS and page cache. |
| Disk | ≤ 350 GB after 1 year | Validator-minimal pruning of blocks and app state. |
| Block processing | Finalize + commit a full-size block well inside the block time | Measured against the Go node in the post-mainnet benchmark phase. |

## State storage defaults

`celestia-rs abci --home <dir>` keeps application state in
`<dir>/data/application.rocksdb/` (RocksDB) — see ADR 0002 for the measurements
behind these defaults:

| Setting | Flag | Default | Budget impact |
|---|---|---|---|
| Backend | `--db-backend` | `rocksdb` (`redb` available) | RocksDB disk ≈ 1.1–1.4× live data (peaks 1.85× during compaction); redb 2.7–5× |
| Backend block cache | `--db-cache-mib` | 512 | Bounded: data, index and filter blocks all live in it; plus ≤ 128 MiB memtables |
| IAVL node cache | `--node-cache-size` | 1,000,000 nodes | ~200–300 B per node → ≤ ~300 MiB |
| Pruning | `--pruning-keep-recent` / `--pruning-keep-every` / `--pruning-interval` | 100 / 0 / 1 | Retained history dominates live data; pruning every commit keeps commit p99 < 0.5 s at 600 tx/block |

Measured with these settings at a heavy synthetic load (600 txs ≈ 2,200
IAVL writes per block): commit p50 0.30 s / p99 0.48 s / max 0.64 s and
peak RSS 1.25 GiB for the storage layer — well inside the 8 GB budget and the
block time. Raise `--db-cache-mib` (e.g. 2048) on machines that serve
queries; lower `--pruning-keep-recent` to shrink disk further.

The binary uses jemalloc as its global allocator (default feature
`jemalloc`; RocksDB's C++ allocations go through it too on Linux). On
glibc malloc the node's RSS settled at 4-5x its live heap; with jemalloc
RSS stays within ~1.5x of the heap (`docs/benchmarks/memory.md`).
`--features memstats` adds heap gauges and jemalloc's heap profiler for
leak hunting.

RocksDB keeps up to 512 SST files open: raise the macOS shell default
(`ulimit -n 4096`) before starting the node.

## Implications

- Default pruning keeps only recent state and blocks inside the evidence /
  unbonding window; archival mode is not a goal.
- Erasure coding, NMT hashing and signature verification must be
  multi-core-aware but must not assume more than 10 cores.
- No dependency may require x86-only intrinsics; everything must build for
  `aarch64-apple-darwin`.
