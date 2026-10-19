#!/usr/bin/env bash
# Go `celestia-appd export` of recorded devnet scenarios, the oracle for
# `celestia-rs export` (crates/devnet-conformance/tests/export_genesis.rs).
#
# Each scenario's recorded ABCI requests are re-driven through a fresh Go
# celestia-appd (no comet, `examples/inject_evidence.rs` without injected
# heights), then `celestia-appd export` writes
# conformance/fixtures/devnet/<scenario>/export.json (state at the last
# committed height).
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/export-fixtures.sh <scenario>...
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-0}
P_APP=$((36658 + OFF))
(cd "$ROOT" && CARGO_INCREMENTAL=0 cargo build -q -p devnet-conformance --example inject_evidence)
for SC in "$@"; do
  IN=$ROOT/conformance/fixtures/devnet/$SC
  WORK=$(mktemp -d)
  H=$WORK/home
  CHAIN_ID=$(python3 -c "import json; print(json.load(open('$IN/meta.json'))['chain_id'])")
  celestia-appd init node --chain-id "$CHAIN_ID" --home "$H" >/dev/null 2>&1
  cp "$IN/genesis.json" "$H/config/genesis.json"
  sed -i.bak 's/^pruning = "default"/pruning = "nothing"/' "$H/config/app.toml"
  rm -f "$H/config/app.toml.bak"
  sed -i.bak -e "s#localhost:9090#localhost:$((9090 + OFF))#" -e "s#0.0.0.0:9090#0.0.0.0:$((9090 + OFF))#" \
    -e "s#tcp://localhost:1317#tcp://localhost:$((1317 + OFF))#" "$H/config/app.toml"
  rm -f "$H/config/app.toml.bak"
  celestia-appd start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$P_APP" >"$WORK/app.log" 2>&1 &
  APP=$!
  sleep 5
  "$ROOT/target/debug/examples/inject_evidence" "$SC" "$WORK/abci.rec" "127.0.0.1:$P_APP" >"$WORK/redrive.log" 2>&1 || true
  sleep 1
  kill "$APP" 2>/dev/null || true
  wait "$APP" 2>/dev/null || true
  if celestia-appd export --home "$H" >"$WORK/export.json" 2>"$WORK/export.err"; then
    python3 - "$WORK/export.json" "$IN/export.json" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
json.dump(d, open(sys.argv[2], "w"), indent=1, sort_keys=True)
print(sys.argv[2], "height", d["initial_height"] - 1)
PY
    rm -rf "$WORK"
  else
    echo "export of $SC failed; logs in $WORK" >&2
    tail -5 "$WORK/export.err" >&2
  fi
done
