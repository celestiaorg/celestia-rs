#!/usr/bin/env bash
# Line and region coverage of the Rust state machine under the execution
# differentials (MILESTONES.md F1.3): builds fuzz-diff instrumented
# (cargo-llvm-cov), runs `deliver` over every scenario at both app versions
# (with the IBC / hyperlane drivers, GOVPASS and the v9 -> v10 upgrade),
# `exec` in every mode and `pp`, then writes an lcov file and a per-file
# summary.
#
# usage: DEVNET_BIN=<dir> crates/fuzz-diff/coverage.sh <out-dir> [blocks]
#
# Needs cargo-llvm-cov (`cargo install cargo-llvm-cov --locked`) and the
# llvm-tools component.
set -uo pipefail
OUT=$(mkdir -p "${1:?out dir}" && cd "$1" && pwd)
BLOCKS=${2:-150}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=$(cd "${DEVNET_BIN:?DEVNET_BIN}" && pwd)
cd "$ROOT"
# shellcheck disable=SC1090
source <(cargo llvm-cov show-env --export-prefix --release)
cargo llvm-cov clean --workspace
cargo build --release -p fuzz-diff
FUZZ_DIFF=$CARGO_LLVM_COV_TARGET_DIR/release/fuzz-diff
[ -x "$FUZZ_DIFF" ] || FUZZ_DIFF=$ROOT/target/release/fuzz-diff
export FUZZ_DIFF
V10=$BIN/celestia-appd
V9=$BIN/v9/celestia-appd
LOG=$OUT/runs.txt
: >"$LOG"
run() { # <name> <env...> -- <cmd...>
  local name=$1
  shift
  local envs=()
  while [ "$1" != "--" ]; do envs+=("$1"); shift; done
  shift
  env "${envs[@]}" "$@" >"$OUT/$name.out" 2>"$OUT/$name.err"
  echo "$name rc=$? $(tail -n 1 "$OUT/$name.err")" | tee -a "$LOG"
}
for scen in gov staking staking-lifecycle hyperlane authz-feegrant fibre vesting zkism-forwarding \
  ibc ibc-edge ibc-upgrades utf8 blob consensus evidence empty \
  v9-gov v9-staking v9-hyperlane v9-authz-feegrant v9-vesting v9-zkism-forwarding v9-ibc v9-utf8 v9-blob v9-exec; do
  case $scen in v9-*) A=$V9 ;; *) A=$V10 ;; esac
  hl=()
  case $scen in *hyperlane* | *zkism*) hl=(DELIVER_HL=1) ;; esac
  run "deliver-$scen" GO_APPD="$A" DELIVER_IBC=1 DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1 "${hl[@]}" -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$BLOCKS" 8 1
  run "govpass-$scen" GO_APPD="$A" DELIVER_IBC=1 DELIVER_GOVPASS=1 DELIVER_SIM=1 "${hl[@]}" -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$BLOCKS" 8 2
  case $scen in ibc-edge | ibc-upgrades) ;; *)
    for mode in strings nil synth json; do
      run "exec-$mode-$scen" GO_APPD="$A" EXEC_MODE=$mode -- crates/fuzz-diff/exec-diff.sh "$scen" 10 3
    done
    ;;
  esac
  run "pp-$scen" GO_APPD="$A" -- crates/fuzz-diff/pp-diff.sh "$scen" 20 4
done
for s in 1 2 3 4; do
  run "upgrade-$s" GO_APPD="$V9" GO_APPD_NEXT="$V10" DELIVER_UPGRADE=$((10 + s * 5)) DELIVER_IBC=1 DELIVER_HL=1 \
    DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1 -- crates/fuzz-diff/deliver-diff.sh v9-upgrade "$BLOCKS" 6 "$s"
done
run "deliver-upgrade" GO_APPD="$V10" DELIVER_REPLAY_STOP=signal DELIVER_IBC=1 -- \
  crates/fuzz-diff/deliver-diff.sh upgrade "$BLOCKS" 8 1
cargo llvm-cov report --release --lcov --output-path "$OUT/lcov.info"
cargo llvm-cov report --release --summary-only >"$OUT/summary.txt"
echo "wrote $OUT/lcov.info and $OUT/summary.txt"
