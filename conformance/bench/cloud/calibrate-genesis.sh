#!/usr/bin/env bash
# Compare CPU placements with a fixed source tip. Capture a
# separate sampled run so profiling never contributes to a reported timing.
set -euo pipefail
cd "$HOME/genesis-sync/source"
exec 9>../setup.lock
flock -n 9
[[ -f ../ready ]]
out=$(mktemp -d "$HOME/genesis-sync/calibration.XXXXXX")
mkdir "$out/results"
ln -sfn "$out/results" "$HOME/genesis-sync/latest-calibration"
# Serve the block store without the voting key that produced the recording.
# Generate one non-validator key for all trials in this calibration.
target/devnet-bin/cometbft init --home "$out/non-validator" >"$out/init.log" 2>&1
run_sync() {
    local placement=$1 trial=$2 d start status
    d="$out/$placement-$trial"
    mkdir -p "$d/chain"
    cp target/bench/chain/{genesis.json,meta.json} "$d/chain/"
    cp -a target/bench/chain/home "$d/chain/"
    cp "$out/non-validator/config/priv_validator_key.json" "$d/chain/home/core/config/"
    cp "$out/non-validator/data/priv_validator_state.json" "$d/chain/home/core/data/"
    # Block sync needs a successor block to verify a commit and switches to
    # consensus near the peer tip. Keep the target within verifiable history.
    jq -e '.last_height |= (. - 3) | select(.last_height > 0)' target/bench/chain/meta.json >"$d/chain/meta.json"
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
    # Finish writing the copied RocksDB files before starting the clock.
    # Read them in a fixed order so trials start with resident source data.
    sync -f "$d/chain/home"
    find "$d/chain/home" -type f -print0 | LC_ALL=C sort -z | xargs -0 -r cat >/dev/null
    start=$(date +%s)
    status=0
    BENCH_DIR="$d" BENCH_PHASES=B BENCH_KINDS=rs timeout "${run_timeout:-300}" bash "$d/harness.sh" >"$d/run.log" 2>&1 || status=$?
    cp "$d/run.log" "$out/results/$placement-$trial.log"
    if [[ -f "$d/work/full-rs.log" ]]; then
      tail -n 1000 "$d/work/full-rs.log" >"$out/results/$placement-$trial-node.log"
    fi
    if [[ "$status" != 0 ]]; then return "$status"; fi
    cp "$d/results/blocksync-rs.json" "$out/results/$placement-$trial.json"
    echo "$(( $(date +%s) - start ))" > "$out/results/$placement-$trial.runtime"
    if [[ "$trial" == profile ]]; then
      sudo -n "$PERF" report --stdio --no-children --percent-limit 0.5 -i "$d/perf.data" > "$out/results/profile.txt"
    fi
    rm -rf "$d/chain/home" "$d/work/full-rs"
}

# A trial changes one numeric constant in an isolated copy of the pinned
# source. Require the operator's independently computed Git blob before build.
if (( $# )); then
  [[ $# == 3 && "$2" =~ ^[0-9a-f]{40}$ && "$3" =~ ^[0-9]{1,3}\.[0-9]{1,3}$ ]]
  expected_blob=$2
  reference=$3
  trial_tree="$out/source"
  git worktree add --quiet --detach "$trial_tree" 2da1bf7cc88a1afbe84f2c0ef04cfda11a3fc2d8
  trap 'git worktree remove --force "$trial_tree"' EXIT
  pool=crates/tier-4-runtime/blocksync/src/pool.rs
  case "$1" in
    floor-[2-8])
      floor=${1#floor-}
      sed -i "s/^pub const MIN_REQ_LIMIT: i64 = 2;$/pub const MIN_REQ_LIMIT: i64 = $floor;/" "$trial_tree/$pool" ;;
    [2-9][0-9][0-9][0-9])
      (( 10#$1 > 2000 ))
      sed -i "s/^pub const PEER_CONN_WAIT: Duration = Duration::from_secs(3);$/pub const PEER_CONN_WAIT: Duration = Duration::from_millis($1);/" "$trial_tree/$pool" ;;
    control) ;;
    *) exit 2 ;;
  esac
  [[ "$(git hash-object "$trial_tree/$pool")" == "$expected_blob" ]]
  export PATH="$HOME/.cargo/bin:$PATH"
  export CARGO_TARGET_DIR="$HOME/genesis-sync/trial-target"
  (cd "$trial_tree" && cargo build --release -p celestia-rs) >"$out/build.log" 2>&1 || { tail -n 20 "$out/build.log"; exit 1; }
  export CELESTIA_RS="$CARGO_TARGET_DIR/release/celestia-rs"
  run_timeout=74
  run_sync split candidate
  if [[ "$1" == control ]] || jq -e --argjson reference "$reference" '.secs < $reference' "$out/results/split-candidate.json" >/dev/null; then
    if (cd "$trial_tree" && cargo test --workspace --all-features) >"$out/tests.log" 2>&1; then
      echo pass > "$out/results/tests.txt"
    else
      tail -n 50 "$out/tests.log" > "$out/results/test-failure.log"
      echo fail > "$out/results/tests.txt"
      exit 1
    fi
  fi
  exit 0
fi

for placement in shared split; do
  run_sync "$placement" 1
  run_sync "$placement" 2
done
for placement in split shared; do
  if jq -es '.[0].heights == .[1].heights and (.[1].secs / .[0].secs | . >= 0.98 and . <= 1.02)' \
    "$out/results/$placement-1.json" "$out/results/$placement-2.json" >/dev/null; then
    selected=$placement
    run_sync "$placement" baseline
    break
  fi
done

# Keep the diagnostic profile even when both timing pairs are noisy.
sudo -n apt-get -qq install -y "linux-tools-$(uname -r)" || sudo -n apt-get -qq install -y linux-tools-generic
PERF=$(command -v perf || true)
if [[ -z "$PERF" ]] || ! "$PERF" version; then
  PERF=$(find -L /usr/lib/linux-tools -type f -executable -name perf | sort -V | tail -1)
fi
[[ -n "$PERF" ]] || { echo 'No executable perf installed' >&2; exit 1; }
"$PERF" version
export PERF
run_sync "${selected:-split}" profile
