# B1 benchmarks: Go celestia-appd vs celestia-rs

> **Status (2026-10-15): Linux numbers only. The reference numbers for B1
> are pending a run on the target Mac Mini M4 (16 GB / 512 GB,
> `docs/hardware.md`); the commands below reproduce everything there.**
> The Linux run below was taken on a shared, oversubscribed 4-vCPU cloud
> VM (load average 10-15 from other jobs during the run): absolute
> latencies are inflated and noisy, and only the Go/Rust ratios of
> measurements taken back to back are meaningful.

Both nodes are release builds on the same machine and see the same
inputs: Go celestia-appd v10.4.0 / celestia-core v0.42.3 (go1.26.6,
`conformance/devnet/build-bins.sh`) and celestia-rs (`cargo build
--release -p celestia-rs`, default features: RocksDB, jemalloc).

## Method

Scripts: `conformance/bench/`. Memory (long single-lifetime soak, allocator): `memory.md`.

1. **`gen-chain.sh` — a recorded heavy chain.** One Go validator
   (celestia-appd as a standalone ABCI app behind celestia-core, with
   `abci-recorder` in between, as `conformance/devnet/devnet.sh`) is loaded
   by `bench_load` (`crates/tier-5-node/celestia-rs/examples/bench_load.rs`,
   txs signed locally from fixed seeds): every height up to 600 bank sends
   to fresh addresses (each creates an account, so state grows) + up to 180
   small PFBs (100 B-1 KB), and every 25 heights 4 big PFBs alternating
   4 x 1.9 MB (128x128 square) and 4 x 7.3 MB (256x256 square, the v10
   `gov_max_square_size`). celestia-app caps a block at 600 SDK messages +
   200 PFBs (`appconsts.MaxSDKMessages` / `MaxPFBMessages`), so ~780
   txs/block is the protocol maximum, not 1,000; blocks averaged 500 txs
   because the generator shares the block time with signing.
   Result: 513 heights, 257,456 txs, 0.35 GiB of block data, 21 blocks over
   2 MiB (up to 27.9 MiB), a 2.3 GB `abci.rec` with every ABCI frame, and
   the validator's homes (block store, snapshots every 100 heights).
2. **`run.sh` phase A — app-only replay.** `abci_bench`
   (`crates/devnet-conformance/examples/abci_bench.rs`) replays the
   recording into a fresh app over the ABCI socket — `celestia-appd start
   --with-comet=false` or `celestia-rs abci --home` — with one driver and
   one transport for both: every request is sent with its own Flush and
   timed to its response; runs of CheckTx are pipelined as celestia-core's
   mempool does and timed as a batch; PrepareProposal / ProcessProposal are
   read-only and repeated (3x, 10x for blocks over 2 MiB) for latency
   distributions; every FinalizeBlock app hash is checked against the
   recording (0 mismatches for both). Both apps prune state identically
   (keep 100 versions, prune every 10 heights).
3. **Phase B — block sync.** The recorded validator is restarted (it keeps
   producing empty blocks); a fresh Go full node, then a fresh Rust full
   node (`celestia-rs start`), block-syncs the 513 heights from it.
4. **Phase C — startup.** Each synced full node is stopped (SIGTERM) and
   restarted: time until RPC answers and until it commits a new height
   (store open + handshake + catch-up of the few heights it missed).
5. **Phase D — state sync.** A fresh full node restores the validator's
   latest snapshot (light client over the validator's RPC, trust height =
   snapshot - 10) and then follows.

RSS (`VmRSS` / peak `VmHWM`) and `du` of the data directory are taken at the
end of each phase.

## Reproduce (Mac Mini or any machine)

```bash
conformance/devnet/build-bins.sh                     # Go celestia-appd, cometbft, abci-recorder
cargo build --release -p celestia-rs --example bench_load
cargo build --release -p devnet-conformance --example abci_bench
cargo build --release -p celestia-rs
ulimit -n 4096                                       # macOS: RocksDB keeps up to 512 SSTs open
export DEVNET_BIN=$PWD/target/devnet-bin DEVNET_PORT_OFFSET=21000 BENCH_DIR=$PWD/target/bench
conformance/bench/gen-chain.sh                       # ~25 min, ~4.1 GB (recording + homes)
conformance/bench/run.sh                             # ~1.5 h; results in $BENCH_DIR/results/results.md
```

`BENCH_HEIGHTS` / `BENCH_BIG_EVERY` size the chain, `BENCH_PHASES="A"`
runs a subset, `CELESTIA_RS_ALT=<binary>` adds a second Rust build to
phase A (e.g. `--no-default-features --features rocksdb`, the system
allocator). Record `sysctl -n machdep.cpu.brand_string`, `sysctl hw.ncpu
hw.memsize` and keep the machine otherwise idle.

## Machine (Linux run)

`nproc` 4, Intel(R) Xeon(R) Processor @ 2.10GHz (cloud VM), 15.7 GiB RAM
(14.3 GB cgroup limit shared with other jobs), Linux 6.18, rustc 1.99.0,
go1.26.6. Load average during phase A: 11-15 (another agent compiling and a
memory soak running).

## Results (Linux, shared VM; Mac Mini numbers pending)

Rust/Go < 1 means Rust is faster or smaller. "Max-size squares" are the 21
blocks over 2 MiB (128x128 and 256x256 squares, up to 27.9 MiB); "light"
blocks are the other 492 (up to ~800 txs, ~0.35 MiB).

### A. App-only replay (513 heights, 257,456 txs)

| metric | Go celestia-appd | celestia-rs | Rust/Go |
|---|---|---|---|
| FinalizeBlock+Commit p50 | 1059 ms | 589 ms | 0.56x |
| FinalizeBlock+Commit p99 | 15736 ms | 1804 ms | 0.11x |
| FinalizeBlock+Commit, max-size squares p50 / p99 | 722 / 5058 ms | 406 / 1246 ms | 0.56x / 0.25x |
| FinalizeBlock p50 / p99 | 795 / 4912 ms | 405 / 1090 ms | 0.51x / 0.22x |
| Commit p50 / p99 | 191 / 14569 ms | 161 / 1063 ms | 0.84x / 0.07x |
| Commit at pruning heights (every 10th) p50 | 9589 ms | 707 ms | 0.07x |
| **PrepareProposal, max-size squares p50** / p99 (210 samples) | 1188 / 4913 ms | **1297** / 2935 ms | **1.09x** / 0.60x |
| PrepareProposal, 256x256 squares p50 (6 blocks x 10) | 1541 ms | 1581 ms | 1.03x |
| ProcessProposal, max-size squares p50 / p99 | 1303 / 4828 ms | 1247 / 2460 ms | 0.96x / 0.51x |
| PrepareProposal, light blocks p50 / p99 | 284 / 1980 ms | 224 / 703 ms | 0.79x / 0.36x |
| ProcessProposal, light blocks p50 / p99 | 253 / 1934 ms | 220 / 648 ms | 0.87x / 0.33x |
| execution throughput (FinalizeBlock+Commit only) | 0.45 blocks/s, 225 tx/s | 1.60 blocks/s, 802 tx/s | 3.6x faster |
| CheckTx (new txs) | 971 tx/s | 2007 tx/s | 2.1x faster |
| CheckTx (recheck) | 2127 tx/s | 3374 tx/s | 1.6x faster |
| app RSS at end (peak) | 2093 MB (4544) | 1473 MB (2513) | 0.70x |
| **app state on disk at end** | 329 MB | **549 MB** | **1.67x** |
| app hash mismatches vs the recording | 0 / 513 | 0 / 513 | |

Go's commit p99 is dominated by IAVL pruning (keep 100, every 10 heights:
p50 9.6 s at pruning heights vs 175 ms otherwise); with Go's default
`pruning = "default"` (keep 362,880) that cost moves out of the window of
this run, so compare the p50 rows first.

### B-D. Full nodes (Go `celestia-appd start` vs `celestia-rs start`)

| metric | Go | Rust | Rust/Go |
|---|---|---|---|
| B. block sync of 513 heights from the Go validator | 467.7 s (1.1 blocks/s) | 396.6 s (1.3 blocks/s) | 0.85x |
| B. RSS after block sync (peak) | 1860 MB (2225) | 1264 MB (1625) | 0.68x |
| B. data/ after block sync | 1295 MB | 960 MB | 0.74x |
| **C. restart: RPC answering** | 1.18 s | **2.10 s** | **1.78x** |
| C. restart: RSS | 297 MB | 188 MB | 0.63x |
| D. state sync of snapshot 900: restored | 56.4 s | 13.2 s | 0.23x |
| D. state sync: first block after the snapshot | 60.3 s | 16.3 s | 0.27x |
| D. RSS / data/ after state sync | 1012 MB / 157 MB | 452 MB / 150 MB | 0.45x / 0.96x |

Phase C also records the time to the first height above the validator's
height at restart (Go 7.9 s, Rust 13.1 s), but it is not comparable: the
validator kept producing empty blocks, and the Rust node was restarted ~400
heights behind it (it block-synced them at 18 blocks/s) while the Go node
was restarted far closer to the tip; `run.sh` should stop the validator
before C on the Mac run. Both full nodes ran their own default pruning
(`pruning = "default"` in app.toml: Go keeps 362,880 versions, the Rust node
maps it to keep 100 / every height).

### Where Rust is slower, and the top 3 optimization opportunities

1. **PrepareProposal on max-size squares (1.03-1.09x Go at p50).** The
   extended data square is computed on one thread with one `Vec<u8>` per
   share (`square::erasure::compute_extended_data_square`: 4 x width^2
   allocations, rows then columns encoded sequentially) and the 4 x width
   NMT roots are hashed sequentially (`ExtendedDataSquare::row_roots` /
   `column_roots`). A 256x256 square is 262,144 shares of 512 B. Encode
   rows / columns and hash roots in parallel (bounded to the M4's 10 cores),
   use one contiguous buffer for the square, and let ProcessProposal reuse
   the EDS / DAH the proposer just built for the same block (keyed by the
   data hash) instead of recomputing it.
2. **Startup: RPC up after 2.1 s vs 1.2 s.** Of that, ~2.0 s is opening
   the application state (RocksDB + loading the IAVL roots and node cache of
   every store) before the RPC server, P2P and the handshake start; the
   handshake itself and peer dial are <1.5 s. Open the per-store trees
   lazily or in parallel, and start RPC (status) before the app finishes
   loading.
3. **App state on disk 1.67x Go.** RocksDB is configured with no
   compression above the bottommost level (`kvdb::rocks`: `DBCompressionType::None`
   + bottommost Zstd) and pruning leaves tombstones until compaction reaches
   them; goleveldb compresses every level with Snappy. Use LZ4 on L1+ (IAVL
   values are not all hashes: accounts, balances, blobs' metadata), and
   `compact_range` the pruned key ranges (or enable periodic compaction) so
   space is reclaimed after pruning. Note the full node's whole `data/` is
   already 0.74x Go's (smaller block store), so this is about app state only.

Everywhere else Rust is at or below Go: block execution 2-3.6x faster,
CheckTx 1.6-2.1x, state sync 4x faster, and 30-55% less memory.

## Tuning after B1 (2026-10-15, Linux shared VM)

Same 4-vCPU shared VM as above (load average 4-7 from other jobs during
these runs), so again compare ratios of runs taken back to back. Two
tools that need no Go binaries or recorded chain:

- `cargo run --release -p square --example eds_bench -- <iters> <widths…>`:
  erasure extension + DAH of synthetic squares. It prints a digest of the
  row and column roots; a Go program using rsmt2d v0.15.3 + nmt v0.24.5
  with celestia-app's `ErasuredNamespacedMerkleTree` prints the same digest
  for 32x32 up to 256x256 squares (parity check beyond the fixtures).
- `cargo run --release -p celestia-rs --example proposal_bench`: a local
  single-validator app on RocksDB driven in-process with `bench_load`'s
  load (600 bank sends + 180 small PFBs per height, 4 x 1.9 MB or
  4 x 7.3 MB PFBs every other height: 256x256 squares). CheckTx of every
  tx, then PrepareProposal, ProcessProposal, FinalizeBlock, Commit;
  `--keep` / `--open-only` time reopening the state, `--analyze` breaks
  the state database down by store and key kind.

### What changed

| change | commit |
|---|---|
| EDS in one contiguous buffer; Q1 / Q2 / Q3 extension, row / column NMT roots and the subtree roots of large blobs' commitments on scoped threads (at most 10, the M4's cores; `CELESTIA_RS_SQUARE_THREADS` overrides); leaves hashed without building `namespace ‖ share` copies | `square: flat parallel EDS, …` |
| Valid secp256k1 signatures remembered (65,536 entries; only successes, keyed by key, message digest and signature): a tx's signature is otherwise verified by CheckTx, PrepareProposal, ProcessProposal and FinalizeBlock | `x/ante: remember valid secp256k1 signatures` |
| ProcessProposal reuses the data root of the square this node built in PrepareProposal when its own construction of the proposal's square is byte-identical (the root is a pure function of the shares; Go recomputes, acceptance is unchanged); PrepareProposal decodes normal txs once | `app: ProcessProposal reuses …` |
| RocksDB: LZ4 above the bottommost level (was none), Zstd at the bottom; compact-on-deletion collector for pruned ranges; memtables written out on close | `kvdb: LZ4/Zstd …` |
| `start` flushes every database when it returns, `abci` flushes on SIGTERM / SIGINT (the next start no longer replays up to 2 x 64 MiB of WAL per database) | `celestia-rs: flush every database at shutdown` |

None of these changes consensus-visible output: the square / NMT / shares
Go vectors, the rsmt2d and DAH fixtures, the mainnet DAH test
(`m1-integration`), and every devnet replay (`replay_scenarios` incl. all
v9 scenarios, `replay_empty`, `persistent_replay`) pass after each change.

### Results

**Extend + DAH** (`eds_bench`, median of 7, three interleaved rounds:
range over rounds; Go = rsmt2d's own parallel extension and root
computation, `GOMAXPROCS` = 4):

| original width | Rust before | Rust after | Go rsmt2d | after / Go |
|---|---|---|---|---|
| 64 | 41-49 ms | 38-42 ms | 29-48 ms | ~1.0-1.3x |
| 128 | 208-242 ms | 106-149 ms | 105-164 ms | ~0.9-1.0x |
| 256 | 554-695 ms | 300-428 ms | 382-456 ms | ~0.8x |

The DAH (hash-bound, SHA extensions on both sides) is now faster than Go's;
extension of squares up to 128x128 (GF(2^8) leopard) is still ~2x Go's per
core: `leopard-codec` multiplies with scalar table lookups where
klauspost/reedsolomon uses AVX2 / NEON nibble shuffles. 256x256 squares use
`reed-solomon-simd` (SIMD GF(2^16)) and are at par.

**Proposal pipeline** (`proposal_bench --heights 8 --big-every 2 --reps 1`,
two interleaved rounds, medians; "big" = 8 blocks of 8-30 MB in 256x256
squares, "light" = 6 blocks of 780 txs / 0.4 MB):

| | before | after | after / before |
|---|---|---|---|
| PrepareProposal, big | 913 ms | 543 ms | 0.59x |
| ProcessProposal, big | 928 ms | 243 ms | 0.26x |
| FinalizeBlock, big | 364 ms | 278 ms | 0.76x |
| PrepareProposal, light | 209 ms | 102 ms | 0.49x |
| ProcessProposal, light | 182 ms | 74 ms | 0.41x |
| FinalizeBlock, light | 293 ms | 208 ms | 0.71x |
| CheckTx of 780 new txs | 235 ms | 234 ms | 1.0x |
| Commit | 49-59 ms | 52-53 ms | 1.0x |

Applied to the B1 ratios, PrepareProposal on max-size squares goes from
1.09x Go to roughly 0.65x and ProcessProposal from 0.96x to roughly 0.25x
(own proposals; a Go-proposed block still pays one EDS + DAH, ~0.6x). The
"before" binary was built before the release profile gained
`overflow-checks = true`; the "after" one with it.

Stack samples (gdb, main thread) of light blocks before this work: 24%
secp256k1 verification (k256), 16% tx decoding, 13% fee deduction (bech32
decoding of addresses in `x/bank`), ~20% allocator. After it, CheckTx (the
first verification of each signature) is the one place still paying full
price: next candidates are a faster secp256k1 verifier (libsecp256k1 is
~2-3x k256) and caching decoded addresses.

**Restart.** Reopening the application state of a 120-height run
(`proposal_bench --keep`, then `--open-only`) took 553 ms when the run
ended with 18 MB of WAL and 55 ms when it did not; B1's full node was
SIGTERMed with up to 2 x 64 MiB of WAL in each of its RocksDB databases
(application, blockstore, state, evidence), which is where its ~2 s
before RPC went. With the shutdown flush the WAL is empty at the next
start. The full-node restart (phase C) still has to be re-measured with
these builds (Mac Mini run); the part that was slower than Go is gone.

**App state on disk** (`proposal_bench --heights 120`, 93,600 txs, keep
100 versions):

| | before | after |
|---|---|---|
| `application.rocksdb` at rest | 303 MB (285 SST + 18 WAL) | 272 MB (all SST) |
| same, Zstd on every level + 32 KiB blocks (experiment) | | 269 MB |

Compression headroom is used up: the state holds 3.05 M IAVL nodes (avg
~95 B of value, two 32-byte child hashes in every inner node) and 2.46 M
orphan records (`o ‖ to-version ‖ node key`, 73 MB of keys before
compression; IAVL v1 in Go keeps none and prunes by diffing versions).
B1's 549 MB was measured with `du` after a SIGTERM that left the WAL
(up to 128 MiB) on disk, so part of the 1.67x gap was WAL; the rest is the
node layout. Closing it further needs a storage format change (no orphan
records, or the node's own hash instead of its children's), which is not
consensus-visible but needs a migration of existing homes; left for a
follow-up. Projected for 512 GB: at the validator pruning defaults the
state stays in the hundreds of MB, far inside the 350 GB budget
(`docs/hardware.md`), which the block store dominates.

**Not changed:** FinalizeBlock+Commit and CheckTx were already 1.8-3.6x and
1.6-2.1x faster than Go in B1; the signature cache and the single decode
also trim FinalizeBlock (above). The Mac Mini run should redo phases A and
C with these builds.

## SIMD erasure coding and libsecp256k1 (2026-10-16, Linux shared VM)

Same VM, again compared only back to back. These are the two "next candidates"
named above. Neither one changes output for any input on which Rust already
matched Go. The secp256k1 change also fixes two classes of key on which it
did not. After each change these all pass: the square / shares / NMT /
rsmt2d Go vectors, the DAH fixtures, `m1-integration`, `replay_scenarios`, `replay_empty`,
`persistent_replay` and `query_corpus`. `fuzz-diff --ops eds` against the Go
oracles also passed: 69,395 inputs on v9 and 96,928 on v10, with no
mismatch.

| change | commit |
|---|---|
| GF(2^8) leopard (squares up to 128x128) moves to the in-repo `leopard` crate. It is leopard-codec's algorithm, with the FFT / IFFT butterflies done by split-nibble `pshufb` (SSSE3, AVX2) or `tbl` (NEON) kernels selected at run time. Scalar tables remain the fallback (`CELESTIA_RS_LEOPARD_KERNEL=scalar`). Every kernel is checked against the scalar one for all 256 multipliers, and encode / reconstruct against leopard-codec | `square: SIMD GF(2^8) leopard …` |
| secp256k1 signature verification moves from k256 to libsecp256k1, behind cosmos-sdk's `secp256k1_nocgo.go` rules. It is checked against 563 vectors that carry Go's own result. The vectors also exposed two places where the k256 verifier differed from Go (hybrid keys, 05-prefixed keys); both are now fixed | `x/ante: verify secp256k1 signatures with libsecp256k1` |

**Extension of one square on one thread** (`eds_bench`). Go is rsmt2d with
klauspost/reedsolomon, using AVX-512 GFNI on this VM:

| original width | leopard-codec (scalar) | SIMD leopard | Go rsmt2d |
|---|---|---|---|
| 64 | 16-19 ms | 4-5 ms | 7-11 ms |
| 128 | 80-95 ms | 32-34 ms | 34-42 ms |

Squares of 256x256 already used SIMD GF(2^16) and are unchanged.

**secp256k1.** One verification of a 300-byte message, timed with the same
2,000 signatures for each verifier:

| verifier | time per verification |
|---|---|
| k256 | 97-101 µs |
| libsecp256k1 | 38-41 µs |

`proposal_bench --heights 10 --big-every 0 --reps 1` runs light blocks of
780 txs. The table gives medians over heights 2-10 and three interleaved
rounds:

| | before | after | after / before |
|---|---|---|---|
| CheckTx of 780 new txs | 212 ms | 155 ms | 0.73x |
| PrepareProposal | 96 ms | 92 ms | ~1.0x |
| ProcessProposal | 81 ms | 90 ms | ~1.0x (noise) |
| FinalizeBlock | 189 ms | 194 ms | ~1.0x |

Only CheckTx was expected to move: the other three phases hit the signature
cache. The rest of CheckTx is tx decoding, fee deduction (bech32 address
decoding in `x/bank`) and the allocator.

**App state on disk.** A compact IAVL format (no orphan records, contiguous
pruning by tree diff) is designed in
[ADR 0006](../adr/0006-compact-iavl-storage.md). It is not implemented:
it changes every existing home and needs a migration that is tested on a
real home first.
