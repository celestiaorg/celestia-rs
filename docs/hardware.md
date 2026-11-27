# Target hardware

The target machine for a pruned celestia-rs mainnet validator has
**8 CPU cores, 16 GB of memory and a 500 GB NVMe SSD**. These are targets
that the budgets below are designed and tested against. Minimum requirements
are not established yet. Release binaries run on Apple silicon macOS
(`aarch64-apple-darwin`) and on `x86_64` and `aarch64` Linux.

| Resource | Target |
|---|---|
| CPU | 8 cores |
| Memory | 16 GB |
| Disk | 500 GB NVMe SSD |

These targets do not cover Fibre storage. A validator that turns on the
opt-in [Fibre server](operator-guide.md#fibre-server) must provision storage
for Fibre shards separately.

## Budgets

These budgets cover consensus validator operation. A bridge also stores a
header index and preserves transaction data that consensus prunes inside
the DA retention window. Size that storage for the retained block volume
and pruning settings. [Bridge retention](operator-guide.md#bridge-retention)

| Resource | Budget | Notes |
|---|---|---|
| Resident memory | ≤ 8 GB steady state, ≤ 12 GB peak | Leaves headroom for the OS and page cache. |
| Disk | ≤ 350 GB after 1 year | Validator-minimal pruning of blocks and app state, plus a fixed ≤ 1 GB for the consensus WAL (`data/cs.wal`, rotated and capped like Go). |
| Block processing | Finalize + commit a full-size block well inside the block time | Measured against the Go node in the post-mainnet benchmark phase. |

## State storage defaults

`celestia-rs abci --home <dir>` keeps application state in
`<dir>/data/application.rocksdb/` (RocksDB) — see ADR 0002 for the measurements
behind these defaults:

| Setting | Flag | Default | Budget impact |
|---|---|---|---|
| Backend | `--db-backend` | `rocksdb` (`redb` available) | RocksDB disk ≈ 1.1–1.4× live data (peaks 1.85× during compaction); redb 2.7–5× |
| Backend block cache | `--db-cache-mib` | 512 | Bounded: data, index and filter blocks all live in it; plus ≤ 256 MiB memtables (2 × 128 MiB, application database) |
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

- Default consensus pruning keeps recent state and blocks inside the
  evidence and unbonding window.
- Erasure coding, NMT hashing and signature verification must be
  multi-core-aware but must not assume more than 8 cores.
- No dependency may require x86-only intrinsics; everything must build for
  `aarch64-apple-darwin`.
