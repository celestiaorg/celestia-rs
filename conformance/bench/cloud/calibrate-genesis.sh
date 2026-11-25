#!/usr/bin/env bash
# Compare CPU placements using the same 90-height recording. Capture a
# separate sampled run so profiling never contributes to a reported timing.
set -euo pipefail
cd "$HOME/genesis-sync/source"
exec 9>../setup.lock
flock -n 9
[[ -f ../ready ]]
out=$(mktemp -d "$HOME/genesis-sync/calibration.XXXXXX")
mkdir "$out/results"
ln -sfn "$out/results" "$HOME/genesis-sync/latest-calibration"
run_sync() {
    local placement=$1 trial=$2 d start
    d="$out/$placement-$trial"
    mkdir -p "$d/chain"
    cp target/bench/chain/{genesis.json,meta.json} "$d/chain/"
    cp -a target/bench/chain/home "$d/chain/"
    cp conformance/bench/run.sh "$d/harness.sh"
    # The harness finds the repository from its script path. Use the pinned
    # source directory when running this temporary instrumented copy.
    sed -i '/^ROOT=/c\ROOT=$PWD' "$d/harness.sh"
    sed -i 's/^    sleep 0\.5$/    sleep 0.05/' "$d/harness.sh"
    # Phase C restarts the node and can advance the observed height. Finish
    # this trial after B and let the existing EXIT trap stop its processes.
    sed -i '/^  # C: restart/i\  return 0' "$d/harness.sh"
    if [[ "$placement" == split ]]; then
      sed -i 's/^  celestia-appd start/  taskset -c 0,4 celestia-appd start/; s/^  cometbft start/  taskset -c 0,4 cometbft start/; s/"\$RS" start/taskset -c 1-3,5-7 "$RS" start/' "$d/harness.sh"
    fi
    if [[ "$trial" == profile ]]; then
      # Phase B is the only enabled phase and returns before the restart.
      sed -i '/^  start_full "\$kind" "\$h" "\$W\/full-\$kind.log"$/a\  sudo -n "$PERF" record -e cpu-clock -F 99 -p "$FULL_PID" -o "$BENCH_DIR/perf.data" >"$BENCH_DIR/perf.log" 2>\&1 \&' "$d/harness.sh"
    fi
    start=$(date +%s)
    BENCH_DIR="$d" BENCH_PHASES=B BENCH_KINDS=rs timeout 300 bash "$d/harness.sh" >"$d/run.log" 2>&1
    cp "$d/results/blocksync-rs.json" "$out/results/$placement-$trial.json"
    echo "$(( $(date +%s) - start ))" > "$out/results/$placement-$trial.runtime"
    cp "$d/run.log" "$out/results/$placement-$trial.log"
    if [[ "$trial" == profile ]]; then
      sudo -n "$PERF" report --stdio --no-children --percent-limit 0.5 -i "$d/perf.data" > "$out/results/profile.txt"
    fi
    rm -rf "$d/chain/home" "$d/work/full-rs"
}

for placement in shared split; do
  run_sync "$placement" 1
  run_sync "$placement" 2
done
for placement in split shared; do
  if jq -es '.[0].heights == .[1].heights and (.[1].secs / .[0].secs | . >= 0.98 and . <= 1.02)' \
    "$out/results/$placement-1.json" "$out/results/$placement-2.json" >/dev/null; then
    run_sync "$placement" baseline
    sudo -n apt-get -qq install -y "linux-tools-$(uname -r)" || sudo -n apt-get -qq install -y linux-tools-generic
    PERF=$(find /usr/lib/linux-tools -type f -name perf | sort -V | tail -1)
    export PERF
    run_sync "$placement" profile
    break
  fi
done
