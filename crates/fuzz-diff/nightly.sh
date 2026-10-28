#!/usr/bin/env bash
# Nightly execution differentials (MILESTONES.md C1.2 / F1.1): runs one
# shard of the `json_*`, `c_*` (consensus types, MILESTONES.md F1.7),
# `exec`, `pp` and `deliver` harnesses against Go celestia-app v10.5.0 and
# v9.0.8 (celestia-core v0.42.3), rotating scenarios, modes and flags with
# the seed so that consecutive nights cover different combinations.
#
# usage: crates/fuzz-diff/nightly.sh <job> <seed> [findings-dir]
#   job: regressions | json | consensus | exec | pp | deliver-<k>of<n> | upgrade-<k>of<n> |
#        campaign-<k>of<n>
#
# Environment:
#   DEVNET_BIN   Go binaries: $DEVNET_BIN/celestia-appd (v10.5.0) and
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
git -C "$ROOT" rev-parse HEAD >"$FINDINGS/revision.txt"
git -C "$ROOT" diff --binary HEAD >"$FINDINGS/worktree.patch"
git -C "$ROOT" status --porcelain >"$FINDINGS/worktree.status"
printf '%s\n' "$JOB seed=$SEED budget=$BUDGET blocks=$BLOCKS" >"$FINDINGS/job.txt"
if command -v go >/dev/null 2>&1; then
  go version -m "$V10" "$V9" >"$FINDINGS/go-build-info.txt" 2>&1
fi
FAILED=0
RUNS=0
SKIPPED=0
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
    SKIPPED=$((SKIPPED + 1))
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
  printf '%s\n' "$rc" >"$out.exit"
  printf '%s\n' "${envs[@]}" >"$out.env"
  local line
  line=$(tail -n 1 "$out.err")
  echo "  rc=$rc $(($(date +%s) - t0))s: $line" | tee -a "$SUMMARY"
  if [ $rc -ne 0 ]; then
    FAILED=1
    echo "$name" >>"$FINDINGS/failed"
    grep -m 20 '^MISMATCH' "$out.out" | sed 's/^/  /' | cut -c1-400 | tee -a "$SUMMARY"
    { echo "seed=$SEED"; echo "env: ${envs[*]}"; echo "cmd: $*"; } >"$out.repro"
  else
    # Keep TYPE / GOV_EXECUTED and finalized-tx counts even on success:
    # F1.2 and F1.3 need evidence from clean campaigns too.
    rm -f "$out.go.log"
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
  local scen=$1 s=$2 mode
  local f=(DELIVER_IBC=1)
  case $scen in *hyperlane* | *zkism*)
    mode=$(pick "$((s / 3))" noop message-id merkle-root)
    f+=(DELIVER_HL=1 "DELIVER_HL_ISM=$mode")
    [ "$mode" != noop ] && f+=(DELIVER_HL_REJECT=1)
    ;;
  esac
  [ $((s % 2)) -eq 0 ] && f+=(DELIVER_WIRE=1)
  [ $((s % 3)) -eq 0 ] && f+=(DELIVER_SWEEP=1)
  [ $((s % 4)) -eq 1 ] && f+=(DELIVER_RECHECK=1)
  [ $((s % 5)) -eq 2 ] && f+=(DELIVER_SIM=1)
  [ $((s % 3)) -eq 1 ] && f+=(DELIVER_GOVPASS=1)
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

# Full campaign: every verifier mode for Hyperlane scenarios, including
# rejection/retry coverage for signed modes. Other scenarios run once.
campaign_deliver_run() { # <scenario> <seed> <flags...>
  local scen=$1 s=$2 mode
  shift 2
  case $scen in *hyperlane* | *zkism*)
    for mode in noop message-id merkle-root; do
      local f=(DELIVER_HL=1 "DELIVER_HL_ISM=$mode")
      [ "$mode" != noop ] && f+=(DELIVER_HL_REJECT=1)
      deliver_run "$scen" "$s" "$@" "${f[@]}"
    done
    ;;
  *) deliver_run "$scen" "$s" "$@" ;;
  esac
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
regressions)
  # Pinned deliver findings (crates/fuzz-diff/deliver-regressions.txt).
  n=0
  while read -r scen blocks per seed envs; do
    [ -n "$scen" ] || continue
    n=$((n + 1))
    # GO_APPD_NEXT=v10 names the v10 binary (the v9 -> v10 switch).
    envs=${envs//GO_APPD_NEXT=v10/GO_APPD_NEXT=$V10}
    appd=$(appd_for "$scen")
    case $scen in v9-upgrade*) appd=$V9 ;; esac
    # shellcheck disable=SC2086
    run "regression-$n-$scen-s$seed" 0 GO_APPD="$appd" $envs -- \
      "$ROOT/crates/fuzz-diff/deliver-diff.sh" "$scen" "$blocks" "$per" "$seed"
  done < <(grep -v '^#' "$ROOT/crates/fuzz-diff/deliver-regressions.txt")
  # A large invalid-denom error used to exceed the exec client's HTTP/2
  # header limit and masquerade as a consensus mismatch (nightly seed 13).
  run "regression-exec-fibre-s13008" 0 GO_APPD="$V10" EXEC_MODE=strings -- \
    "$ROOT/crates/fuzz-diff/exec-diff.sh" fibre 20 13008
  ;;
json)
  OPS=json_count,json_gov,json_pfm,json_pfm_next,json_ica_tx,json_ica_vb,json_ica_pd,json_ica_md,json_ics20,json_ack,json_ack_check,json_ica_pb
  half=$((BUDGET / 2 - 60))
  for v in 10 9; do
    run "json-v$v-s$SEED" "$half" -- "$FUZZ_DIFF" --app-version $v --ops "$OPS" \
      --iters 1000000000 --max-secs "$half" --seed "$SEED" --findings "$FINDINGS/json-v$v"
  done
  ;;
consensus)
  # celestia-core's types (votes, proposals, headers, blocks, commits,
  # validator sets and updates, params, evidence, part sets, signatures):
  # the v10 oracle only (v9 runs the same celestia-core).
  OPS=c_vote,c_proposal,c_header,c_block,c_commit,c_valset,c_valupd,c_params,c_evidence,c_partset,c_ed25519,c_secp256k1,c_prop,c_secp256r1,c_ics23,c_groth16,c_vblock
  t=$((BUDGET - 60))
  run "consensus-s$SEED" "$t" -- "$FUZZ_DIFF" --app-version 10 --ops "$OPS" \
    --iters 1000000000 --max-secs "$t" --seed "$SEED" --findings "$FINDINGS/consensus"
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
    # Divide by the flag cycle so every verifier sees both settings of
    # WIRE / SWEEP / RECHECK. Rejection setup cannot be combined with upgrades.
    mode=$(pick "$((j / 6))" noop message-id merkle-root)
    f=(DELIVER_IBC=1 DELIVER_HL=1 "DELIVER_HL_ISM=$mode")
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
    items+=("d:$scen" "g:$scen")
  done
  for i in $(seq 1 12); do items+=("u:$i"); done
  for it in $(shard "$k" "$n" 0 "${items[@]}"); do
    case $it in
    d:*)
      scen=${it#d:}
      f=(DELIVER_IBC=1 DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1)
      [ "$scen" = upgrade ] && f+=(DELIVER_REPLAY_STOP=signal)
      campaign_deliver_run "$scen" "$SEED" "${f[@]}"
      ;;
    g:*)
      # The same with every proposal passed (handler error texts reach
      # state through `failed_reason`).
      scen=${it#g:}
      f=(DELIVER_IBC=1 DELIVER_GOVPASS=1 DELIVER_RECHECK=1)
      [ "$scen" = upgrade ] && f+=(DELIVER_REPLAY_STOP=signal)
      campaign_deliver_run "$scen" "$((SEED + 1))" "${f[@]}"
      ;;
    u:*)
      i=${it#u:}
      mode=$(pick "$((i - 1))" noop message-id merkle-root)
      upgrade_run "$((SEED * 100 + i))" DELIVER_IBC=1 DELIVER_HL=1 "DELIVER_HL_ISM=$mode" DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1
      ;;
    esac
  done
  ;;
*)
  echo "unknown job $JOB" >&2
  exit 2
  ;;
esac

case $JOB in
campaign-*of*)
  if [ "$SKIPPED" -ne 0 ]; then
    FAILED=1
    echo "INCOMPLETE campaign: $SKIPPED required runs skipped" | tee -a "$SUMMARY"
  fi
  ;;
esac

echo "$JOB seed $SEED: $RUNS runs, $SKIPPED skipped in $(elapsed)s, failed=$FAILED" | tee -a "$SUMMARY"
exit $FAILED
