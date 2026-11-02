#!/usr/bin/env bash
# Persistent Go/Rust Comet RPC comparison at fixed, matched chain heights.
# Usage: RPC_PERSISTENCE_OUT=/tmp/rpc-persistence bash conformance/devnet/rpc-persistence.sh
# This is an isolated devnet campaign, not a claim of complete #189 coverage.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
RS=${RS_BIN:-$ROOT/target/debug/celestia-rs}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-12000}
W=$(mktemp -d)
FAIL=()
OUT=${RPC_PERSISTENCE_OUT:?set RPC_PERSISTENCE_OUT to a new capture directory}
mkdir "$OUT"
OUT=$(cd "$OUT" && pwd)
python3 - "$BIN" "$RS" "${MAINNET_OBSERVATION:?set MAINNET_OBSERVATION to a fresh observation}" "$OUT" "$ROOT" <<'PY'
from datetime import datetime,timezone,timedelta
import hashlib,json,pathlib,shutil,subprocess,sys
bin_dir,rs,observation,out,root=map(pathlib.Path,sys.argv[1:])
obs=json.loads(observation.read_text())
observed=datetime.fromisoformat(obs['observed_at']);now=datetime.now(timezone.utc)
assert observed.tzinfo is not None and timedelta(0)<=now-observed<=timedelta(hours=24)
assert obs['chain_id']=='celestia' and obs['version']=='10.5.0'
parsed={}
for route in ('status','abci_info'):
    raw=obs['responses'][route]['body'].encode()
    assert hashlib.sha256(raw).hexdigest()==obs['responses'][route]['sha256']
    parsed[route]=json.loads(raw)['result']
status=parsed['status'];info=parsed['abci_info']['response']
assert status['node_info']['network']=='celestia' and not status['sync_info']['catching_up']
assert info['version']=='10.5.0' and status['node_info']['protocol_version']['app']==info['app_version']=='10'
assert status['sync_info']['latest_block_height']==info['last_block_height']
shutil.copyfile(observation,out/'mainnet-observation.json')
for name,module,commit in (
    ('celestia-appd','github.com/celestiaorg/celestia-app/v10@v10.5.0','6013e758d5dceb75827ad191c8d739d7c43cfa04'),
    ('cometbft','github.com/celestiaorg/celestia-core@v0.42.3','a5537d84a008d566abc503d2ea41e30f2301403d')):
    manifest=json.loads((bin_dir/(name+'.build.json')).read_text())
    assert manifest['module']==module and manifest['source_commit']==commit
    assert hashlib.sha256((bin_dir/name).read_bytes()).hexdigest()==manifest['binary_sha256']
    if name=='celestia-appd': assert manifest['build_tags']==['ledger']
    shutil.copyfile(bin_dir/(name+'.build.json'),out/(name+'.build.json'))
sources=('conformance/devnet/rpc-persistence.sh','conformance/devnet/lib-mixed.sh','conformance/mainnet-rpc/persistence/capture.py')
receipt={'rs_binary_sha256':hashlib.sha256(rs.read_bytes()).hexdigest(),
         'rs_version':subprocess.check_output([str(rs),'version','--long'],text=True),
         'driver_commit':subprocess.check_output(['git','-C',str(root),'rev-parse','HEAD'],text=True).strip(),
         'sources':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in sources}}
(out/'build-receipts.json').write_text(json.dumps(receipt,indent=2)+'\n')
PY
source "$ROOT/conformance/devnet/lib-mixed.sh"
trap 'cleanup; echo "node logs: $W"' EXIT
# RPC is durable state; progress must not depend on the caller's RUST_LOG filter.
height() {
  curl -fsS --max-time 5 "$(rpc "$1" "$2")/status" 2>/dev/null | \
    python3 -c 'import json,sys;print(json.load(sys.stdin)["result"]["sync_info"]["latest_block_height"])' 2>/dev/null || echo 0
}
wait_height() {
  local deadline=$((SECONDS+$4))
  while [ "$SECONDS" -lt "$deadline" ]; do
    if ! kill -0 "$(cat "$1/n$2/pid")" 2>/dev/null; then
      echo "node $2 exited before height $3" >&2
      tail -n 20 "$1/n$2/node.log" >&2
      return 1
    fi
    [ "$(height "$1" "$2")" -ge "$3" ] && return 0
    sleep 1
  done
  echo "node $2 did not reach height $3" >&2
  return 1
}

for profile in ${RPC_PERSISTENCE_PROFILES:-retain-kv prune-kv retain-null prune-null}; do
  case "$profile" in retain-kv|prune-kv|retain-null|prune-null) ;; *) echo "unsupported profile: $profile" >&2; exit 1;; esac
  CHAIN_ID=rs-rpc-persistence-$profile
  D=$W/$profile
  mkdir "$D" "$OUT/$profile"
  # Keep an independent Go full node serving RPC while the sole validator is
  # stopped, so latest/future-height errors see a common, quiescent tip.
  make_net "$D" "$CHAIN_ID" "go rs go" "5000000000 0 0"
  for i in 0 1 2; do
    python3 - "$D/n$i/config/config.toml" "$D/n$i/config/app.toml" "$profile" <<'PY'
import pathlib,re,sys
config,app=map(pathlib.Path,sys.argv[1:3]);profile=sys.argv[3]
s=config.read_text()
s=re.sub(r'^indexer = .*$', 'indexer = "'+('null' if profile.endswith('null') else 'kv')+'"',s,flags=re.M)
s=re.sub(r'^discard_abci_responses = .*$', 'discard_abci_responses = false',s,flags=re.M)
config.write_text(s)
s=app.read_text()
settings={'pruning':'custom' if profile.startswith('prune') else 'nothing',
          'pruning-keep-recent':'10','pruning-interval':'10','min-retain-blocks':'0'}
for key,value in settings.items():
    s=re.sub(r'^'+re.escape(key)+r' = .*$',key+' = '+('0' if key=='min-retain-blocks' else '"'+value+'"'),s,flags=re.M)
app.write_text(s)
PY
    cp "$D/n$i/config/config.toml" "$OUT/$profile/config-$i.toml"
    cp "$D/n$i/config/app.toml" "$OUT/$profile/app-$i.toml"
  done
  cp "$D/n0/config/genesis.json" "$OUT/$profile/genesis.json"
  for i in 0 1 2; do start_node "$D" "$i"; done
  ensure_up "$D" 0 1 2
  for i in 0 1 2; do wait_height "$D" "$i" 3 180; done
  SENDER=$(celestia-appd keys show user0 -a --keyring-backend test --home "$D/n0")
  RECIPIENT=$(celestia-appd keys show user1 -a --keyring-backend test --home "$D/n0")
  for amount in 101 102 103; do
    celestia-appd tx bank send user0 "$RECIPIENT" "${amount}utia" \
      --home "$D/n0" --keyring-backend test --chain-id "$CHAIN_ID" \
      --node "tcp://127.0.0.1:$((26657+OFF))" --fees 20000utia --gas 200000 -y -o json > "$OUT/$profile/tx-$amount.json"
    HASH=$(python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));assert d["code"]==0;print(d["txhash"])' "$OUT/$profile/tx-$amount.json")
    # Search retained blocks directly: /tx is intentionally unavailable with
    # null indexing, and subscribing after broadcast can miss a fast inclusion.
    python3 - "$(rpc "$D" 0)" "$HASH" > "$OUT/$profile/inclusion-$amount.json" <<'PY'
import base64,hashlib,json,sys,time,urllib.request
base,wanted=sys.argv[1:];deadline=time.monotonic()+90;next_height=1
def rpc(path):
    return json.load(urllib.request.urlopen(base+path,timeout=5))['result']
while time.monotonic()<deadline:
    top=int(rpc('/status')['sync_info']['latest_block_height'])
    for height in range(next_height,top+1):
        block=rpc('/block?height='+str(height))
        for index,tx in enumerate(block['block']['data']['txs'] or []):
            if hashlib.sha256(base64.b64decode(tx)).hexdigest().upper()==wanted:
                result=rpc('/block_results?height='+str(height))['txs_results'][index]
                assert result['code']==0,result
                print(json.dumps({'hash':wanted,'height':height,'index':index,'result':result}))
                raise SystemExit(0)
    next_height=top+1
    time.sleep(0.2)
raise SystemExit('transaction not included within 90 seconds')
PY
  done
  for i in 0 1 2; do wait_height "$D" "$i" 16 180; done
  H=$(height "$D" 0)
  H=$((H-2))
  for i in 0 1 2; do wait_height "$D" "$i" "$((H+1))" 90; done
  capture() {
    # A restarted follower must resume progress while a validator is still
    # producing commits; do not freeze the chain at its old pre-restart tip.
    wait_height "$D" 1 "$(height "$D" 2)" 90
    kill_node "$D" 0
    sleep 2
    local tip
    tip=$(height "$D" 2)
    wait_height "$D" 1 "$tip" 90
    python3 "$ROOT/conformance/mainnet-rpc/persistence/capture.py" \
      --go "$(rpc "$D" 2)" --rs "$(rpc "$D" 1)" --height "$H" \
      --sender "$SENDER" --tx "$HASH" --chain-id "$CHAIN_ID" \
      --out "$OUT/$profile/$1.json" || FAIL+=("$profile/$1")
  }
  capture before-restart
  RESTART_TARGET=$(($(height "$D" 2)+3))
  for i in 1 2; do kill_node "$D" "$i"; done
  # Exercise an explicit ordered restart separately from the committed shared
  # simultaneous-restart stall: let the validator advance before followers
  # select between block sync and consensus catch-up.
  start_node "$D" 0
  wait_height "$D" 0 "$RESTART_TARGET" 180
  for i in 1 2; do start_node "$D" "$i"; done
  ensure_up "$D" 0 1 2
  for i in 0 1 2; do wait_height "$D" "$i" "$RESTART_TARGET" 180; done
  capture after-restart
  start_node "$D" 0
  ensure_up "$D" 0
  for i in 0 1 2; do wait_height "$D" "$i" "$((H+25))" 300; done
  capture after-pruning-boundary
  for i in 1 2; do kill_node "$D" "$i"; done
  OFF=$((OFF+100))
done
if [ "${#FAIL[@]}" -ne 0 ]; then printf 'FAILED: %s\n' "${FAIL[@]}"; exit 1; fi
echo "All captured fixed-height Comet cases match; captures: $OUT"
