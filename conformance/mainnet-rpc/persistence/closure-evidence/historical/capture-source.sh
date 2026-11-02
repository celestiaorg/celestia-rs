#!/usr/bin/env bash
# Fresh pre-v10 queries through the official current-release multiplexer.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:?pinned helper binaries}
RS=${RS_BIN:?native provider binary}
MUX=${MULTIPLEXER_BIN:?official v10.5.0 Linux x86_64 release binary}
V9=${EMBEDDED_V9_BIN:?v9.0.8 binary extracted by the official multiplexer}
OUT=${HISTORICAL_CAPTURE_OUT:?new output directory}
OFF=${DEVNET_PORT_OFFSET:-15000}
W=$(mktemp -d)
FAIL=()
CHAIN_ID=test
mkdir "$OUT"
source "$ROOT/conformance/devnet/lib-mixed.sh"
trap 'cleanup; echo "node logs: $W"' EXIT
export PATH=$(dirname "$V9"):$BIN:$PATH
D=$W/network
mkdir "$D"
make_net "$D" "$CHAIN_ID" "go rs go" "5000000000 0 0"
python3 - "$D" <<'PY'
import pathlib,re,sys
for home in pathlib.Path(sys.argv[1]).glob('n*'):
 if not home.is_dir():continue
 p=home/'config/app.toml';s=p.read_text();s=re.sub(r'^pruning = .*$', 'pruning = "nothing"',s,flags=re.M)
 s=re.sub(r'(\[api\]\n[^\[]*?^enable = )false',r'\g<1>true',s,flags=re.M)
 s=re.sub(r'(\[grpc\]\n[^\[]*?^enable = )false',r'\g<1>true',s,flags=re.M);p.write_text(s)
 p=home/'config/config.toml';s=p.read_text();s=re.sub(r'^indexer = .*$','indexer = "kv"',s,flags=re.M);s=re.sub(r'^discard_abci_responses = .*$','discard_abci_responses = false',s,flags=re.M);p.write_text(s)
PY
# Every running Go node is the current official multiplexer; v9 is only the CLI.
mkdir "$W/current"
ln -s "$MUX" "$W/current/celestia-appd"
export PATH=$W/current:$BIN:$PATH
height() {
 curl -fsS --max-time 5 "$(rpc "$1" "$2")/status" 2>/dev/null | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["sync_info"]["latest_block_height"])' 2>/dev/null || echo 0
}
wait_height() {
 local deadline=$((SECONDS+$4))
 while [ "$SECONDS" -lt "$deadline" ]; do
  kill -0 "$(cat "$1/n$2/pid")" 2>/dev/null || { tail -20 "$1/n$2/node.log" >&2; return 1; }
  [ "$(height "$1" "$2")" -ge "$3" ] && return 0
  sleep 1
 done
 return 1
}
start_node() {
 local h=$1/n$2
 if [ "$(cat "$h/kind")" = go ]; then
  "$MUX" start --force-no-bbr --home "$h" --address "tcp://127.0.0.1:$((36658+OFF+10*$2))" >> "$h/node.log" 2>&1 &
 else
  RUST_LOG=${RUST_LOG:-info} "$RS" start --home "$h" --p2p-laddr "127.0.0.1:$((26656+OFF+10*$2))" --persistent-peers "$(cat "$h/peers")" --moniker "node$2" --allow-duplicate-ip >> "$h/node.log" 2>&1 &
 fi
 echo $! > "$h/pid"; PIDS+=($!)
}
for i in 0 1 2; do start_node "$D" "$i"; done
ensure_up "$D" 0 1 2
for i in 0 1 2; do wait_height "$D" "$i" 3 180; done
HOME_DIR=$D/n0 CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$((26657+OFF)) \
 PATH=$(dirname "$V9"):$PATH bash "$ROOT/conformance/devnet/workloads/v9-upgrade.sh" > "$OUT/workload.log" 2>&1
for i in 0 1 2; do wait_height "$D" "$i" 30 240; done
kill_node "$D" 0
sleep 3
TIP=$(height "$D" 2)
for i in 1 2; do wait_height "$D" "$i" "$TIP" 90; done
SENDER=$("$V9" keys show user0 -a --home "$D/n0" --keyring-backend test)
python3 - "$(rpc "$D" 2)" "$OUT/selection.json" <<'PY'
import base64,hashlib,json,sys,urllib.request
base,out=sys.argv[1:]
def rpc(path):return json.load(urllib.request.urlopen(base+path,timeout=10))['result']
info=rpc('/abci_info')['response'];tip=int(info['last_block_height'])
assert info['version']=='10.5.0' and info['app_version']=='10'
versions=[];hashes=[]
for h in range(1,tip+1):
 b=rpc('/block?height='+str(h))['block'];versions.append((h,int(b['header']['version']['app'])))
 hashes.extend(hashlib.sha256(base64.b64decode(tx)).hexdigest().upper() for tx in b['data']['txs'] or [])
upgrade=next(h-1 for h,v in versions if v==10)
assert upgrade>2 and hashes and versions[0][1]==9
json.dump({'tip':tip,'upgrade_height':upgrade,'heights':sorted(set([1,upgrade-1,upgrade,upgrade+1,tip-1])),'tx':hashes[-1],'info':info,'versions':versions},open(out,'w'),indent=2)
PY
HASH=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["tx"])' "$OUT/selection.json")
for H in $(python3 -c 'import json,sys; print(*json.load(open(sys.argv[1]))["heights"])' "$OUT/selection.json"); do
 python3 "$ROOT/conformance/mainnet-rpc/persistence/capture.py" \
  --go "$(rpc "$D" 2)" --rs "$(rpc "$D" 1)" --height "$H" --sender "$SENDER" --tx "$HASH" --chain-id "$CHAIN_ID" --out "$OUT/height-$H-comet.json" || FAIL+=("height-$H-comet")
 persistent-api --go-grpc "127.0.0.1:$((9090+OFF+20))" --rs-grpc "127.0.0.1:$((9090+OFF+10))" \
  --go-rest "http://127.0.0.1:$((1317+OFF+20))" --rs-rest "http://127.0.0.1:$((1317+OFF+10))" \
  --sender "$SENDER" --height "$H" --tip "$TIP" --out "$OUT/height-$H-api.json" || FAIL+=("height-$H-api")
done
for i in 1 2; do kill_node "$D" "$i"; done
rpc-recorder dump -data "$D/n0/data" -out "$OUT/blocks.bin.gz"
cp "$D/n0/config/genesis.json" "$OUT/genesis.json"
for i in 0 1 2; do
 cp "$D/n$i/config/config.toml" "$OUT/config-$i.toml"
 cp "$D/n$i/config/app.toml" "$OUT/app-$i.toml"
 cp "$D/n$i/node.log" "$OUT/node-$i.log"
done
cp "$MAINNET_OBSERVATION" "$OUT/mainnet-observation.json"
go version -m "$MUX" > "$OUT/multiplexer-build-info.txt"
sha256sum "$MUX" "$RS" > "$OUT/binary-sha256.txt"
cp "$ROOT/conformance/devnet/historical-multiplexer.sh" "$OUT/capture-source.sh"
if [ "${#FAIL[@]}" -ne 0 ]; then printf 'FAILED: %s\n' "${FAIL[@]}"; exit 1; fi
