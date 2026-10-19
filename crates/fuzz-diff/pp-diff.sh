#!/usr/bin/env bash
# ProcessProposal differential (crates/fuzz-diff/src/pp.rs): starts a Go
# celestia-appd as a standalone ABCI app on the scenario's genesis, then
# re-drives the recorded scenario into it and into the Rust app, comparing
# the decisions on mutated proposals at every height.
#
# usage: GO_APPD=<celestia-appd of the scenario's version> \
#        crates/fuzz-diff/pp-diff.sh <scenario> [per-height] [seed]
#
# v10 scenarios need celestia-appd v10.4.0, v9-* scenarios v9.0.8 (both as
# built by conformance/devnet/devnet.sh into $DEVNET_BIN and $DEVNET_BIN/v9).
# v9-upgrade switches binaries mid-run and is not supported (deliver-diff.sh
# supports it: GO_APPD_NEXT).
set -euo pipefail
SCENARIO=${1:?scenario}
PER=${2:-20}
SEED=${3:-1}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APPD=${GO_APPD:?path to celestia-appd}
PORT=$((37658 + ${DEVNET_PORT_OFFSET:-0}))
IN=$ROOT/conformance/fixtures/devnet/$SCENARIO
WORK=$(mktemp -d)
H=$WORK/home
CHAIN_ID=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['chain_id'])" "$IN/meta.json")
"$APPD" init node --chain-id "$CHAIN_ID" --home "$H" >"$WORK/init.log" 2>&1 || {
  echo "$APPD init failed:" >&2
  tail -n 20 "$WORK/init.log" >&2
  exit 4
}
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
# Scenarios recorded without a genesis file (the ibctesting fixtures): one
# from the recorded InitChain.
if [ -f "$IN/genesis.json" ]; then
  cp "$IN/genesis.json" "$H/config/genesis.json"
else
  "$FUZZ_DIFF" genesis "$SCENARIO" >"$H/config/genesis.json"
fi
GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$APPD" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >"$WORK/app.log" 2>&1 &
APP=$!
trap 'kill $APP 2>/dev/null || true; rm -rf "$WORK"' EXIT
UP=
for _ in $(seq 1 300); do
  (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null && UP=1 && break
  kill -0 $APP 2>/dev/null || break
  sleep 1
done
if [ -z "$UP" ]; then
  echo "Go celestia-appd exited or did not listen on $PORT within 300 s" >&2
  tail -n 20 "$WORK/app.log" >&2
  exit 3
fi
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
"$FUZZ_DIFF" pp "$SCENARIO" "127.0.0.1:$PORT" "$PER" "$SEED"
