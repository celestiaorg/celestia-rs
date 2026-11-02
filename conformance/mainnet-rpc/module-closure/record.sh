#!/usr/bin/env bash
# Capture a single freshly populated current-release devnet, with exact receipts.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
WORKLOAD=${1:?workload name}
SCENARIO=v105-closure-$WORKLOAD
: "${MAINNET_OBSERVATION:?fresh observe-mainnet.py output}" "${DEVNET_BIN:?pinned build-bins.sh output}"
: "${POPULATED_REQUESTS_BIN:?built populated_requests example}"
SCRIPT=$ROOT/conformance/devnet/workloads/$WORKLOAD.sh
if [ "$WORKLOAD" = fibre ]; then
  SCRIPT=$ROOT/conformance/devnet/workloads/closure-fibre.sh
  export GENESIS_PATCH=$ROOT/conformance/devnet/workloads/fibre.genesis.py
fi
python3 - "$MAINNET_OBSERVATION" "$DEVNET_BIN" <<'PY'
import datetime,hashlib,json,pathlib,sys
obs=json.loads(pathlib.Path(sys.argv[1]).read_text());bin_dir=pathlib.Path(sys.argv[2])
assert obs['version']=='10.5.0' and obs['chain_id']=='celestia'
now=datetime.datetime.now(datetime.timezone.utc);observed=datetime.datetime.fromisoformat(obs['observed_at'])
assert observed.tzinfo is not None and observed<=now and now-observed<datetime.timedelta(hours=24)
for route in ('status','abci_info'):
 r=obs['responses'][route];assert hashlib.sha256(r['body'].encode()).hexdigest()==r['sha256']
for name,commit in [('celestia-appd','6013e758d5dceb75827ad191c8d739d7c43cfa04'),('cometbft','a5537d84a008d566abc503d2ea41e30f2301403d')]:
 p=bin_dir/name;m=json.loads((bin_dir/(name+'.build.json')).read_text())
 assert m['source_commit']==commit and m['binary_sha256']==hashlib.sha256(p.read_bytes()).hexdigest()
 if name=='celestia-appd':assert m['build_tags']==['ledger']
PY
export DEVNET_CAPTURE=$ROOT/conformance/mainnet-rpc/module-closure/capture.sh
export DEVNET_DETERMINISTIC=1 DEVNET_KEYS_DIR=$ROOT/conformance/mainnet-rpc/populated-governance/devnet-keys
export SAVE_BLOCKS=1 DEVNET_TX_INDEX=kv
"$ROOT/conformance/devnet/populated-closure.sh" "$SCENARIO" 20 "$SCRIPT"
OUT=$ROOT/conformance/fixtures/devnet/$SCENARIO
cp "$MAINNET_OBSERVATION" "$OUT/capture/mainnet-observation.json"
for tool in celestia-appd cometbft; do
  cp "$DEVNET_BIN/$tool.build.json" "$OUT/capture/$tool-build.json"
  cp "$DEVNET_BIN/$tool.build-info.txt" "$OUT/capture/$tool-build-info.txt"
done
python3 "$ROOT/conformance/mainnet-rpc/module-closure/receipt.py" "$ROOT" "$OUT" "$WORKLOAD"
