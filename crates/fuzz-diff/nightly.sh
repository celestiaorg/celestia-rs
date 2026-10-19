#!/usr/bin/env bash
# Nightly execution differentials (MILESTONES.md C1.2 / F1.1): runs one
# shard of the `json_*`, `exec`, `pp` and `deliver` harnesses against Go
# celestia-app v10.4.0 and v9.0.8, rotating scenarios, modes and flags with
# the seed so that consecutive nights cover different combinations.
#
# usage: crates/fuzz-diff/nightly.sh <job> <seed> [findings-dir]
#   job: json | exec | pp | deliver-<k>of<n> | upgrade-<k>of<n> | campaign-<k>of<n>
#
# Environment:
#   DEVNET_BIN   Go binaries: $DEVNET_BIN/celestia-appd (v10.4.0) and
#                $DEVNET_BIN/v9/celestia-appd (v9.0.8)
#   FUZZ_DIFF    the fuzz-diff binary (default target/release/fuzz-diff)
#   BUDGET_SECS  wall-clock budget of the shard (default 5400); a run is
#                only started while at least its estimate is left
#   BLOCKS       generated blocks per deliver run (default 300)
#   DRY_RUN=1    print the runs, do not execute them
#
# Every run's output is kept under the findings dir; a run with a
# mismatch (exit 1) keeps its Go log too and makes the script exit 1 after
# the remaining runs. `campaign-*` is C1.4's fresh campaign: every scenario
# at every app version with every flag combination, not rotated.
set -uo pipefail
JOB=${1:?job}
SEED=${2:?seed}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
FINDINGS=${3:-$ROOT/target/nightly-findings}
BIN=$(cd "${DEVNET_BIN:?DEVNET_BIN}" && pwd)
export FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
BUDGET=${BUDGET_SECS:-5400}
BLOCKS=${BLOCKS:-300}
V10=$BIN/celestia-appd
V9=$BIN/v9/celestia-appd
START=$(date +%s)
mkdir -p "$FINDINGS"
FAILED=0
RUNS=0
SUMMARY=$FINDINGS/summary.txt

# Scenarios `deliver` and `pp` drive, per app version. `cli` and
# `comet-queries` record query traffic only; `v9-upgrade` and
# `v9-upgrade-boundary` cross versions (upgrade-* jobs); `upgrade` signals
# v11 and halts, so its generated blocks start at the signal
# (DELIVER_REPLAY_STOP=signal).
V10_SCENARIOS=(gov staking staking-lifecycle hyperlane authz-feegrant fibre vesting
  zkism-forwarding ibc ibc-edge ibc-upgrades utf8 blob consensus evidence empty upgrade)
V9_SCENARIOS=(v9-gov v9-staking v9-hyperlane v9-authz-feegrant v9-vesting
  v9-zkism-forwarding v9-ibc v9-utf8 v9-blob v9-exec v9-empty)

elapsed() { echo $(($(date +%s) - START)); }

# run <name> <estimate-secs> <env...> -- <cmd...>
run() {
  local name=$1 est=$2
  shift 2
  local envs=()
  while [ "$1" != "--" ]; do envs+=("$1"); shift; done
  shift
  if [ $(($(elapsed) + est)) -gt "$BUDGET" ] && [ "$RUNS" -gt 0 ]; then
    echo "SKIP $name (budget: $(elapsed)s used of ${BUDGET}s)" | tee -a "$SUMMARY"
    return
  fi
  RUNS=$((RUNS + 1))
  local out=$FINDINGS/$name
  echo "RUN $name: ${envs[*]} $*" | tee -a "$SUMMARY"
  if [ -n "${DRY_RUN:-}" ]; then return; fi
  local t0
  t0=$(date +%s)
  env "${envs[@]}" GO_LOG="$out.go.log" "$@" >"$out.out" 2>"$out.err"
  local rc=$?
  local line
  line=$(tail -n 1 "$out.err")
  echo "  rc=$rc $(($(date +%s) - t0))s: $line" | tee -a "$SUMMARY"
  if [ $rc -ne 0 ]; then
    FAILED=1
    grep -m 20 '^MISMATCH' "$out.out" | sed 's/^/  /' | cut -c1-400 | tee -a "$SUMMARY"
    { echo "seed=$SEED"; echo "env: ${envs[*]}"; echo "cmd: $*"; } >"$out.repro"
  else
    rm -f "$out.go.log" "$out.out"
  fi
}

# pick <seed> <array...>: the element the seed selects.
pick() {
  local s=$1
  shift
  local a=("$@")
  echo "${a[$((s % ${#a[@]}))]}"
}

# shard <k> <n> <seed> <items...>: items whose rotated index is k mod n.
shard() {
  local k=$1 n=$2 s=$3
  shift 3
  local i=0
  for x in "$@"; do
    if [ $(((i + s) % n)) -eq "$k" ]; then echo "$x"; fi
    i=$((i + 1))
  done
}

# shuffle <seed> <items...>: the items in a seeded order.
shuffle() {
  local s=$1
  shift
  printf '%s\n' "$@" | awk -v s="$s" 'BEGIN{srand(s)}{print rand()"\t"$0}' | sort | cut -f2
}

appd_for() { case $1 in v9-*) echo "$V9" ;; *) echo "$V10" ;; esac; }

# Flags of a deliver run, chosen by the seed: IBC always (the mock
# counterparty's packets cover every scenario), the others in rotation.
deliver_flags() {
  local scen=$1 s=$2
  local f=(DELIVER_IBC=1)
  case $scen in *hyperlane* | *zkism*) f+=(DELIVER_HL=1) ;; esac
  [ $((s % 2)) -eq 0 ] && f+=(DELIVER_WIRE=1)
  [ $((s % 3)) -eq 0 ] && f+=(DELIVER_SWEEP=1)
  [ $((s % 4)) -eq 1 ] && f+=(DELIVER_RECHECK=1)
  [ $((s % 5)) -eq 2 ] && f+=(DELIVER_SIM=1)
  [ "$scen" = upgrade ] && f+=(DELIVER_REPLAY_STOP=signal)
  echo "${f[@]}"
}

deliver_run() { # <scenario> <seed> <flags...>
  local scen=$1 s=$2
  shift 2
  local tag
  tag=$(echo "$@" | sed 's/DELIVER_//g; s/=1//g; s/ /+/g')
  run "deliver-$scen-s$s-${tag:-plain}" $((BLOCKS / 2)) GO_APPD="$(appd_for "$scen")" "$@" -- \
    "$ROOT/crates/fuzz-diff/deliver-diff.sh" "$scen" "$BLOCKS" 8 "$s"
}

upgrade_run() { # <seed> <flags...>
  local s=$1
  shift
  local tag
  tag=$(echo "$@" | sed 's/DELIVER_//g; s/=1//g; s/ /+/g')
  run "upgrade-s$s-${tag:-plain}" $((BLOCKS / 2)) GO_APPD="$V9" GO_APPD_NEXT="$V10" \
    DELIVER_UPGRADE=$((10 + s % 30)) "$@" -- \
    "$ROOT/crates/fuzz-diff/deliver-diff.sh" v9-upgrade "$BLOCKS" 6 "$s"
}

case $JOB in
json)
  OPS=json_count,json_gov,json_pfm,json_pfm_next,json_ica_tx,json_ica_vb,json_ica_pd,json_ica_md,json_ics20,json_ack,json_ack_check,json_ica_pb
  half=$((BUDGET / 2 - 60))
  for v in 10 9; do
    run "json-v$v-s$SEED" "$half" -- "$FUZZ_DIFF" --app-version $v --ops "$OPS" \
      --iters 1000000000 --max-secs "$half" --seed "$SEED" --findings "$FINDINGS/json-v$v"
  done
  ;;
exec)
  # Rounds until the budget is spent: a new seed and mode per round, the
  # scenarios of both versions in a seeded order.
  r=0
  while [ $(($(elapsed) + 60)) -le "$BUDGET" ] && [ -z "${DRY_RUN:-}" -o $r -lt 1 ]; do
    s=$((SEED * 1000 + r))
    MODE=$(pick "$s" strings nil synth json)
    for scen in $(shuffle "$s" "${V10_SCENARIOS[@]}" "${V9_SCENARIOS[@]}"); do
      case $scen in upgrade | ibc-edge | ibc-upgrades) continue ;; esac
      run "exec-$MODE-$scen-s$s" 60 GO_APPD="$(appd_for "$scen")" EXEC_MODE="$MODE" -- \
        "$ROOT/crates/fuzz-diff/exec-diff.sh" "$scen" 20 "$s"
    done
    r=$((r + 1))
  done
  ;;
pp)
  r=0
  while [ $(($(elapsed) + 60)) -le "$BUDGET" ] && [ -z "${DRY_RUN:-}" -o $r -lt 1 ]; do
    s=$((SEED * 1000 + r))
    for scen in $(shuffle "$s" "${V10_SCENARIOS[@]}" "${V9_SCENARIOS[@]}"); do
      case $scen in upgrade) continue ;; esac
      run "pp-$scen-s$s" 60 GO_APPD="$(appd_for "$scen")" -- \
        "$ROOT/crates/fuzz-diff/pp-diff.sh" "$scen" 40 "$s"
    done
    r=$((r + 1))
  done
  ;;
deliver-*of*)
  k=${JOB#deliver-}; n=${k#*of}; k=${k%of*}
  # Rounds over the shard's scenarios, a new seed (and flag set) per
  # round, until the budget is spent.
  r=0
  while [ $(($(elapsed) + BLOCKS)) -le "$BUDGET" ] && [ -z "${DRY_RUN:-}" -o $r -lt 2 ]; do
    s=$((SEED * 1000 + r))
    for scen in $(shard "$k" "$n" "$SEED" "${V10_SCENARIOS[@]}" "${V9_SCENARIOS[@]}"); do
      # shellcheck disable=SC2046
      deliver_run "$scen" "$s" $(deliver_flags "$scen" "$s")
    done
    r=$((r + 1))
  done
  ;;
upgrade-*of*)
  k=${JOB#upgrade-}; n=${k#*of}; k=${k%of*}
  # Seeds of this shard: distinct per night and shard.
  i=0
  while [ $(($(elapsed) + BLOCKS)) -le "$BUDGET" ] && [ -z "${DRY_RUN:-}" -o $i -lt 3 ]; do
    s=$((SEED * 1000 + i * n + k))
    i=$((i + 1))
    j=$((SEED + i + k))
    f=(DELIVER_IBC=1 DELIVER_HL=1)
    [ $((j % 2)) -eq 0 ] && f+=(DELIVER_WIRE=1)
    [ $((j % 3)) -eq 0 ] && f+=(DELIVER_SWEEP=1)
    [ $((j % 2)) -eq 1 ] && f+=(DELIVER_RECHECK=1)
    upgrade_run "$s" "${f[@]}"
  done
  ;;
campaign-*of*)
  # C1.4: every scenario at both versions with every flag, then the
  # upgrade over 10+ seeds; sharded over n jobs.
  k=${JOB#campaign-}; n=${k#*of}; k=${k%of*}
  items=()
  for scen in "${V10_SCENARIOS[@]}" "${V9_SCENARIOS[@]}"; do
    items+=("d:$scen")
  done
  for i in $(seq 1 12); do items+=("u:$i"); done
  for it in $(shard "$k" "$n" 0 "${items[@]}"); do
    case $it in
    d:*)
      scen=${it#d:}
      f=(DELIVER_IBC=1 DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1)
      case $scen in *hyperlane* | *zkism*) f+=(DELIVER_HL=1) ;; esac
      [ "$scen" = upgrade ] && f+=(DELIVER_REPLAY_STOP=signal)
      deliver_run "$scen" "$SEED" "${f[@]}"
      ;;
    u:*)
      i=${it#u:}
      upgrade_run "$((SEED * 100 + i))" DELIVER_IBC=1 DELIVER_HL=1 DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1
      ;;
    esac
  done
  ;;
*)
  echo "unknown job $JOB" >&2
  exit 2
  ;;
esac

echo "$JOB seed $SEED: $RUNS runs in $(elapsed)s, failed=$FAILED" | tee -a "$SUMMARY"
exit $FAILED
