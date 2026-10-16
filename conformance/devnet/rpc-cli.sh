#!/usr/bin/env bash
# Go client tooling against the Rust node's RPC (ADR 0003, crate `rpc`):
# a single-validator `celestia-rs start` node (genesis from `celestia-appd
# init` + gentx, so config.toml/app.toml carry the Go defaults, ports
# shifted) is driven only through Go celestia-appd v10.4.0's CLI:
#
#  - `status`, `query block/blocks/block-results`, bank/auth/staking
#    queries (ABCI query over JSON-RPC), `query tx`, `query txs`;
#  - `tx bank send` in sync and async broadcast modes, `query wait-tx`
#    (WebSocket subscription), `tx blob pay-for-blob`;
#  - the blob and staking devnet workloads (CLI broadcasts + RPC polling).
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/rpc-cli.sh
# Exit 0 iff all checks pass.
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=$((${DEVNET_PORT_OFFSET:-16000} + 400))
CHAIN_ID=rs-rpc-cli-1
W=$(mktemp -d)
H=$W/home
RS="$ROOT/target/debug/celestia-rs"

(cd "$ROOT" && cargo build -q -p celestia-rs)

A="celestia-appd --home $H"
K=(--keyring-backend test --home "$H")
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
for k in val0 user0; do
  $A keys add $k --keyring-backend test >/dev/null 2>&1
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
python3 - "$H/config/config.toml" "$H/config/app.toml" "$OFF" <<'PY'
import re, sys
*files, d = sys.argv[1:]
for p in files:
    s = open(p).read()
    s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + int(d)), s)
    open(p, "w").write(s)
PY
# tx/block indexing (`celestia-appd init` writes indexer = "null")
sed -i.bak 's#^indexer = "null"#indexer = "kv"#' "$H/config/config.toml"
rm -f "$H/config/config.toml.bak"
P_RPC=$((26657 + OFF))
NODE=tcp://127.0.0.1:$P_RPC
RPC=http://127.0.0.1:$P_RPC

PID=
cleanup() { [ -n "$PID" ] && kill "$PID" 2>/dev/null; wait 2>/dev/null || true; rm -rf "$W"; }
trap cleanup EXIT
fail() { echo "FAILED: $*"; tail -n 30 "$W/node.log" | sed 's/\x1b\[[0-9;]*m//g'; exit 1; }
json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
height() { curl -s "$RPC/status" | json 'd["result"]["sync_info"]["latest_block_height"]' 2>/dev/null || echo 0; }

"$RS" start --home "$H" > "$W/node.log" 2>&1 & PID=$!
for _ in $(seq 1 120); do
  [ "$(height)" -ge 3 ] 2>/dev/null && break
  kill -0 $PID 2>/dev/null || fail "node exited"
  sleep 0.5
done
[ "$(height)" -ge 3 ] || fail "node did not reach height 3"
echo "rust node at height $(height), RPC $RPC"

USER0=$($A keys show user0 -a --keyring-backend test)
VAL0=$($A keys show val0 -a --keyring-backend test)
VALOPER=$($A keys show val0 --bech val -a --keyring-backend test)
q() { celestia-appd query "$@" --node "$NODE" -o json; }
check() { # <python expr over d> <description> <json>
  python3 -c "import json,sys; d=json.loads(sys.argv[1]); assert ($1)" "$3" 2>/dev/null \
    || fail "$2: $(echo "$3" | head -c 400)"
  echo "ok  $2"
}

check 'd["node_info"]["network"]=="'$CHAIN_ID'" and int(d["sync_info"]["latest_block_height"])>=3' "status" \
  "$(celestia-appd status --node "$NODE" 2>&1)"
check 'd["header"]["height"]=="2"' "query block --type=height 2" "$(q block --type=height 2)"
check 'int(d["total_count"])>=2 and len(d["blocks"])==2' "query blocks (block_search)" \
  "$(q blocks --query "block.height >= 2" --limit 2)"
check '"txs_results" in d or "finalize_block_events" in d or "height" in d' "query block-results 2" "$(q block-results 2)"
check 'any(c["denom"]=="utia" for c in d["balances"])' "query bank balances" "$(q bank balances "$USER0")"
check 'd["account"]["value"]["address"]=="'$USER0'"' "query auth account" "$(q auth account "$USER0")"
check 'd["validator"]["operator_address"]=="'$VALOPER'"' "query staking validator" "$(q staking validator "$VALOPER")"
check '"params" in d' "query blob params" "$(q blob params)"

send() { # <mode>
  celestia-appd tx bank send user0 "$VAL0" 1000utia "${K[@]}" --chain-id $CHAIN_ID --node "$NODE" \
    --fees 20000utia --gas 200000 --broadcast-mode "$1" -y -o json
}
OUT=$(send sync)
check 'd["code"]==0 and len(d["txhash"])==64' "tx bank send (sync)" "$OUT"
HASH=$(echo "$OUT" | json 'd["txhash"]')
check 'd["code"]==0 and int(d["height"])>0' "query wait-tx (websocket)" "$(q wait-tx "$HASH" --timeout 60s)"
check 'd["code"]==0 and d["txhash"]=="'$HASH'"' "query tx" "$(q tx "$HASH")"
check 'int(d["total_count"])>=1 and any(t["txhash"]=="'$HASH'" for t in d["txs"])' "query txs (tx_search)" \
  "$(q txs --query "message.sender='$USER0'")"
OUT=$(send async)
check 'd["code"]==0 and len(d["txhash"])==64' "tx bank send (async)" "$OUT"
HASH=$(echo "$OUT" | json 'd["txhash"]')
check 'd["code"]==0' "query wait-tx (async tx)" "$(q wait-tx "$HASH" --timeout 60s)"
# rejected by CheckTx: insufficient fee
OUT=$(celestia-appd tx bank send user0 "$VAL0" 1utia "${K[@]}" --chain-id $CHAIN_ID --node "$NODE" \
  --fees 1utia --gas 200000 -y -o json 2>&1 || true)
check 'd["code"]==13' "tx with insufficient fee rejected (code 13)" "$OUT"
OUT=$(celestia-appd tx blob pay-for-blob 0x00000000000000c0ffee 0xdeadbeef \
  "${K[@]}" --from user0 --chain-id $CHAIN_ID --node "$NODE" --fees 40000utia --gas 400000 -y -o json)
check 'd["code"]==0' "tx blob pay-for-blob" "$OUT"
HASH=$(echo "$OUT" | json 'd["txhash"]')
check 'd["code"]==0 and any(e["type"]=="celestia.blob.v1.EventPayForBlobs" for e in d["events"])' \
  "query wait-tx (pay-for-blob)" "$(q wait-tx "$HASH" --timeout 60s)"

for w in blob staking; do
  HOME_DIR=$H CHAIN_ID=$CHAIN_ID NODE=$NODE bash "$ROOT/conformance/devnet/workloads/$w.sh" > "$W/$w.log" 2>&1 \
    || { tail -n 20 "$W/$w.log"; fail "$w workload"; }
  echo "ok  $w workload ($(grep -c . "$W/$w.log") lines of output) at height $(height)"
done
kill -0 $PID 2>/dev/null || fail "node exited"
echo "rpc-cli: all checks passed (height $(height))"
