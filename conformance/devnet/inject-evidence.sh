#!/usr/bin/env bash
# Derive a misbehavior scenario from a recorded one: replay its ABCI
# requests into a fresh Go celestia-appd with duplicate-vote evidence
# injected at the given heights (see crates/devnet-conformance/examples/
# inject_evidence.rs), record the Go responses and dump committed state.
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/devnet/inject-evidence.sh <src-scenario> <out-scenario> <height>...
set -euo pipefail

SRC=${1:?src scenario}
DST=${2:?out scenario}
shift 2
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-0}
P_APP=$((36658 + OFF))
IN=$ROOT/conformance/fixtures/devnet/$SRC
OUT=$ROOT/conformance/fixtures/devnet/$DST
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
celestia-appd start --force-no-bbr --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP >"$WORK/app.log" 2>&1 &
APP=$!
trap 'kill $APP 2>/dev/null || true' EXIT
sleep 5
(cd "$ROOT" && CARGO_INCREMENTAL=0 cargo run -q -p devnet-conformance --example inject_evidence -- \
  "$SRC" "$WORK/abci.rec" "127.0.0.1:$P_APP" "$@")
sleep 1
kill $APP
wait $APP 2>/dev/null || true
LAST=$(python3 - "$WORK/abci.rec" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
i = n = 0
while i < len(d):
    conn, dr, ln = struct.unpack("<IBI", d[i:i+9]); body = d[i+9:i+9+ln]; i += 9 + ln
    if dr == 0 and body[:1] == b"\x5a": n += 1
print(n)
PY
)
rm -rf "$OUT" && mkdir -p "$OUT/state"
cp "$IN/genesis.json" "$OUT/genesis.json"
cp "$WORK/abci.rec" "$OUT/abci.rec"
for h in 1 "$LAST"; do
  devnet-oracle statedump --db "$H/data/application.db" --height "$h" >"$OUT/state/$h.json"
done
cat >"$OUT/meta.json" <<JSON
{
  "scenario": "$DST",
  "chain_id": "$CHAIN_ID",
  "derived_from": "$SRC",
  "injected_misbehavior_heights": "$*",
  "committed_heights": $LAST,
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
rm -rf "$WORK"
echo "wrote $OUT (heights 1..$LAST)"
