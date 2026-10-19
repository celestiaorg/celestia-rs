#!/usr/bin/env bash
# Determinism of the Rust binary (MILESTONES.md C1.3 / F1.9): runs
# `deliver` on a few scenarios and seeds with DELIVER_TRACE under several
# configurations of the same binary (default, one square thread, signature
# cache off) and requires byte-identical traces; every run must also be
# clean against Go. The traces are kept under <out>/<label>/ so that runs
# of other builds (release vs dist) and targets (x86_64 vs aarch64) can be
# compared with `diff -r` (they must be identical too).
#
# usage: DEVNET_BIN=<dir> [FUZZ_DIFF=<bin>] crates/fuzz-diff/determinism.sh <out> <label> [blocks]
#
# Bash 3.2 compatible (macOS runners).
set -uo pipefail
OUT=${1:?out dir}
LABEL=${2:?label}
BLOCKS=${3:-120}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=$(cd "${DEVNET_BIN:?DEVNET_BIN}" && pwd)
export FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
DIR=$OUT/$LABEL
mkdir -p "$DIR"
FAILED=0

# <name> <scenario> <seed> <env...>
CASES="
blob blob 1 DELIVER_IBC=1
ibc ibc 2 DELIVER_IBC=1 DELIVER_WIRE=1 DELIVER_SWEEP=1
hyperlane hyperlane 3 DELIVER_HL=1 DELIVER_RECHECK=1
gov gov 4 DELIVER_IBC=1 DELIVER_SIM=1
v9-staking v9-staking 5 DELIVER_IBC=1 DELIVER_WIRE=1
upgrade v9-upgrade 6 DELIVER_UPGRADE=20 DELIVER_IBC=1 DELIVER_HL=1 DELIVER_RECHECK=1
"
CONFIGS="default threads1 sigoff"

echo "$CASES" | while read -r name scen seed envs; do
  [ -n "$name" ] || continue
  case $scen in
  v9-upgrade) appd="GO_APPD=$BIN/v9/celestia-appd GO_APPD_NEXT=$BIN/celestia-appd" ;;
  v9-*) appd="GO_APPD=$BIN/v9/celestia-appd" ;;
  *) appd="GO_APPD=$BIN/celestia-appd" ;;
  esac
  for cfg in $CONFIGS; do
    case $cfg in
    default) extra="" ;;
    threads1) extra="CELESTIA_RS_SQUARE_THREADS=1" ;;
    sigoff) extra="CELESTIA_RS_SIG_CACHE=off" ;;
    esac
    t=$DIR/$name-$cfg.trace
    # shellcheck disable=SC2086
    env $appd $envs $extra DELIVER_TRACE="$t" \
      "$ROOT/crates/fuzz-diff/deliver-diff.sh" "$scen" "$BLOCKS" 8 "$seed" >"$DIR/$name-$cfg.out" 2>"$DIR/$name-$cfg.err"
    rc=$?
    echo "$LABEL $name $cfg rc=$rc $(wc -l <"$t" | tr -d ' ') heights: $(tail -n 1 "$DIR/$name-$cfg.err")"
    if [ $rc -ne 0 ]; then
      grep -m 5 '^MISMATCH' "$DIR/$name-$cfg.out" | cut -c1-400
      echo "FAIL" >>"$DIR/failed"
    fi
    if [ "$cfg" != default ] && ! cmp -s "$t" "$DIR/$name-default.trace"; then
      echo "NONDETERMINISTIC $LABEL $name: $cfg differs from default at $(cmp "$t" "$DIR/$name-default.trace" | head -n 1)"
      diff "$DIR/$name-default.trace" "$t" | head -n 4
      echo "FAIL" >>"$DIR/failed"
    fi
  done
  # Only the traces are compared across builds and targets.
  rm -f "$DIR/$name"-threads1.trace "$DIR/$name"-sigoff.trace
done
[ -f "$DIR/failed" ] && FAILED=1
exit $FAILED
