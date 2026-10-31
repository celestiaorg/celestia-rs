#!/usr/bin/env bash
set -euo pipefail

# Rebuild both CORS modes and the complete evidence manifest. APPD must be the
# exact tagged/version-stamped v10.5.0 oracle used by the checked-in capture.
APPD=${APPD:?set APPD to the v10.5.0 celestia-appd oracle}
RPC_PORT=${RPC_PORT:-28657}
ROOT=$(cd "$(dirname "$0")" && pwd); OUT=$ROOT/testdata
cd "$ROOT"
EXPECTED_APPD=eba70d65dde2f6082606e5e4c946a2c41736435e409bb8c69398ca5665847b38
ACTUAL_APPD=$(sha256sum "$APPD" | cut -d' ' -f1)
test "$ACTUAL_APPD" = "$EXPECTED_APPD" || { echo "unexpected APPD SHA-256: $ACTUAL_APPD" >&2; exit 1; }
go version -m "$APPD" | grep -q 'github.com/celestiaorg/celestia-app/v10/cmd/celestia-appd'

P2P_PORT=$((RPC_PORT - 1)); PPROF_PORT=$((RPC_PORT - 20597))
GRPC_PORT=$((RPC_PORT - 17567)); API_PORT=$((RPC_PORT - 25340))
WORK=$(mktemp -d); HOME_DIR=$WORK/home; PID=
cleanup() { [ -z "$PID" ] || kill "$PID" 2>/dev/null || true; rm -rf "$WORK"; }
trap cleanup EXIT
stop_node() {
  [ -z "$PID" ] || kill "$PID" 2>/dev/null || true
  [ -z "$PID" ] || wait "$PID" 2>/dev/null || true
  PID=
}
start_node() {
  "$APPD" start --force-no-bbr --home "$HOME_DIR" >>"$WORK/node.log" 2>&1 & PID=$!
  for _ in $(seq 1 30); do
    if curl -fsS "http://127.0.0.1:$RPC_PORT/status" >/dev/null 2>&1; then return; fi
    kill -0 "$PID"; sleep 1
  done
  return 1
}

mkdir -p "$OUT/oracle"
"$APPD" init ws190 --chain-id ws190-105-1 --home "$HOME_DIR" >/dev/null
for key in val0 user0 user1; do "$APPD" keys add "$key" --home "$HOME_DIR" --keyring-backend test >/dev/null 2>&1; done
for key in val0 user0 user1; do
  address=$("$APPD" keys show "$key" --address --home "$HOME_DIR" --keyring-backend test)
  "$APPD" genesis add-genesis-account "$address" 1000000000000utia --home "$HOME_DIR" >/dev/null
done
"$APPD" genesis gentx val0 5000000000utia --chain-id ws190-105-1 \
  --keyring-backend test --fees 500000utia --home "$HOME_DIR" >/dev/null
"$APPD" genesis collect-gentxs --home "$HOME_DIR" >/dev/null

python3 - "$HOME_DIR/config/config.toml" "$HOME_DIR/config/app.toml" \
  "$RPC_PORT" "$P2P_PORT" "$PPROF_PORT" "$GRPC_PORT" "$API_PORT" <<'PY'
import pathlib,re,sys
config,app=map(pathlib.Path,sys.argv[1:3]);rpc,p2p,pprof,grpc,api=sys.argv[3:]
s=config.read_text().replace("tcp://127.0.0.1:26657",f"tcp://127.0.0.1:{rpc}")
s=s.replace("tcp://0.0.0.0:26656",f"tcp://127.0.0.1:{p2p}").replace("localhost:6060",f"127.0.0.1:{pprof}")
values={"cors_allowed_origins":'["https://allowed.example"]',"max_subscription_clients":"3",
 "experimental_subscription_buffer_size":"100","experimental_websocket_write_buffer_size":"100",
 "experimental_close_on_slow_client":"true","max_body_bytes":"1000000"}
for name,value in values.items():s=re.sub(rf"^{name} = .*$",f"{name} = {value}",s,flags=re.M)
config.write_text(s)
s=app.read_text().replace("localhost:9090",f"127.0.0.1:{grpc}")
s=s.replace("0.0.0.0:9090",f"127.0.0.1:{grpc}").replace("tcp://localhost:1317",f"tcp://127.0.0.1:{api}")
app.write_text(s)
PY
cp "$HOME_DIR/config/genesis.json" "$OUT/oracle/genesis.json"
cp "$HOME_DIR/config/config.toml" "$OUT/oracle/config-cors-enabled.toml"
cp "$HOME_DIR/config/app.toml" "$OUT/oracle/app.toml"

gh api repos/celestiaorg/celestia-app/releases/latest >"$WORK/release-api.json"
tag_object=$(git ls-remote https://github.com/celestiaorg/celestia-app.git refs/tags/v10.5.0 | cut -f1)
python3 - "$WORK/release-api.json" "$OUT/release.json" "$tag_object" "$(date -u +%Y-%m-%dT%H:%M:%SZ)" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]));out={'repository':'celestiaorg/celestia-app','latest_release':r['tag_name'],
 'published_at':r['published_at'],'target_commitish':r['target_commitish'],'tag_object':sys.argv[3],
 'verified_at':sys.argv[4],'commands':['gh api repos/celestiaorg/celestia-app/releases/latest',
 'git ls-remote https://github.com/celestiaorg/celestia-app.git refs/tags/v10.5.0']}
json.dump(out,open(sys.argv[2],'w'),indent=2);open(sys.argv[2],'a').write('\n')
PY
started=$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)
endpoint=${MAINNET_RPC:-https://rpc.celestia.pops.one}
curl -fsS "$endpoint/status" -o "$OUT/mainnet-status.json"
status_done=$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)
curl -fsS "$endpoint/abci_info" -o "$OUT/mainnet-abci-info.json"
finished=$(date -u +%Y-%m-%dT%H:%M:%S.%NZ)
python3 - "$OUT/mainnet-observation.json" "$endpoint" "$started" "$status_done" "$finished" <<'PY'
import json,sys
json.dump({'endpoint':sys.argv[2],'started_at':sys.argv[3],'status_completed_at':sys.argv[4],
 'finished_at':sys.argv[5]},open(sys.argv[1],'w'),indent=2);open(sys.argv[1],'a').write('\n')
PY

start_node
go run . -rpc "http://127.0.0.1:$RPC_PORT" -pprof "http://127.0.0.1:$PPROF_PORT" \
  -clients 3 -large-frame-bytes 16777216 -appd "$APPD" -home "$HOME_DIR" \
  -chain-id ws190-105-1 -node "tcp://127.0.0.1:$RPC_PORT" -out "$OUT/go-v10.5.0.json"
stop_node
sed -i 's/cors_allowed_origins = \["https:\/\/allowed.example"\]/cors_allowed_origins = []/' "$HOME_DIR/config/config.toml"
cp "$HOME_DIR/config/config.toml" "$OUT/oracle/config-cors-disabled.toml"
start_node
go run . -upgrades-only -rpc "http://127.0.0.1:$RPC_PORT" \
  -pprof "http://127.0.0.1:$PPROF_PORT" -out "$OUT/go-v10.5.0-cors-disabled.json"
stop_node

go version -m "$APPD" >"$OUT/go-v10.5.0.build-info.txt"
sha256sum "$APPD" >>"$OUT/go-v10.5.0.build-info.txt"
printf '%s\n' 'source-module github.com/celestiaorg/celestia-app/v10@v10.5.0' \
  'source-commit 6013e758d5dceb75827ad191c8d739d7c43cfa04' >>"$OUT/go-v10.5.0.build-info.txt"

python3 - "$ROOT" <<'PY'
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[1]);names=['go.mod','go.sum','main.go','main_test.go','record.sh','verify.py',
 'testdata/go-v10.5.0.json','testdata/go-v10.5.0-cors-disabled.json','testdata/go-v10.5.0.build-info.txt',
 'testdata/release.json','testdata/mainnet-status.json','testdata/mainnet-abci-info.json',
 'testdata/mainnet-observation.json','testdata/oracle/genesis.json','testdata/oracle/config-cors-enabled.toml',
 'testdata/oracle/config-cors-disabled.toml','testdata/oracle/app.toml']
m={'schema':1,'artifacts_sha256':{n:hashlib.sha256((root/n).read_bytes()).hexdigest() for n in names}}
(root/'testdata/manifest.json').write_text(json.dumps(m,indent=2)+'\n')
PY
python3 "$ROOT/verify.py"
