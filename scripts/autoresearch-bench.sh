#!/usr/bin/env bash
# One measurement of the B1 app-only replay (conformance/bench/run.sh,
# phase A) for one app, printed as `name: value` lines:
#
#   prepare_heavy_p50_ms   PrepareProposal on max-size squares, median
#   state_disk_mb          application state on disk after the replay
#   app_hash_mismatches    must be 0
#
# The chain must already be recorded with conformance/bench/gen-chain.sh.
# The default is a short run: the first 260 heights (13 max-size blocks,
# 10 PrepareProposal repetitions each) and one repetition of light blocks.
# BENCH_FULL=1 replays the whole chain as `run.sh` does by default; only a
# full run gives a `state_disk_mb` comparable with the benchmark report.
#
# usage: [BENCH_KIND=rs|go] [BENCH_FULL=1] [BENCH_DIR=target/bench]
#        scripts/autoresearch-bench.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
KIND=${BENCH_KIND:-rs}
export BENCH_DIR=${BENCH_DIR:-$ROOT/target/bench}
export DEVNET_BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export DEVNET_PORT_OFFSET=${DEVNET_PORT_OFFSET:-21000}
if [ "$KIND" = rs ]; then
  cargo build -q --release --locked -p celestia-rs
fi
cargo build -q --release --locked -p devnet-conformance --example abci_bench
ulimit -n 4096
if [ -z "${BENCH_FULL:-}" ]; then
  export BENCH_MAX_HEIGHT=${BENCH_MAX_HEIGHT:-260} BENCH_REPS=${BENCH_REPS:-1}
fi
OUT=$BENCH_DIR/results/replay-$KIND.json
rm -f "$OUT"
T0=$(date +%s)
# report.py needs both apps' results; the JSON is what is read here.
BENCH_PHASES=A BENCH_KINDS=$KIND nice -n 10 conformance/bench/run.sh || [ -f "$OUT" ]
python3 - "$OUT" <<'PY'
import json, sys
s = json.load(open(sys.argv[1]))["summary"]
print(f"prepare_heavy_p50_ms: {s['prepare_heavy']['p50_ms']:.1f}")
print(f"prepare_heavy_mean_ms: {s['prepare_heavy']['mean_ms']:.1f}")
print(f"prepare_heavy_p99_ms: {s['prepare_heavy']['p99_ms']:.1f}")
print(f"prepare_heavy_n: {s['prepare_heavy']['n']}")
print(f"state_disk_mb: {s['state_disk_mb']}")
print(f"app_hash_mismatches: {s['app_hash_mismatches']}")
PY
echo "run_secs: $(( $(date +%s) - T0 ))"
