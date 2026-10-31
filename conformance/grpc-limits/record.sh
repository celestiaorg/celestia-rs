#!/usr/bin/env bash
# Fresh, bounded, loopback-only transport capture. Creates its own validator.
# Requires a reviewed v10.5.0 ledger oracle and the matching .build.json stamp.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ORACLE_BIN=${ORACLE_BIN:?set ORACLE_BIN to the reviewed celestia-appd binary}
OUT=${OUT:?set OUT to an empty capture directory}
OFF=${DEVNET_PORT_OFFSET:-5000}
export GOTOOLCHAIN=${GOTOOLCHAIN:-go1.26.9}
mkdir -p "$OUT"
[ -z "$(ls -A "$OUT")" ] || { echo "OUT must be empty" >&2; exit 1; }
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d)
NODE_PID=
stop_node() {
  if [ -n "$NODE_PID" ]; then
    kill -TERM "$NODE_PID" 2>/dev/null || true
    wait "$NODE_PID" || true
    NODE_PID=
  fi
}
cleanup() { stop_node; rm -rf "$WORK"; }
trap cleanup EXIT
python3 - "$ORACLE_BIN" <<'PY'
import hashlib,json,pathlib,sys
p=pathlib.Path(sys.argv[1]); meta=json.loads(pathlib.Path(str(p)+'.build.json').read_text())
assert meta['module']=='github.com/celestiaorg/celestia-app/v10@v10.5.0'
assert meta['source_commit']=='6013e758d5dceb75827ad191c8d739d7c43cfa04'
assert meta['build_tags']==['ledger']
assert meta['binary_sha256']==hashlib.sha256(p.read_bytes()).hexdigest()
PY
# Re-verify the selected release against a read-only mainnet endpoint; local
# captures never submit to it. These observations are part of provenance.
curl --fail --silent --show-error --max-time 30 "${MAINNET_RPC:-https://rpc.celestia.pops.one}/abci_info" > "$OUT/mainnet-abci-info.json"
curl --fail --silent --show-error --max-time 30 "${MAINNET_RPC:-https://rpc.celestia.pops.one}/status" > "$OUT/mainnet-status.json"
python3 - "$OUT" <<'PY'
import json,pathlib,sys
p=pathlib.Path(sys.argv[1]);assert json.loads((p/'mainnet-abci-info.json').read_text())['result']['response']['version']=='10.5.0'
assert json.loads((p/'mainnet-status.json').read_text())['result']['node_info']['network']=='celestia'
PY
HOME_DIR=$WORK/node
"$ORACLE_BIN" init grpc-limits --chain-id rs-grpc-limits-1 --home "$HOME_DIR" >/dev/null 2>&1
"$ORACLE_BIN" keys add validator --keyring-backend test --home "$HOME_DIR" >/dev/null 2>&1
ADDR=$("$ORACLE_BIN" keys show validator -a --keyring-backend test --home "$HOME_DIR")
"$ORACLE_BIN" genesis add-genesis-account "$ADDR" 100000000000000utia --home "$HOME_DIR" >/dev/null 2>&1
"$ORACLE_BIN" genesis gentx validator 5000000000utia --chain-id rs-grpc-limits-1 --keyring-backend test --home "$HOME_DIR" --fees 500000utia >/dev/null 2>&1
"$ORACLE_BIN" genesis collect-gentxs --home "$HOME_DIR" >/dev/null 2>&1
python3 - "$HOME_DIR" "$OFF" <<'PY'
import pathlib,re,sys
home=pathlib.Path(sys.argv[1]);off=int(sys.argv[2])
for name in ['config.toml','app.toml']:
 p=home/'config'/name;s=p.read_text()
 s=re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m:m[1]+str(int(m[2])+off),s)
 if name=='config.toml':
  s=re.sub(r'^grpc_laddr = .*$',f'grpc_laddr = "tcp://127.0.0.1:{26658+off}"',s,flags=re.M)
  s=re.sub(r'^pex = .*$', 'pex = false',s,flags=re.M)
 else:
  start=s.index('[grpc]');end=s.index('\n[',start+1);part=s[start:end]
  part=re.sub(r'^enable = .*$', 'enable = true',part,flags=re.M)
  part=re.sub(r'^max-recv-msg-size = .*$', 'max-recv-msg-size = "64"',part,flags=re.M)
  part=re.sub(r'^max-send-msg-size = .*$', 'max-send-msg-size = "128"',part,flags=re.M)
  s=s[:start]+part+s[end:]
 p.write_text(s)
PY
(cd "$ROOT/conformance/grpc-limits/recorder" && go build -buildvcs=false -o "$WORK/record" .)
start_node() {
  "$ORACLE_BIN" start --force-no-bbr --home "$HOME_DIR" >> "$WORK/node.log" 2>&1 &
  NODE_PID=$!
  for attempt in $(seq 1 60); do
    if curl --fail --silent "http://127.0.0.1:$((26657+OFF))/status" > "$WORK/status.json" \
      && python3 - "$((9090+OFF))" <<'PY'
import socket,sys
try:
 with socket.create_connection(('127.0.0.1',int(sys.argv[1])),timeout=1): pass
except OSError:
 raise SystemExit(1)
PY
    then
      if python3 - "$WORK/status.json" <<'PY'
import json,sys
assert int(json.load(open(sys.argv[1]))['result']['sync_info']['latest_block_height'])>0
PY
      then return; fi
    fi
    sleep 1
  done
  cat "$WORK/node.log" >&2
  return 1
}
record() {
  local name=$1 method=$2
  shift 2
  "$WORK/record" -endpoint "127.0.0.1:$((9090+OFF))" -method "$method" "$@" > "$OUT/$name.json"
}
start_node
record go-sdk-config /cosmos.base.node.v1beta1.Service/Config
record go-sdk-block-status /tendermint.rpc.grpc.BlockAPI/Status
record go-sdk-block-stream /tendermint.rpc.grpc.BlockAPI/BlockByHeight -prefix-hex 0801 -streaming
record go-sdk-unknown-service /missing.Service/Query
record go-sdk-unknown-method /cosmos.bank.v1beta1.Query/Missing
record go-sdk-unknown-block-method /tendermint.rpc.grpc.BlockAPI/Missing
record go-sdk-core-broadcast /tendermint.rpc.grpc.BroadcastAPI/Ping
"$WORK/record" -endpoint "127.0.0.1:$((26658+OFF))" -method /tendermint.rpc.grpc.BroadcastAPI/Ping -sizes 4194303,4194304,4194305 | gzip -n > "$OUT/go-core-request-boundary.json.gz"
cp "$HOME_DIR/config/genesis.json" "$OUT/genesis.json"
cp "$HOME_DIR/config/config.toml" "$OUT/config.toml"
cp "$HOME_DIR/config/app.toml" "$OUT/app.toml"
for limit in 5 6 7; do
  stop_node
  sed -i "s/^max-send-msg-size = .*$/max-send-msg-size = \"$limit\"/" "$HOME_DIR/config/app.toml"
  start_node
  record "go-sdk-send-$limit" /cosmos.base.node.v1beta1.Service/Config
done
stop_node
cp "$ORACLE_BIN.build.json" "$OUT/go-build.json"
go version -m "$ORACLE_BIN" > "$OUT/go-build-info.txt"
python3 - "$ROOT" "$OUT" <<'PY'
import datetime,hashlib,json,pathlib,subprocess,sys
root,out=map(pathlib.Path,sys.argv[1:]);build=json.loads((out/'go-build.json').read_text())
files={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(out.iterdir())}
meta={'schema_version':1,'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),'oracle_build':build,
      'recorder_compiler':subprocess.check_output(['go','version'],text=True).strip(),
      'capture_source_sha256':{name:hashlib.sha256((root/name).read_bytes()).hexdigest() for name in ['conformance/grpc-limits/record.sh','conformance/grpc-limits/verify.py','conformance/grpc-limits/recorder/main.go','conformance/grpc-limits/recorder/go.mod','conformance/grpc-limits/recorder/go.sum']},
      'sha256':files,
      'state_scope':'fresh isolated Go single-validator devnet; Config success height is dynamic, BlockAPI size includes its own runtime metadata; no matched-state universal parity claim',
      'exclusions':['HTTP/2 framing timing and dynamic capture heights','Go/RS build identity and node identity in Status responses','compressed messages, keepalive, handshake deadlines, memory-growth campaigns','connection quotas zero/two and prolonged cancellation/recovery loads']}
(out/'meta.json').write_text(json.dumps(meta,indent=2)+'\n')
PY
python3 "$ROOT/conformance/grpc-limits/verify.py" "$OUT"
