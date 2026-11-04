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
# llvm-tools component. For LLVM branch counters, select an explicit nightly:
# COVERAGE_BRANCH=1 COVERAGE_TOOLCHAIN=nightly-2026-11-02 ...
set -uo pipefail
OUT_ARG=${1:?fresh out dir}
mkdir -p "$(dirname -- "$OUT_ARG")" || exit 1
# Never mix a previous branch inventory with new metadata or a failed rerun.
mkdir -- "$OUT_ARG" || { echo "coverage requires a fresh output directory" >&2; exit 1; }
OUT=$(cd "$OUT_ARG" && pwd) || exit 1
BLOCKS=${2:-150}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=$(cd "${DEVNET_BIN:?DEVNET_BIN}" && pwd)
cd "$ROOT"
CARGO=(cargo)
RUSTC=(rustc)
if [ -n "${COVERAGE_TOOLCHAIN:-}" ]; then
  CARGO+=("+$COVERAGE_TOOLCHAIN")
  RUSTC+=("+$COVERAGE_TOOLCHAIN")
fi
BRANCH_FLAGS=()
case ${COVERAGE_BRANCH:-0} in
  0) ;;
  1)
    [ -n "${COVERAGE_TOOLCHAIN:-}" ] || { echo "branch coverage requires explicit COVERAGE_TOOLCHAIN" >&2; exit 1; }
    BRANCH_FLAGS=(--branch)
    ;;
  *) echo "COVERAGE_BRANCH must be 0 or 1" >&2; exit 1 ;;
esac
printf '%s\n' "branch=${COVERAGE_BRANCH:-0}" "toolchain=${COVERAGE_TOOLCHAIN:-active}" >"$OUT/coverage-mode.txt"
"${RUSTC[@]}" -Vv >"$OUT/rustc.txt" || exit 1
"${CARGO[@]}" llvm-cov --version >"$OUT/cargo-llvm-cov.txt" || exit 1
# Keep the local ABCI / oracle listeners below Linux's default ephemeral
# range. Otherwise an unrelated outgoing connection can intermittently
# occupy a fixed harness port between sequential cases.
export DEVNET_PORT_OFFSET=${DEVNET_PORT_OFFSET:--10000}
# Its own target directory: a build in target/ while this runs would
# replace instrumented libraries.
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target/cov}
# Capture setup failure before sourcing: process substitution can hide it.
"${CARGO[@]}" llvm-cov show-env --export-prefix --release "${BRANCH_FLAGS[@]}" >"$OUT/coverage-env.sh" || exit 1
# shellcheck disable=SC1090
source "$OUT/coverage-env.sh" || exit 1
"${CARGO[@]}" llvm-cov clean --workspace || exit 1
"${CARGO[@]}" build --locked --release -p fuzz-diff || exit 1
FUZZ_DIFF=$CARGO_TARGET_DIR/release/fuzz-diff
[ -x "$FUZZ_DIFF" ] || { echo "no instrumented fuzz-diff at $FUZZ_DIFF" >&2; exit 1; }
export FUZZ_DIFF
V10=$BIN/celestia-appd
V9=$BIN/v9/celestia-appd
LOG=$OUT/runs.txt
: >"$LOG"
git rev-parse HEAD >"$OUT/revision.txt"
git diff --binary HEAD >"$OUT/worktree.patch"
git status --porcelain --untracked-files=all --ignore-submodules=none >"$OUT/worktree.status"
if command -v go >/dev/null 2>&1; then
  go version -m "$V10" "$V9" >"$OUT/go-build-info.txt" 2>&1
else
  echo "go command unavailable; compiler provenance not captured" >"$OUT/go-build-info.txt"
fi
FAILED=0
run() { # <name> <env...> -- <cmd...>
  local name=$1
  shift
  local envs=()
  while [ "$1" != "--" ]; do envs+=("$1"); shift; done
  shift
  env "${envs[@]}" "$@" >"$OUT/$name.out" 2>"$OUT/$name.err"
  local rc=$?
  [ "$rc" -eq 0 ] || FAILED=1
  echo "$name rc=$rc $(tail -n 1 "$OUT/$name.err")" | tee -a "$LOG"
}
for scen in gov staking staking-lifecycle hyperlane authz-feegrant fibre vesting zkism-forwarding \
  ibc ibc-edge ibc-upgrades utf8 blob consensus evidence empty \
  v9-gov v9-staking v9-hyperlane v9-authz-feegrant v9-vesting v9-zkism-forwarding v9-ibc v9-utf8 v9-blob v9-exec; do
  case $scen in v9-*) A=$V9 ;; *) A=$V10 ;; esac
  hl=()
  case $scen in *hyperlane* | *zkism*) hl=(DELIVER_HL=1) ;; esac
  run "deliver-$scen" GO_APPD="$A" DELIVER_IBC=1 DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1 "${hl[@]}" -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$BLOCKS" 8 1
  run "govpass-$scen" GO_APPD="$A" DELIVER_IBC=1 DELIVER_GOVPASS=1 DELIVER_SIM=1 \
    DELIVER_GOV_REQUIRE=/ibc.applications.interchain_accounts.host.v1.MsgUpdateParams \
    DELIVER_TYPE_REQUIRE=/cosmos.gov.v1beta1.MsgDeposit "${hl[@]}" -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$BLOCKS" 8 2
  case $scen in ibc-edge | ibc-upgrades) ;; *)
    for mode in strings nil synth json; do
      run "exec-$mode-$scen" GO_APPD="$A" EXEC_MODE=$mode -- crates/fuzz-diff/exec-diff.sh "$scen" 10 3
    done
    ;;
  esac
  run "pp-$scen" GO_APPD="$A" -- crates/fuzz-diff/pp-diff.sh "$scen" 20 4
done
for scen in hyperlane v9-hyperlane; do
  case $scen in v9-*) A=$V9 ;; *) A=$V10 ;; esac
  run "hyperlane-registry-$scen" GO_APPD="$A" DELIVER_HL=1 DELIVER_HL_ISM=merkle-root \
    DELIVER_HL_REGISTRY_REQUIRE=1 -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$BLOCKS" 8 6
done
for scen in gov v9-gov; do
  case $scen in v9-*) A=$V9 ;; *) A=$V10 ;; esac
  run "gov-abstain-$scen" GO_APPD="$A" DELIVER_GOVPASS=1 DELIVER_GOV_ABSTAIN_REQUIRE=1 -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$BLOCKS" 8 5
done
for scen in authz-feegrant v9-authz-feegrant; do
  case $scen in
    v9-*) A=$V9; SCRIPT=crates/fuzz-diff/fixtures/stake-authz-boundaries-v9.script ;;
    *) A=$V10; SCRIPT=crates/fuzz-diff/fixtures/stake-authz-boundaries.script ;;
  esac
  run "stake-authz-$scen" GO_APPD="$A" \
    DELIVER_SCRIPT="$SCRIPT" \
    DELIVER_SCRIPT_REQUIRE=crates/fuzz-diff/fixtures/stake-authz-boundaries.expected.json -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" 21 1 17362
done
for scen in evidence v9-authz-feegrant; do
  case $scen in
    v9-*)
      A=$V9
      SCRIPT=crates/fuzz-diff/fixtures/evidence-submit-boundaries-v9.script
      EXPECT=crates/fuzz-diff/fixtures/evidence-submit-boundaries-v9.expected.json
      N=2
      ;;
    *)
      A=$V10
      SCRIPT=crates/fuzz-diff/fixtures/evidence-submit-boundaries.script
      EXPECT=crates/fuzz-diff/fixtures/evidence-submit-boundaries.expected.json
      N=3
      ;;
  esac
  run "evidence-submit-$scen" GO_APPD="$A" \
    DELIVER_SCRIPT="$SCRIPT" DELIVER_SCRIPT_REQUIRE="$EXPECT" -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" "$N" 1 17362
done
for scen in evidence v9-authz-feegrant; do
  case $scen in v9-*) A=$V9 ;; *) A=$V10 ;; esac
  run "evidence-validation-$scen" GO_APPD="$A" \
    DELIVER_SCRIPT=crates/fuzz-diff/fixtures/evidence-validation-boundaries.script \
    DELIVER_SCRIPT_REQUIRE=crates/fuzz-diff/fixtures/evidence-validation-boundaries.expected.json -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" 7 1 17363
done
for scen in authz-feegrant v9-authz-feegrant; do
  case $scen in v9-*) A=$V9 ;; *) A=$V10 ;; esac
  run "evidence-stale-$scen" GO_APPD="$A" DELIVER_EVIDENCE_REQUIRE=stale -- \
    crates/fuzz-diff/deliver-diff.sh "$scen" 4 0 17364
done
run "evidence-removed-validator-authz-feegrant" GO_APPD="$V10" \
  DELIVER_EVIDENCE_REQUIRE=removed-validator -- \
  crates/fuzz-diff/deliver-diff.sh authz-feegrant 1 0 17365
for s in 1 2 3 4; do
  run "upgrade-$s" GO_APPD="$V9" GO_APPD_NEXT="$V10" DELIVER_UPGRADE=$((10 + s * 5)) DELIVER_IBC=1 DELIVER_HL=1 \
    DELIVER_WIRE=1 DELIVER_SWEEP=1 DELIVER_RECHECK=1 -- crates/fuzz-diff/deliver-diff.sh v9-upgrade "$BLOCKS" 6 "$s"
done
run "deliver-upgrade" GO_APPD="$V10" DELIVER_REPLAY_STOP=signal DELIVER_IBC=1 -- \
  crates/fuzz-diff/deliver-diff.sh upgrade "$BLOCKS" 8 1
"${CARGO[@]}" llvm-cov report --release --lcov --output-path "$OUT/lcov.info" || exit 1
"${CARGO[@]}" llvm-cov report --release --summary-only >"$OUT/summary.txt" || exit 1
"${CARGO[@]}" llvm-cov report --release --json --output-path "$OUT/llvm.json" || exit 1
if [ "${COVERAGE_BRANCH:-0}" = 1 ]; then
  python3 crates/fuzz-diff/branch-report.py "$OUT/llvm.json" --out "$OUT/branches.json" || exit 1
fi
echo "wrote $OUT/lcov.info and $OUT/summary.txt"
exit "$FAILED"
