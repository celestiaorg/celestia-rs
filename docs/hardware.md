# Hardware requirements

These are the hardware requirements for celestia-rs on Mainnet and Mocha.
Release binaries run on Apple silicon macOS (`aarch64-apple-darwin`) and on
`x86_64` and `aarch64` Linux with glibc 2.35 or newer. Use a local NVMe SSD.

| Profile | CPU | Memory | Disk |
|---|---|---|---|
| Pruned full node | 8 CPU cores | 16 GB | 500 GB NVMe SSD |
| Validator (pruned) | 8 CPU cores | 16 GB | 500 GB NVMe SSD |
| Archive node | 8 CPU cores | 16 GB | 4 TB NVMe SSD, expandable |

Pruned uses `pruning = "default"` and `min-retain-blocks = 3000` in
`app.toml`. Archive uses `pruning = "nothing"`, `min-retain-blocks = 0` and
`tx_index.indexer = "kv"`, and keeps every block and app state from the
height it starts at (its state-sync snapshot). Archive disk grows with the
chain: 4 TB is a starting size, so plan to add storage. A Go archive of full
Mainnet history measured about 10 TB in the
[Celestia hardware guide](https://docs.celestia.org/operate/getting-started/hardware-requirements/).

## Evidence

The [2026-11-26 Mainnet runs](benchmarks/operator-sync-2026-11-26.md) on
8-vCPU `n2-standard-8` VMs measured celestia-rs at a peak of 6.0 GB of cgroup
memory during state sync and 4.1 GB over 600 blocks with its DA bridge on.
The [1,000-block runs](benchmarks/operator-block-range-2026-11-26.md)
measured a node home of 1.8 and 1.9 GiB after 1,000 blocks.

Keep **16 GB** for a dedicated pruned full node or validator with default
caches. The 8-vCPU benchmark hosts had 32 GB installed and snapshots disabled. Their sampled
peaks leave headroom. A sustained run with snapshots on and a hard 16 GB
limit would test that headroom directly.

Choose **32 GB** for additional headroom if you keep the larger catch-up
caches enabled (about 10–11 GiB for the node alone), serve heavy RPC traffic,
or share the host with other services. To justify raising the baseline,
measure a default-cache node on a 16 GB host through sync, snapshots and
compaction under representative traffic. Track host memory pressure, swap
activity and missed signing deadlines alongside process RSS.

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
| Backend block cache | `--db-cache-mib` | 512 | Bounded: data, index and filter blocks all live in it; plus ≤ 256 MiB memtables (2 × 128 MiB, application database). It fills within minutes of catching up, so RSS grows by about the size raised: `--db-cache-mib 4096` costs about 3.5 GiB more than the default (measured on mainnet `follow` nodes) |
| IAVL node cache | app.toml `iavl-cache-size` (`--node-cache-size` for `abci`) | 4,000,000 nodes | about 510 B per cached node (measured at mainnet height ~560k: 3.58M nodes took 1.43 GB more RSS than 0.78M) → about 2.0 GB when full; 781,250 (celestia-app's default) is the low-memory setting, ~6% slower at height ~560k |
| Pruning | `--pruning-keep-recent` / `--pruning-keep-every` / `--pruning-interval` | 100 / 0 / 1 | Retained history dominates live data; pruning every commit keeps commit p99 < 0.5 s at 600 tx/block |

With the default caches, a node's steady state is about 2.0 GB for the node
cache when full, 512 MiB of block cache, up to 256 MiB of memtables and about
1.5 GB for the rest of the process: about 4.3 GB, inside the 8 GB budget.
Homes made by an older `init`, or migrated from Go, keep `iavl-cache-size =
781250` in `app.toml` until the operator raises it; the node logs a hint
at startup.

The catch-up settings in the operator guide ("Block sync mainnet from
genesis (P2P)", "Speed":
`iavl-cache-size = 8000000` and `--db-cache-mib 4096`) add about 2.0 GB
and 3.5 GiB to that, so a catching-up node uses about 10-11 GiB. That is
inside a 16 GB machine but above the 8 GB steady-state budget: return to
the defaults once the node has caught up.

Measured with these settings at a heavy synthetic load (600 txs ≈ 2,200
IAVL writes per block): commit p50 0.30 s / p99 0.48 s / max 0.64 s and
peak RSS 1.25 GiB for the storage layer — well inside the 8 GB budget and the
block time. Raise `--db-cache-mib` (e.g. 2048) on machines that serve
queries; lower `--pruning-keep-recent` to shrink disk further.

The binary uses mimalloc as its global allocator (default feature
`mimalloc`; RocksDB's C++ allocations go through it too).
`celestia-rs version --long` prints it. Over 100,000 mainnet blocks
(heights 558k-658k) it executes blocks about 6% faster than jemalloc,
with about 3-4% more peak and end memory (`docs/benchmarks/memory.md`). On glibc malloc the node's RSS
settled at 4-5x its live heap. jemalloc remains a build option
(`--no-default-features --features rocksdb,jemalloc`), and
`--no-default-features --features rocksdb,memstats` adds jemalloc heap
gauges and its heap profiler for leak hunting.

RocksDB keeps up to 512 SST files open: raise the macOS shell default
(`ulimit -n 4096`) before starting the node.

## Implications

- Default consensus pruning keeps recent state and blocks inside the
  evidence and unbonding window.
- Erasure coding, NMT hashing and signature verification must be
  multi-core-aware but must not assume more than 8 cores.
- No dependency may require x86-only intrinsics; everything must build for
  `aarch64-apple-darwin`.
