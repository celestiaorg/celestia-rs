# Block execution speed at mainnet height ~560k (November 2026)

This record covers the block-execution performance work of 2026-11-26 to 2026-11-28. It lists what merged, what was measured and dropped (with the numbers, so the ideas are not tried again blind), how the runs were made, and what is still open.

All runs replay mainnet blocks through the node's block executor (`crates/tier-5-node/celestia-rs/examples/replay_bench.rs`). Every run must have 0 app-hash mismatches, and the A/B arms of a comparison must have the same `responses_digest` (a chained SHA-256 of every FinalizeBlock response: tx results with gas and events, block events, and validator updates).

## Method

- **Machine:** an NVMe cloud VM (rs-perf-2), with nothing else running.
- **Home:** a copy of a pruned mainnet home at height 558,370, copied fresh for every run.
- **Caches:** IAVL node cache 4,000,000 and block cache 512 MiB (the shipped defaults), unless a row says otherwise.
- **Window:** heights 558,371–573,370 (15,000 blocks). Use this window for absolute numbers and for any change that affects reads. The first 5,000 blocks of a cold copy are much more expensive while the node cache fills:

  | | blocks/s | disk reads/block |
  |---|---|---|
  | first 5,000 (558,371–563,370) | ≈ 23.7 | 364 |
  | last 10,000 (563,371–573,370) | ≈ 28.2 | 113 |

  A 5,000-block window is acceptable for A/B comparisons with the same start, but its absolute numbers are about 13% below the 15,000-block window.
- **Order:** arms alternate (A B A B …). The baseline spread, about ±0.5–1.5% between baseline runs, is the noise floor. A result inside the baseline spread is not a gain.
- **Long run:** 100,000 blocks (558,371–658,370), RSS sampled every 5 s. This measured memory growth and allocator behaviour ([memory.md](memory.md)).
- **Profiles:** `perf record -F 499 --call-graph fp` with `CELESTIA_PHASE_PROFILE=1`, on a build with frame pointers. The steps are in [profiling-runbook.md](profiling-runbook.md).

## Merged

| PR | Change | Measured |
|---|---|---|
| #720 | Application RocksDB: 128 MiB write buffer, no compact-on-deletion collector | +15.4% blocks/s; compaction writes about 3× lower |
| #726 | gostr R-byte prefilter, JSON string ASCII fast path, one allocation per IAVL node hash | small; part of the cumulative number below |
| #728 | IAVL node cache default 4,000,000 (was 781,250) | +6.3% over 15,000 blocks |
| #732 | IAVL links remember the child they last loaded (weak pointer); cache-hit hash check | +11.1% (cache 781,250), +11.9% (4M); node-cache lookups −98% |
| #735 | Commit signature checks on a dedicated `sig-check-N` thread pool | no speed claim (about 0.1–0.2 ms/block in a microbenchmark); fixes an early-stop test that was unstable under load |
| #750 | mimalloc as the default allocator | +7.1% (15,000 blocks, 3 runs), +5.8% (100,000 blocks); peak RSS +3.3–6% |
| #754 | `replay_bench --per-block-csv` | tooling |

Cumulative, from the baseline before #720 to #732: 19.57 → 26.53 blocks/s (+35.6%) over 15,000 blocks. #750 adds about 6% on top of that.

## Measured and dropped

| Idea | Result | Why it was dropped |
|---|---|---|
| Lazy child links: do not read a child record only to learn its hash/height/size | no gain | Inner records keep only child node keys (Go v1 format). A parent on a write path needs its untouched sibling's hash when it is rehashed, so the reads only move from the tx to flush. At 559k there were 552 reference reads per block, 329 of them later traversed. Kept unmerged on `claude/lazy-child-links`. |
| Read prefetch / MultiGet | not built | Committed reads were about 20% of finalize at 559k, and handlers do not know their keys ahead of time. |
| Latest-value cache below gas metering | low value | Reference reads were only about 220 per block at 559k. Kept unmerged on `claude/value-cache`, off by default. |
| Larger RocksDB block cache by default | not adopted | The default stays 512 MiB. The memory cost of larger caches is in [hardware.md](../hardware.md). |
| Validator record and operator address decode memo (content-addressed, below gas metering) | +0.7% (23.03 → 23.20, 5,000 blocks) | Inside the baseline spread (22.68–23.44); under the 1.5% bar. Microbenchmark: decode 1.68 µs vs clone 0.16 µs, bech32 operator 0.88 µs. The distribution records are rewritten every block, so a memo cannot help them. Branch `claude/validator-decode-memo`. |
| `Mutator::set` descends through the parent's own link (copies only the sibling) | −0.8% (inside noise); lookups and disk reads unchanged | No measurable effect. Branch `claude/iavl-move-links`. |
| Lock-free remembered link (`ArcSwapWeak` instead of `Mutex<Weak>`) | slower: get 28 ns vs 22 ns, clone 55 ns vs 23 ns (microbenchmark) | The remaining cost of `Remembered::get` (about 4.7% self time) is cache-miss latency that any tree walk pays, not lock instructions. Checking the touched bit before setting it added 2 ns. |
| Decode each message once instead of `prost_compatible` re-marshalling | < 1% reachable | On the legacy driver (app v1 at 559k) each message goes through `prost_compatible` once, so a cache has nothing to reuse. Its gogoproto decode is the rejection check (Go never runs a message gogoproto cannot decode), so only the re-marshal half of 2.04% could go, and proving prost reads the original bytes the same way needs schema-aware checks. |
| jemalloc `background_thread:true` | +3.3% | Superseded by mimalloc (+7.1%). It is the fallback if mimalloc is reverted. |
| jemalloc `background_thread:true,tcache_max:32768` | +3.5% | The larger thread cache adds nothing. |
| glibc malloc | not re-measured | RSS settled at 4–5× the live heap in the 2026-10 soak ([memory.md](memory.md)). |

## Profile at 559k (main with #732, cache 4M, about 1,900 blocks after warm-up)

- **Run:** exec 42.1 ms/block (p50 37.8, p90 55.2); finalize 27.6 ms, commit 6.1 ms.
- **Threads (all samples):** executor 52.1%, `rocksdb:low` 28.5%, `iavl-prune` 15.8%, `rocksdb:high` 4.8%.
- **Executor groups:**

  | Group | Share | Detail |
  |---|---|---|
  | RocksDB | 23.6% | reads 14.4%, almost all IAVL `node_bytes` from `child_ref`; zstd 4.2% |
  | allocator | 12.0% | before #750 |
  | prost | 8.1% | |
  | txdecode | 6.65% | |
  | node-cache work | 7.6% | |
  | sha256 | 5.76% | |
  | kernel | 6.0% | |

- **Phases:** begin_block 4.0 ms (distribution 6.3% of executor, staking historical info 3.3%, slashing 3.0%); end_block 1.6 ms (qgb valset 3.5%); tx 17.7 ms (ante 2.9 ms, msgs 10.5 ms; IBC `update_client` is the largest message).

## Profile at 559k on main with every change above (2026-11-28)

[profiling-runbook.md](profiling-runbook.md) run on main `566e6607`: mimalloc, legacy driver, IAVL cache 4,000,000, block cache 512 MiB, heights 558,371–573,370. Outputs are in the benchmark bucket under `rs-replay/prof/v1-558370/` (`run-a.txt`, `blocks.csv`, `folded.txt.gz`, `threads.txt`, `executor-self.txt`, `perf.data.zst`).

- **Run:** 15,000 blocks in 522.8 s = **28.7 blocks/s** (26.53 for #732 alone on the same window).
- **Execution:** exec mean 33.6 ms (p50 30.1, p90 45.3); finalize 20.6 ms, commit 5.1 ms.
- **Node cache:** 330 lookups/block (40.3% hit), 197 disk reads/block.
- **Memory:** peak RSS 3.34 GB.
- **Result check:** `responses_digest` `d4bff64d…30dd`, the same as every earlier run on this window, and the same in the `perf` run.
- **Exec-only blocks/s per 1,000 blocks:** 23.6, 27.5, 24.5, 26.9, 27.4, 29.5, 32.9, 31.4, 30.6, 31.8, 30.4, 32.1, 32.4, 35.4, 36.5. Speed rises as the node cache warms.
- **Executor:** 31.0% of samples. Largest self-time items:
  - sha256 `digest_blocks` 1.75% (IAVL `node_hash`);
  - `Remembered::get` 1.75%.
- **Slowest block:** 558,491 at 5,154 ms, the known one-off RocksDB compaction stall at the first switch to the #720 settings. It is longer under `perf`.
- **Next:** the v0.50-era window (from 8,218,900) and a v2 window are being rerun after a script fix. Their outputs go under `rs-replay/prof/`.

## Profile in the v2 era (2026-11-28)

[profiling-runbook.md](profiling-runbook.md) run on main `566e6607`: mimalloc, app version 2, legacy driver, IAVL cache 4,000,000, block cache 512 MiB, heights 2,371,495–2,386,494. Outputs are in the benchmark bucket under `rs-replay/prof/v2-2371494/`.

- **Run:** 15,000 blocks in 421.3 s = **35.6 blocks/s**.
- **Execution:** exec mean 26.9 ms (p50 23.5, p90 37.1); finalize 15.3 ms, commit 4.1 ms.
- **Node cache:** 253 lookups/block (40.1% hit), 152 disk reads/block.
- **Memory:** peak RSS 2.69 GB.
- **Result check:** `responses_digest` `f5d94c05…29f4d`, the same in the `perf` run.
- **Threads:** executor 31.6% of samples, `rocksdb:low` 23.9%.
- **Executor largest self-time items:** sha256 2.07%, `Remembered::get` 1.73%, `_mi_page_malloc_zero` 1.31%, `Mutator::set` 0.93%, crc32c 0.87%, zstd 0.84%.
- **Slow blocks:**
  - 2,372,703 at 2,630 ms: the one-off RocksDB compaction stall.
  - 2,378,205 at 387 ms: finalize 358 ms, 49,136 node lookups and 30,692 disk reads for a 292 KB block. A state-heavy block, not a large one; worth a look in a later profile.
- **Reading:** the v2 era executes faster than 559k (35.6 vs 28.7 blocks/s) and has no new hot spot. Its profile looks like the 559k one.

## Profile in the v0.50 era (2026-11-28)

[profiling-runbook.md](profiling-runbook.md) run on main `566e6607`: mimalloc, app version 5, v0.50 driver, IAVL cache 4,000,000, block cache 512 MiB, `--save-blocks true` (the window has evidence at 8,218,923, which passed). Heights 8,218,901–8,233,900. Outputs are in the benchmark bucket under `rs-replay/prof/v050-8218900/`.

- **Run:** 15,000 blocks in 295.5 s = **50.8 blocks/s**, with 0 app-hash mismatches.
- **Execution:** exec mean 19.0 ms (p50 16.0, p90 21.6); finalize 7.7 ms, commit 3.8 ms.
- **Node cache:** 37 lookups/block (39.0% hit), 23 disk reads/block, 30 KiB/block.
- **Memory:** peak RSS 2.03 GB.
- **Result check:** `responses_digest` `ac72ec22…95fcdc`, the same in the `perf` run.
- **Threads:** executor 31.7% of samples, `rocksdb:low` 25.9%, `iavl-prune` 10.8%.
- **Executor largest self-time items:** `Remembered::get` 2.15%, sha256 1.96%, `Mutator::set` 1.39%, `_mi_page_malloc_zero` 1.27%, crc32c 1.08%.
- **Slowest block:** 8,220,516 at 2,247 ms (commit 2,109 ms), the one-off RocksDB compaction stall.

## Summary across eras (one build)

| Window | Driver | blocks/s | exec mean ms | finalize ms | disk reads/block | peak RSS |
|---|---|---|---|---|---|---|
| 558,371–573,370 | legacy (app v1) | 28.7 | 33.6 | 20.6 | 197 | 3.34 GB |
| 2,371,495–2,386,494 | legacy (app v2) | 35.6 | 26.9 | 15.3 | 152 | 2.69 GB |
| 8,218,901–8,233,900 | v0.50 (app v5) | 50.8 | 19.0 | 7.7 | 23 | 2.03 GB |

Execution gets faster in later eras, and no era has a new hot spot. The largest executor items are the same everywhere: IAVL hashing, tree walks and allocation. Live `follow` segments are limited by block fetching, not execution, so faster block sources matter more than execution work for a full sync.

## Open follow-ups

- **Era profiles:** done for 559k, the v2 era and the v0.50 era (sections above). Live `follow` segments are limited by RPC fetching, not execution: exec-only rates are 11–27 blocks/s across 1.0M–4.0M heights.
- **mimalloc memory over long runs:** in the 100,000-block soak, RSS still grew in the second half: 69 MiB per 10,000 blocks with mimalloc vs 53 with jemalloc, while the node cache was still filling. The replay segments log RSS every 5 minutes and compare jemalloc and mimalloc builds over about 250,000 blocks. Revert criterion ([memory.md](memory.md)): if RSS grows without bound once the 4,000,000-node cache is full, go back to jemalloc with `background_thread:true` (+3.3%) as a compiled-in `malloc_conf`.
- **Streaming block decode in `light/http.rs`:** `follow` parses each `/block` response into a `serde_json::Value` tree before decoding it. With 8–24 blocks in flight and blocks of several MB, the transient peak can reach about 0.75 GiB. Measure first: compare RSS with `--prefetch 8` and `--prefetch 24` on same-era segments. If the difference at shipped defaults is under about 0.5 GiB, do not change it.
- **First-open RocksDB stall:** all three profile windows have one block of 2–5 s shortly after the home is first opened with the #720 settings, as the first L0 → L5 → L6 compaction cascade runs. Smaller, more frequent L5 → L6 compactions for the application database (`max_bytes_for_level_base`, target file size, `max_subcompactions`) may remove it. Not started.
- **Persistent `iavl-prune` worker:** pruning starts one thread per prune (1,863 threads in about 1,900 blocks, 15.8% of samples). It runs off the executor, so it matters mainly on small machines. Not started.
- **Smaller candidates from the 559k profile:**
  - begin_block distribution and staking work, about 4 ms;
  - IBC `update_client` signature checks;
  - the allocator share after #750, to be re-measured in the next profile.
