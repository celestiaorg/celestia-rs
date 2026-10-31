#!/usr/bin/env bash
# Generate fresh v10.5.0 populated staking evidence on an isolated devnet.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/populated-staking-bin}
OBSERVATION=${MAINNET_OBSERVATION:?set MAINNET_OBSERVATION to a fresh observation from observe-mainnet.py}
OUT=$ROOT/conformance/fixtures/devnet/v105-populated-staking
export DEVNET_BIN=$BIN
"$ROOT/conformance/devnet/build-bins.sh"
(cd "$ROOT/conformance/rpccorpus/recorder" && go build -buildvcs=false \
  -o "$BIN/abci-recorder" ./cmd/abci-frame-recorder && \
  go build -buildvcs=false -o "$BIN/rpc-recorder" .)
(cd "$ROOT/conformance/devnet-oracle" && go build -buildvcs=false \
  -o "$BIN/devnet-oracle" .)
python3 - "$BIN" "$OBSERVATION" <<'PY'
import hashlib,json,pathlib,sys
bin_dir,observation=map(pathlib.Path,sys.argv[1:])
for name,module,commit in (
 ('celestia-appd','github.com/celestiaorg/celestia-app/v10@v10.5.0','6013e758d5dceb75827ad191c8d739d7c43cfa04'),
 ('cometbft','github.com/celestiaorg/celestia-core@v0.42.3','a5537d84a008d566abc503d2ea41e30f2301403d')):
 binary=bin_dir/name; manifest=json.loads((bin_dir/(name+'.build.json')).read_text())
 assert manifest['module']==module and manifest['source_commit']==commit
 assert hashlib.sha256(binary.read_bytes()).hexdigest()==manifest['binary_sha256']
 if name=='celestia-appd': assert manifest['build_tags']==['ledger']
obs=json.loads(observation.read_text())
assert obs['chain_id']=='celestia' and obs['version']=='10.5.0'
PY
DEVNET_CAPTURE="$ROOT/conformance/mainnet-rpc/populated-staking/capture.sh" \
DEVNET_DETERMINISTIC=1 DEVNET_KEYS_DIR="$ROOT/conformance/mainnet-rpc/populated-staking/devnet-keys" \
SAVE_BLOCKS=1 DEVNET_PORT_OFFSET=${DEVNET_PORT_OFFSET:-6000} \
  "$ROOT/conformance/devnet/devnet.sh" v105-populated-staking 40 \
  "$ROOT/conformance/devnet/workloads/staking.sh"
cp "$OBSERVATION" "$OUT/capture/mainnet-observation.json"
cp "$BIN/celestia-appd.build.json" "$OUT/capture/app-build.json"
cp "$BIN/cometbft.build.json" "$OUT/capture/core-build.json"
cp "$BIN/celestia-appd.build-info.txt" "$OUT/capture/app-build-info.txt"
cp "$BIN/cometbft.build-info.txt" "$OUT/capture/core-build-info.txt"
python3 - "$ROOT" "$OUT" <<'PY'
import datetime,hashlib,json,pathlib,sys
root,out=map(pathlib.Path,sys.argv[1:]); cap=out/'capture'
sources=[
 'conformance/devnet/devnet.sh','conformance/devnet/build-bins.sh',
 'conformance/devnet/workloads/staking.sh','conformance/devnet/workloads/wait_tx.py',
 'conformance/mainnet-rpc/populated-staking/capture.sh',
 'conformance/mainnet-rpc/populated-staking/record.sh',
 'conformance/mainnet-rpc/populated-staking/devnet-keys/node_key.json',
 'conformance/mainnet-rpc/populated-staking/devnet-keys/priv_validator_key.json',
 'conformance/rpccorpus/recorder/cmd/populated-queries/main.go',
 'conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go',
 'conformance/rpccorpus/recorder/go.mod','conformance/rpccorpus/recorder/go.sum']
artifacts=['genesis.json','abci.rec','blocks.bin.gz','meta.json','capture/transports.json',
 'capture/mainnet-observation.json','capture/app-build.json','capture/core-build.json',
 'capture/app-build-info.txt','capture/core-build-info.txt','go-build-info.txt',
 'state/1.json','state/40.json']
digest=lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
transport=json.loads((cap/'transports.json').read_text())
metadata={
 'schema_version':1,'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
 'method':'fresh tagged v10.5.0 staking workload and live four-transport capture on one isolated devnet',
 'state_identity':{k:transport[k] for k in ('chain_id','tip_height','query_height','app_hash')},
 'source_sha256':{p:digest(root/p) for p in sources},
 'artifact_sha256':{p:digest(out/p) for p in artifacts},
 'omissions':transport['omissions']+[
   'pagination proof validity is deferred to the proof-focused capture',
   'staking lifecycle completion times beyond height 40 are not covered'],
 'regenerate':'MAINNET_OBSERVATION=<fresh observation.json> conformance/mainnet-rpc/populated-staking/record.sh'}
(cap/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
PY
echo "wrote $OUT"
