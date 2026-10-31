#!/usr/bin/env bash
# Fresh bounded Go/RS gRPC lifecycle capture on isolated loopback nodes.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
ORACLE_BIN=${ORACLE_BIN:?set ORACLE_BIN to reviewed celestia-appd}
CORE_BIN=${CORE_BIN:?set CORE_BIN to reviewed cometbft source-build receipt binary}
RS_BIN=${RS_BIN:?set RS_BIN to the signed-source celestia-rs binary}
RS_BUILD_RECEIPT=${RS_BUILD_RECEIPT:?set RS_BUILD_RECEIPT to the receipt produced by build-rs.sh}
OBSERVATION=${MAINNET_OBSERVATION:?set MAINNET_OBSERVATION to a fresh read-only observation}
OUT=${OUT:?set OUT to an empty output directory}
GO_OFF=${DEVNET_PORT_OFFSET:-6000}
[[ "$GO_OFF" =~ ^[0-9]+$ ]] && [ "$GO_OFF" -le 28000 ] \
  || { echo "DEVNET_PORT_OFFSET must be an integer from 0 through 28000" >&2; exit 1; }
RS_OFF=$((GO_OFF + 1000))
export GOTOOLCHAIN=${GOTOOLCHAIN:-go1.26.9}
mkdir -p "$OUT"
[ -z "$(ls -A "$OUT")" ] || { echo "OUT must be empty" >&2; exit 1; }
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d)
NODE_PID=
stop_node() {
  if [ -n "$NODE_PID" ]; then
    kill -TERM "$NODE_PID" 2>/dev/null || true
    ( sleep 10; kill -KILL "$NODE_PID" 2>/dev/null || true ) &
    local watchdog=$!
    wait "$NODE_PID" 2>/dev/null || true
    kill "$watchdog" 2>/dev/null || true
    wait "$watchdog" 2>/dev/null || true
    NODE_PID=
  fi
}
cleanup() { stop_node; rm -rf "$WORK"; }
trap cleanup EXIT

# Fail before selecting or starting an oracle unless the observation and both
# Go build receipts are exact and fresh. No request is ever sent to mainnet here.
python3 - "$OBSERVATION" "$ORACLE_BIN" "$CORE_BIN" "$RS_BIN" \
  "$RS_BUILD_RECEIPT" "$ROOT" <<'PY'
import datetime,hashlib,json,pathlib,subprocess,sys
observation,app_binary,core_binary,rs_binary,rs_receipt,repo=map(pathlib.Path,sys.argv[1:])
value=json.loads(observation.read_text());now=datetime.datetime.now(datetime.timezone.utc)
observed=datetime.datetime.fromisoformat(value['observed_at'])
assert value['chain_id']=='celestia' and value['version']=='10.5.0'
assert observed.tzinfo and observed<=now and now-observed<=datetime.timedelta(hours=24)
parsed={}
for name in ('status','abci_info'):
 raw=value['responses'][name]['body'].encode()
 assert hashlib.sha256(raw).hexdigest()==value['responses'][name]['sha256']
 parsed[name]=json.loads(raw)
status=parsed['status']['result'];info=parsed['abci_info']['result']['response']
assert status['node_info']['network']=='celestia'
assert info['version']=='10.5.0'
assert status['node_info']['protocol_version']['app']==info['app_version']
assert status['sync_info']['latest_block_height']==info['last_block_height']
for binary,module,commit,tags in (
 (app_binary,'github.com/celestiaorg/celestia-app/v10@v10.5.0','6013e758d5dceb75827ad191c8d739d7c43cfa04',['ledger']),
 (core_binary,'github.com/celestiaorg/celestia-core@v0.42.3','a5537d84a008d566abc503d2ea41e30f2301403d',[])):
 receipt=json.loads(pathlib.Path(str(binary)+'.build.json').read_text())
 assert receipt['module']==module and receipt['source_commit']==commit
 assert receipt['compiler']=='go version go1.26.9 linux/amd64' and receipt['build_tags']==tags
 assert receipt['binary_sha256']==hashlib.sha256(binary.read_bytes()).hexdigest()
rs=json.loads(rs_receipt.read_text())
assert rs_receipt.resolve()==pathlib.Path(str(rs_binary.resolve())+'.build.json')
assert set(rs)=={
 'schema_version','built_at','binary_sha256','source_commit','source_commit_signature_verified',
 'cargo_lock_sha256','rustc_vv','cargo_version','target','build_command','profile',
 'package','binary','locked','default_features','features','version_long',
}
assert rs['schema_version']==1 and rs['source_commit_signature_verified'] is True
assert datetime.datetime.fromisoformat(rs['built_at'])<=now
assert rs['binary_sha256']==hashlib.sha256(rs_binary.read_bytes()).hexdigest()
head=subprocess.check_output(['git','-C',str(repo),'rev-parse','HEAD'],text=True).strip()
assert rs['source_commit']==head
subprocess.run(['git','-C',str(repo),'verify-commit',head],check=True,
 stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
assert subprocess.check_output(['git','-C',str(repo),'status','--porcelain'],text=True).strip()==''
assert rs['cargo_lock_sha256']==hashlib.sha256((repo/'Cargo.lock').read_bytes()).hexdigest()
assert rs['target']=='x86_64-unknown-linux-gnu'
assert 'release: 1.98.0' in rs['rustc_vv']
assert 'host: x86_64-unknown-linux-gnu' in rs['rustc_vv']
assert rs['cargo_version'].startswith('cargo 1.98.0 ')
assert rs['build_command']==['cargo','build','--locked','--release','--package','celestia-rs','--bin','celestia-rs']
assert rs['profile']=='release' and rs['locked'] is True
assert rs['package']==rs['binary']=='celestia-rs'
assert rs['default_features'] is True and rs['features']==['rocksdb','jemalloc']
version=subprocess.check_output([str(rs_binary),'version','--long'],text=True).strip()
assert rs['version_long']==version and f'commit: {head}' in version.splitlines()
PY

(cd "$ROOT/conformance/grpc-lifecycle/recorder" && \
  go build -buildvcs=false -o "$WORK/recorder" .)
GO_HOME=$WORK/go-home
RS_HOME=$WORK/rs-home
"$ORACLE_BIN" init grpc-limits --chain-id rs-grpc-limits-1 --home "$GO_HOME" >/dev/null 2>&1
"$ORACLE_BIN" keys add validator --keyring-backend test --home "$GO_HOME" >/dev/null 2>&1
ADDR=$("$ORACLE_BIN" keys show validator -a --keyring-backend test --home "$GO_HOME")
"$ORACLE_BIN" genesis add-genesis-account "$ADDR" 100000000000000utia --home "$GO_HOME" >/dev/null 2>&1
"$ORACLE_BIN" genesis gentx validator 5000000000utia --chain-id rs-grpc-limits-1 \
  --keyring-backend test --home "$GO_HOME" --fees 500000utia >/dev/null 2>&1
"$ORACLE_BIN" genesis collect-gentxs --home "$GO_HOME" >/dev/null 2>&1
cp "$GO_HOME/config/config.toml" "$WORK/go-config.base.toml"
cp "$GO_HOME/config/app.toml" "$WORK/go-app.base.toml"

patch_go() {
  local quota=$1
  python3 - "$GO_HOME" "$GO_OFF" "$quota" <<'PY'
import ipaddress,pathlib,re,sys,tomllib
home=pathlib.Path(sys.argv[1]);off=int(sys.argv[2]);quota=int(sys.argv[3])
work=home.parent
config=home/'config/config.toml';text=(work/'go-config.base.toml').read_text()
text,count=re.subn(r'(?:127\.0\.0\.1|0\.0\.0\.0|localhost):(\d{4,5})',lambda m:'127.0.0.1:'+str(int(m[1])+off),text)
assert count >= 4
text=re.sub(r'^grpc_laddr = .*$',f'grpc_laddr = "tcp://127.0.0.1:{26658+off}"',text,flags=re.M)
text=re.sub(r'^grpc_max_open_connections = .*$',f'grpc_max_open_connections = {quota}',text,flags=re.M)
text=re.sub(r'^pex = .*$', 'pex = false',text,flags=re.M)
config.write_text(text)
app=home/'config/app.toml';text=(work/'go-app.base.toml').read_text()
text,count=re.subn(r'(?:127\.0\.0\.1|0\.0\.0\.0|localhost):(\d{4,5})',lambda m:'127.0.0.1:'+str(int(m[1])+off),text)
assert count >= 2
start=text.index('[grpc]');end=text.index('\n[',start+1);part=text[start:end]
part=re.sub(r'^enable = .*$', 'enable = true',part,flags=re.M)
part=re.sub(r'^max-recv-msg-size = .*$', 'max-recv-msg-size = "64"',part,flags=re.M)
part=re.sub(r'^max-send-msg-size = .*$', 'max-send-msg-size = "128"',part,flags=re.M)
text=text[:start]+part+text[end:];app.write_text(text)
cfg=tomllib.loads(config.read_text());app_cfg=tomllib.loads(app.read_text())
assert cfg['p2p']['pex'] is False
listeners=[cfg['rpc']['laddr'],cfg['rpc']['grpc_laddr'],cfg['p2p']['laddr']]
if cfg['rpc'].get('pprof_laddr'): listeners.append(cfg['rpc']['pprof_laddr'])
for key in ('priv_validator_laddr','priv_validator_grpc_laddr'):
 if cfg.get(key): listeners.append(cfg[key])
if cfg['instrumentation']['prometheus']:
 listeners.append(cfg['instrumentation']['prometheus_listen_addr'])
for section in ('api','grpc','grpc-web'):
 value=app_cfg.get(section,{})
 if value.get('enable') is True: listeners.append(value['address'])
for value in listeners:
 host,port=value.removeprefix('tcp://').rsplit(':',1)
 host=host.strip('[]')
 assert ipaddress.ip_address(host).is_loopback,(value,host)
 assert 0<int(port)<=65535,value
assert not re.search(r'(?<![0-9])0\.0\.0\.0:|(?:^|[/\[])(?:::|\[::\]):',config.read_text()+app.read_text())
PY
}

wait_ready() {
  local rpc=$1 core=$2 log=$3
  for _ in $(seq 1 90); do
    if ! kill -0 "$NODE_PID" 2>/dev/null; then cat "$log" >&2; return 1; fi
    if curl --fail --silent --max-time 1 "http://$rpc/status" > "$WORK/status.json" \
      && python3 - "$WORK/status.json" "$core" <<'PY'
import json,socket,sys
assert int(json.load(open(sys.argv[1]))['result']['sync_info']['latest_block_height'])>0
host,port=sys.argv[2].rsplit(':',1)
with socket.create_connection((host,int(port)),timeout=1):pass
PY
    then return; fi
    sleep 1
  done
  cat "$log" >&2
  return 1
}

patch_go 0
cp "$GO_HOME/config/genesis.json" "$OUT/genesis.json"
cp "$GO_HOME/config/app.toml" "$OUT/go-app.toml"
for quota in 0 2; do
  patch_go "$quota"
  cp "$GO_HOME/config/config.toml" "$OUT/go-quota-$quota-config.toml"
  "$ORACLE_BIN" start --force-no-bbr --home "$GO_HOME" > "$WORK/go-$quota.log" 2>&1 &
  NODE_PID=$!
  wait_ready "127.0.0.1:$((26657+GO_OFF))" "127.0.0.1:$((26658+GO_OFF))" "$WORK/go-$quota.log"
  LONG=()
  [ "$quota" -eq 2 ] && LONG=(-long)
  python3 "$ROOT/conformance/grpc-lifecycle/resources.py" \
    --node-pid "$NODE_PID" --implementation go --listener core --quota "$quota" \
    --capture "$OUT/go-quota-$quota.json" \
    --resources "$OUT/go-quota-$quota-resources.json" -- \
    "$WORK/recorder" -implementation go -quota "$quota" "${LONG[@]}" \
    -endpoint "127.0.0.1:$((26658+GO_OFF))"
  if [ "$quota" -eq 0 ]; then
    python3 "$ROOT/conformance/grpc-lifecycle/resources.py" \
      --node-pid "$NODE_PID" --implementation go --listener sdk \
      --capture "$OUT/go-sdk.json" --resources "$OUT/go-sdk-resources.json" -- \
      "$WORK/recorder" -implementation go -listener sdk \
      -endpoint "127.0.0.1:$((9090+GO_OFF))"
  fi
  stop_node
done

mkdir -p "$RS_HOME/config" "$RS_HOME/data"
cp "$GO_HOME/config/genesis.json" "$RS_HOME/config/genesis.json"
cp "$GO_HOME/config/priv_validator_key.json" "$RS_HOME/config/priv_validator_key.json"
printf '%s\n' '{"height":"0","round":0,"step":0}' > "$RS_HOME/data/priv_validator_state.json"
cat > "$RS_HOME/config/app.toml" <<EOF
[grpc]
enable = true
address = "127.0.0.1:$((9090+RS_OFF))"
max-recv-msg-size = "64"
max-send-msg-size = "128"
[api]
enable = false
EOF
cp "$RS_HOME/config/app.toml" "$OUT/rs-app.toml"
for quota in 0 2; do
  cat > "$RS_HOME/config/config.toml" <<EOF
moniker = "grpc-lifecycle"
[rpc]
laddr = "tcp://127.0.0.1:$((26657+RS_OFF))"
grpc_laddr = "tcp://127.0.0.1:$((26658+RS_OFF))"
grpc_max_open_connections = $quota
[p2p]
laddr = "tcp://127.0.0.1:$((26656+RS_OFF))"
pex = false
addr_book_strict = false
[tx_index]
indexer = "kv"
EOF
  cp "$RS_HOME/config/config.toml" "$OUT/rs-quota-$quota-config.toml"
  python3 - "$RS_HOME/config/config.toml" "$RS_HOME/config/app.toml" <<'PY'
import ipaddress,pathlib,sys,tomllib
config=tomllib.loads(pathlib.Path(sys.argv[1]).read_text())
app=tomllib.loads(pathlib.Path(sys.argv[2]).read_text())
assert config['p2p']['pex'] is False
listeners=[config['rpc']['laddr'],config['rpc']['grpc_laddr'],config['p2p']['laddr']]
for section in ('api','grpc','grpc-web'):
 value=app.get(section,{})
 if value.get('enable') is True: listeners.append(value['address'])
for value in listeners:
 host,port=value.removeprefix('tcp://').rsplit(':',1)
 host=host.strip('[]')
 assert ipaddress.ip_address(host).is_loopback,(value,host)
 assert 0<int(port)<=65535,value
PY
  "$RS_BIN" --home "$RS_HOME" start > "$WORK/rs-$quota.log" 2>&1 &
  NODE_PID=$!
  wait_ready "127.0.0.1:$((26657+RS_OFF))" "127.0.0.1:$((26658+RS_OFF))" "$WORK/rs-$quota.log"
  python3 "$ROOT/conformance/grpc-lifecycle/resources.py" \
    --node-pid "$NODE_PID" --implementation rs --listener core --quota "$quota" \
    --capture "$OUT/rs-quota-$quota.json" \
    --resources "$OUT/rs-quota-$quota-resources.json" -- \
    "$WORK/recorder" -implementation rs -quota "$quota" \
    -endpoint "127.0.0.1:$((26658+RS_OFF))"
  if [ "$quota" -eq 0 ]; then
    python3 "$ROOT/conformance/grpc-lifecycle/resources.py" \
      --node-pid "$NODE_PID" --implementation rs --listener sdk \
      --capture "$OUT/rs-sdk.json" --resources "$OUT/rs-sdk-resources.json" -- \
      "$WORK/recorder" -implementation rs -listener sdk \
      -endpoint "127.0.0.1:$((9090+RS_OFF))"
  fi
  stop_node
done

python3 "$ROOT/conformance/grpc-lifecycle/freeze.py" \
  --repo "$ROOT" --capture "$OUT" --observation "$OBSERVATION" \
  --go-bin "$ORACLE_BIN" --go-core-bin "$CORE_BIN" \
  --go-app-build "$ORACLE_BIN.build.json" --go-core-build "$CORE_BIN.build.json" \
  --rs-bin "$RS_BIN" --rs-build-receipt "$RS_BUILD_RECEIPT" \
  --recorder-bin "$WORK/recorder"
python3 "$ROOT/conformance/grpc-lifecycle/verify.py" "$OUT"
