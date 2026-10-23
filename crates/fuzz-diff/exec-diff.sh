#!/usr/bin/env bash
# Execution differential (crates/fuzz-diff/src/exec.rs): starts a Go
# celestia-appd as a standalone ABCI app on the scenario's genesis, then
# re-drives the recorded scenario into it and into the Rust app, comparing
# Simulate results of mutated txs (invalid UTF-8 strings) after every Commit.
#
# usage: GO_APPD=<celestia-appd of the scenario's version> \
#        crates/fuzz-diff/exec-diff.sh <scenario> [per-height] [seed]
#
# v10 scenarios need celestia-appd v10.5.0, v9-* scenarios v9.0.8 (both as
# built by conformance/devnet/devnet.sh into $DEVNET_BIN and $DEVNET_BIN/v9).
# v9-upgrade switches binaries mid-run and is not supported (deliver-diff.sh
# supports it: GO_APPD_NEXT).
set -euo pipefail
SCENARIO=${1:?scenario}
PER=${2:-20}
SEED=${3:-1}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APPD=${GO_APPD:?path to celestia-appd}
PORT=$((37758 + ${DEVNET_PORT_OFFSET:-0}))
GRPC=$((39590 + ${DEVNET_PORT_OFFSET:-0}))
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
# Simulate is served on the gRPC server only (not through ABCI Query).
python3 - "$H/config/app.toml" "$GRPC" <<'PY'
import re, sys
p, port = sys.argv[1], sys.argv[2]
s = open(p).read()
i = s.index("[grpc]")
j = s.index("[", i + 1)
sec = re.sub(r'(?m)^enable = .*$', 'enable = true', s[i:j])
sec = re.sub(r'(?m)^address = .*$', 'address = "127.0.0.1:%s"' % port, sec)
open(p, "w").write(s[:i] + sec + s[j:])
PY
GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$APPD" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >"$WORK/app.log" 2>&1 &
APP=$!
# The Go app must be gone before the next run binds the same ports: its
# shutdown can stall, so SIGKILL it after 10 s.
stop_app() {
  kill "$APP" 2>/dev/null || return 0
  for _ in $(seq 1 100); do
    kill -0 "$APP" 2>/dev/null || return 0
    sleep 0.1
  done
  kill -9 "$APP" 2>/dev/null || true
  while kill -0 "$APP" 2>/dev/null; do sleep 0.1; done
}
trap 'stop_app; rm -rf "$WORK"' EXIT
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
"$FUZZ_DIFF" exec "$SCENARIO" "127.0.0.1:$PORT" "127.0.0.1:$GRPC" "$PER" "$SEED"
