#!/usr/bin/env bash
# Delivery differential (crates/fuzz-diff/src/deliver.rs): starts a Go
# celestia-appd as a standalone ABCI app, re-drives the recorded scenario
# into it and into the Rust app with funded fuzz accounts added to the
# genesis, then finalizes generated blocks of signed txs on both, comparing
# CheckTx, PrepareProposal, ProcessProposal, every tx result and the app
# hash of every block.
#
# usage: GO_APPD=<celestia-appd of the scenario's version> \
#        crates/fuzz-diff/deliver-diff.sh <scenario> [blocks] [txs-per-block] [seed]
#
# v10 scenarios need celestia-appd v10.4.0, v9-* scenarios v9.0.8 (both as
# built by conformance/devnet/devnet.sh into $DEVNET_BIN and $DEVNET_BIN/v9).
# v9-upgrade switches binaries mid-run and is not supported. Environment:
# DELIVER_TYPES=<substr,...> narrows the synthesized message types.
set -euo pipefail
SCENARIO=${1:?scenario}
BLOCKS=${2:-200}
PER=${3:-8}
SEED=${4:-1}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APPD=${GO_APPD:?path to celestia-appd}
PORT=$((37858 + ${DEVNET_PORT_OFFSET:-0}))
IN=$ROOT/conformance/fixtures/devnet/$SCENARIO
WORK=$(mktemp -d)
H=$WORK/home
CHAIN_ID=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['chain_id'])" "$IN/meta.json")
"$APPD" init node --chain-id "$CHAIN_ID" --home "$H" >/dev/null 2>&1
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
# Scenarios recorded without a genesis file (the ibctesting fixtures): one
# from the recorded InitChain.
if [ -f "$IN/genesis.json" ]; then
  cp "$IN/genesis.json" "$H/config/genesis.json"
else
  "$FUZZ_DIFF" genesis "$SCENARIO" >"$H/config/genesis.json"
fi
GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$APPD" start --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >"$WORK/app.log" 2>&1 &
APP=$!
# GO_LOG=<file> keeps the Go app's log.
trap 'kill $APP 2>/dev/null || true; if [ -n "${GO_LOG:-}" ]; then cp "$WORK/app.log" "$GO_LOG"; fi; rm -rf "$WORK"' EXIT
for _ in $(seq 1 60); do
  (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null && break
  sleep 1
done
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
"$FUZZ_DIFF" deliver "$SCENARIO" "127.0.0.1:$PORT" "$BLOCKS" "$PER" "$SEED"
