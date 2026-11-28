# Profiling runbook: one 15,000-block window from a restored checkpoint home

How the block-execution profiles in [block-execution-2026-11.md](block-execution-2026-11.md) are made.

Run this once per window: v2/v3 era (about 2.4M), v0.50 era (after 6.68M), and 559k for continuity.

## 0. Inputs per window

- `HOME_H`: a stopped node home whose state is at height `H` (a restored checkpoint). The bench copies it next to itself first, so leave free disk of about 1.2× its size on the same filesystem.
- A source for blocks `H+1 … H+15001`. Use one of:
  - `BLOCKS_HOME`: a stopped node home whose block store holds those heights. A synced node or the segment's own block-store home works.
  - `--blocks-jsonl a.jsonl.gz,b.jsonl.gz`: block dumps, if the segment exports them.
  - Note: the bench needs block `H+15001` too, because its header carries the app hash it checks against.
- Use the same machine class as the earlier rs-perf runs (NVMe), and nothing else heavy on it.

## 1. Build (main at or after #754, frame pointers for perf)

```sh
git fetch origin && git checkout origin/main
RUSTFLAGS="-C force-frame-pointers=yes" \
  cargo build --release --locked -p celestia-rs --example replay_bench
B=target/release/examples/replay_bench
```

Keep default features. The config line must then print `allocator: mimalloc []`. Do not set `MALLOC_CONF` or `MIMALLOC_*`.

## 2. Run A: timing and counters (no perf)

```sh
OUT=prof/<era>-<H>; mkdir -p $OUT
CELESTIA_PHASE_PROFILE=1 $B \
  --home "$HOME_H" --blocks 15000 \
  --blocks-home "$BLOCKS_HOME" --save-blocks true \
  --iavl-cache 4000000 --db-cache-mib 512 \
  --per-block-csv $OUT/blocks.csv \
  > $OUT/run-a.txt 2> $OUT/run-a.progress
```

- If the blocks come from dumps, use `--blocks-jsonl` in place of `--blocks-home`.
- Keep `--save-blocks true`. Evidence validation reads earlier headers from the block store of the home being executed. Without it, a window that contains evidence fails at the evidence block ("validate 8218923: don't have header #8218921" in the v0.50 window from 8,218,900). Saving blocks adds the block-store write to `validate_ms`, as on a real node.
- Leave pruning at the bench default, which is what `celestia-rs start` does: copied home, background pruning, `pruning-interval 1`.
- Must hold: `app_hash_mismatches: 0` in `run-a.txt`. Record `responses_digest`.

## 3. Run B: CPU profile (same command under perf)

```sh
perf record -F 499 -g --call-graph fp -o $OUT/perf.data -- \
  env CELESTIA_PHASE_PROFILE=1 $B \
    --home "$HOME_H" --blocks 15000 --blocks-home "$BLOCKS_HOME" --save-blocks true \
    --iavl-cache 4000000 --db-cache-mib 512 \
    > $OUT/run-b.txt 2> $OUT/run-b.progress
# folded stacks (all threads, thread name as the first frame):
perf script -i $OUT/perf.data -F comm,tid,ip,sym --no-inline \
  | inferno-collapse-perf --pid > $OUT/folded.txt
# or: stackcollapse-perf.pl --tid
# thread shares:
perf report -i $OUT/perf.data --no-children --sort comm --stdio > $OUT/threads.txt
# executor-only flat profile:
perf report -i $OUT/perf.data --no-children --comm <executor thread name> \
  --sort sym --stdio --percent-limit 0.3 > $OUT/executor-self.txt
```

- The executor thread is the one with the largest share in `threads.txt`. In the 559k profile it was `replay_bench`, the main thread.
- Run B's `responses_digest` must equal run A's.

## 4. Outputs to keep per window

1. `run-a.txt` in full: config and allocator lines, blocks/s (`wall_secs`), exec mean/p50/p90/max, the ten slow blocks, the phase table from `CELESTIA_PHASE_PROFILE`, node cache line, `per_block:` line, `responses_digest`.
2. `blocks.csv` (one row per block: height, txs, block_bytes, total, validate, finalize, commit, rest ms, node_lookups, disk_reads).
3. `folded.txt` (gzip it), `threads.txt` and `executor-self.txt`.
4. Peak RSS of run A (`/usr/bin/time -v` around the command, or the 5 s sampler).
5. The app version of the window (from the home) and which driver ran it (legacy or v0.50).
6. Per 1,000-block window, blocks/s from `run-a.progress` (it prints ms/block every 100 blocks), so a slow sub-range is visible.

## 5. Notes

- 15,000 blocks from a cold copy include the node cache warming up (#732 analysis: the first ~5,000 blocks read about 3× as much from disk). Report the full window. `blocks.csv` lets the analysis split out the warm part.
- Two windows of different eras are not comparable as absolute speed: block sizes differ. Compare per tx and per MB using `blocks.csv`.
