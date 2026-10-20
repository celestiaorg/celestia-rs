#!/usr/bin/env bash
# Record the query corpus of a devnet scenario against Go celestia-appd.
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/querycorpus/record.sh <scenario>...
#
# For each scenario: start a fresh Go celestia-appd (standalone ABCI,
# pruning "nothing", gRPC and REST API enabled), re-drive the scenario's
# recorded ABCI requests into it, then issue the corpus requests over ABCI
# Query and REST (crates/devnet-conformance/examples/record_queries.rs) and
# write conformance/querycorpus/<scenario>.json.gz. The Go node keeps all
# heights, so requests target several historical heights.
#
# Binaries come from $DEVNET_BIN (default target/devnet-bin), as for
# conformance/devnet/devnet.sh; APP_V9=1 uses celestia-app v9.0.8
# ($DEVNET_BIN/v9/celestia-appd) for the v9-* scenarios. PROFILE=release
# builds the recorder in release mode (default: dev).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
APPD=$BIN/celestia-appd
[ -n "${APP_V9:-}" ] && APPD=$BIN/v9/celestia-appd
PROFILE=${PROFILE:-dev}
OFF=${DEVNET_PORT_OFFSET:-0}
P_APP=$((36658 + OFF)); P_API=$((1317 + OFF)); P_GRPC=$((9090 + OFF)); P_RPC=$((26657 + OFF))

(cd "$ROOT" && CARGO_INCREMENTAL=0 cargo build -q --profile "$PROFILE" -p devnet-conformance --example record_queries --example record_grpc_reflection)
OUTDIR=$PROFILE; [ "$PROFILE" = dev ] && OUTDIR=debug
REC=$ROOT/target/$OUTDIR/examples/record_queries

for SCENARIO in "$@"; do
  IN=$ROOT/conformance/fixtures/devnet/$SCENARIO
  WORK=$(mktemp -d)
  H=$WORK/home
  CHAIN_ID=$(python3 -c "import json; print(json.load(open('$IN/meta.json'))['chain_id'])")
  "$APPD" init node --chain-id "$CHAIN_ID" --home "$H" >/dev/null 2>&1
  cp "$IN/genesis.json" "$H/config/genesis.json"
  python3 - "$H/config/app.toml" "$P_API" "$P_GRPC" <<'PY'
import re, sys
p, api, grpc = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
s = s.replace('pruning = "default"', 'pruning = "nothing"')
def section(s, name, f):
    i = s.index("[%s]" % name)
    j = s.find("\n[", i + 1)
    j = len(s) if j < 0 else j
    return s[:i] + f(s[i:j]) + s[j:]
s = section(s, "api", lambda b: b.replace("enable = false", "enable = true", 1)
            .replace("tcp://localhost:1317", "tcp://127.0.0.1:" + api))
s = section(s, "grpc", lambda b: b.replace("enable = false", "enable = true", 1)
            .replace("localhost:9090", "127.0.0.1:" + grpc))
open(p, "w").write(s)
PY
  sed -i.bak -e "s#tcp://127.0.0.1:26657#tcp://127.0.0.1:$P_RPC#" "$H/config/config.toml"
  rm -f "$H/config/config.toml.bak"
  "$APPD" start --force-no-bbr --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP >"$WORK/app.log" 2>&1 &
  APP=$!
  trap 'kill $APP 2>/dev/null || true' EXIT
  for _ in $(seq 1 60); do
    (exec 3<>/dev/tcp/127.0.0.1/$P_APP) 2>/dev/null && break
    sleep 1
  done
  sleep 2
  "$REC" "$SCENARIO" "127.0.0.1:$P_APP" "127.0.0.1:$P_API" "$ROOT/conformance/querycorpus/$SCENARIO.json"
  gzip -9 -n -f "$ROOT/conformance/querycorpus/$SCENARIO.json"
  if [ "$SCENARIO" = empty ]; then
    # v2alpha1 reflection is served by the gRPC server only.
    "$ROOT/target/$OUTDIR/examples/record_grpc_reflection" "127.0.0.1:$P_GRPC" \
      "$ROOT/conformance/querycorpus/reflection-v2alpha1.json"
  fi
  kill $APP 2>/dev/null || true
  wait $APP 2>/dev/null || true
  trap - EXIT
  rm -rf "$WORK"
  echo "wrote conformance/querycorpus/$SCENARIO.json.gz"
done
