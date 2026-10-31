#!/usr/bin/env bash
# Generate fresh transaction-submission evidence on an isolated Go devnet.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/live-tx-bin}
OBSERVATION=${MAINNET_OBSERVATION:?set MAINNET_OBSERVATION to a fresh observation from observe-mainnet.py}
OUT=$ROOT/conformance/fixtures/devnet/v105-live-tx
export DEVNET_BIN=$BIN
"$ROOT/conformance/devnet/build-bins.sh"
(cd "$ROOT/conformance/rpccorpus/recorder" && go build -buildvcs=false \
  -o "$BIN/abci-recorder" ./cmd/abci-frame-recorder && \
  go build -buildvcs=false -o "$BIN/live-tx" ./cmd/live-tx && \
  go build -buildvcs=false -o "$BIN/rpc-recorder" .)
(cd "$ROOT/conformance/devnet-oracle" && go build -buildvcs=false \
  -o "$BIN/devnet-oracle" .)
python3 - "$BIN" "$OBSERVATION" <<'PY'
import datetime,hashlib,json,pathlib,sys
bin_dir,observation=map(pathlib.Path,sys.argv[1:])
for name,module,commit in (
 ('celestia-appd','github.com/celestiaorg/celestia-app/v10@v10.5.0','6013e758d5dceb75827ad191c8d739d7c43cfa04'),
 ('cometbft','github.com/celestiaorg/celestia-core@v0.42.3','a5537d84a008d566abc503d2ea41e30f2301403d')):
 manifest=json.loads((bin_dir/(name+'.build.json')).read_text())
 assert manifest['module']==module and manifest['source_commit']==commit
 assert hashlib.sha256((bin_dir/name).read_bytes()).hexdigest()==manifest['binary_sha256']
obs=json.loads(observation.read_text());now=datetime.datetime.now(datetime.timezone.utc)
observed=datetime.datetime.fromisoformat(obs['observed_at'])
assert obs['chain_id']=='celestia' and obs['version']=='10.5.0'
assert observed.tzinfo is not None and observed <= now
assert now-observed <= datetime.timedelta(hours=24)
for route in ('status','abci_info'):
 raw=obs['responses'][route]['body'].encode()
 assert hashlib.sha256(raw).hexdigest()==obs['responses'][route]['sha256']
status=json.loads(obs['responses']['status']['body'])['result']
info=json.loads(obs['responses']['abci_info']['body'])['result']['response']
assert status['node_info']['network']==obs['chain_id']
assert info['version']==obs['version']
assert status['node_info']['protocol_version']['app']==info['app_version']
assert status['sync_info']['latest_block_height']==info['last_block_height']
PY
DEVNET_CAPTURE="$ROOT/conformance/mainnet-rpc/live-tx/capture.sh" \
DEVNET_DETERMINISTIC=1 DEVNET_KEYS_DIR="$ROOT/conformance/mainnet-rpc/live-tx/devnet-keys" \
DEVNET_TX_INDEX=kv SAVE_BLOCKS=1 DEVNET_PORT_OFFSET=${DEVNET_PORT_OFFSET:-9000} \
  "$ROOT/conformance/devnet/devnet.sh" v105-live-tx 4
cp "$OBSERVATION" "$OUT/capture/mainnet-observation.json"
cp "$BIN/celestia-appd.build.json" "$OUT/capture/app-build.json"
cp "$BIN/cometbft.build.json" "$OUT/capture/core-build.json"
cp "$BIN/celestia-appd.build-info.txt" "$OUT/capture/app-build-info.txt"
cp "$BIN/cometbft.build-info.txt" "$OUT/capture/core-build-info.txt"
for tool in abci-recorder live-tx rpc-recorder devnet-oracle; do
  go version -m "$BIN/$tool" > "$OUT/capture/$tool-build-info.txt"
done
python3 - "$ROOT" "$OUT" "$BIN" <<'PY'
import datetime,hashlib,json,pathlib,sys
root,out,bin_dir=map(pathlib.Path,sys.argv[1:]);cap=out/'capture'
last=json.loads((out/'meta.json').read_text())['committed_heights']
sources=[
 'conformance/devnet/devnet.sh','conformance/devnet/build-bins.sh',
 'conformance/mainnet-rpc/observe-mainnet.py','conformance/mainnet-rpc/live-tx/capture.sh',
 'conformance/mainnet-rpc/live-tx/record.sh',
 'conformance/mainnet-rpc/live-tx/devnet-keys/node_key.json',
 'conformance/mainnet-rpc/live-tx/devnet-keys/priv_validator_key.json',
 'conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go',
 'conformance/rpccorpus/recorder/cmd/live-tx/main.go',
 'conformance/rpccorpus/recorder/main.go','conformance/rpccorpus/recorder/record.go',
 'conformance/rpccorpus/recorder/validation.go',
 'conformance/rpccorpus/recorder/go.mod','conformance/rpccorpus/recorder/go.sum',
 'conformance/devnet-oracle/main.go','conformance/devnet-oracle/go.mod',
 'conformance/devnet-oracle/go.sum']
artifacts=['genesis.json','abci.rec','blocks.bin.gz','meta.json','go-build-info.txt',
 'capture/transactions.json','capture/inputs/account.json','capture/inputs/account-rest.json',
 'capture/inputs/manifest.json','capture/blob-submission.json','capture/blob-cli-response.json',
 'capture/blob-inclusion.json','capture/blob-before-status.json','capture/blob-before-unconfirmed.json',
 'capture/blob-after-status.json','capture/blob-after-unconfirmed.json',
 'capture/final-abci-info.json',
 'capture/mainnet-observation.json','capture/app-build.json','capture/core-build.json',
 'capture/app-build-info.txt','capture/core-build-info.txt',
 'capture/abci-recorder-build-info.txt','capture/live-tx-build-info.txt',
 'capture/rpc-recorder-build-info.txt','capture/devnet-oracle-build-info.txt',
 'state/1.json',f'state/{last}.json']
artifacts += [str(path.relative_to(out)) for path in sorted((cap/'inputs').glob('*.tx'))]
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
groups={
 'abci-recorder':('github.com/celestiaorg/celestia-rs/conformance/rpccorpus/recorder',
   ['conformance/rpccorpus/recorder/cmd/abci-frame-recorder/main.go','conformance/rpccorpus/recorder/go.mod','conformance/rpccorpus/recorder/go.sum']),
 'live-tx':('github.com/celestiaorg/celestia-rs/conformance/rpccorpus/recorder',
   ['conformance/rpccorpus/recorder/cmd/live-tx/main.go','conformance/rpccorpus/recorder/go.mod','conformance/rpccorpus/recorder/go.sum']),
 'rpc-recorder':('github.com/celestiaorg/celestia-rs/conformance/rpccorpus/recorder',
   ['conformance/rpccorpus/recorder/main.go','conformance/rpccorpus/recorder/record.go','conformance/rpccorpus/recorder/validation.go','conformance/rpccorpus/recorder/go.mod','conformance/rpccorpus/recorder/go.sum']),
 'devnet-oracle':('github.com/celestiaorg/celestia-rs/conformance/devnet-oracle',
   ['conformance/devnet-oracle/main.go','conformance/devnet-oracle/go.mod','conformance/devnet-oracle/go.sum'])}
generators={name:{'sha256':digest(bin_dir/name),'module':module,'sources':files,
 'build_info':f'capture/{name}-build-info.txt','build_info_sha256':digest(cap/f'{name}-build-info.txt')}
 for name,(module,files) in groups.items()}
tx=json.loads((cap/'transactions.json').read_text())
covered=tx['covered']+['representative pay-for-blob transaction through Comet sync via the v10.5 CLI']
metadata={'schema_version':1,'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
 'method':'fresh tagged v10.5.0 live transaction submission on one isolated Go devnet',
 'state_identity':{'chain_id':tx['chain_id'],'initial_height':tx['initial_height'],
                   'final_height':tx['final_height'],'committed_heights':last},
 'source_sha256':{p:digest(root/p) for p in sources},
 'artifact_sha256':{p:digest(out/p) for p in artifacts},
 'generator_binaries':generators,'covered':covered,'omissions':tx['omissions'],
 'regenerate':'MAINNET_OBSERVATION=<fresh observation.json> conformance/mainnet-rpc/live-tx/record.sh'}
(cap/'metadata.json').write_text(json.dumps(metadata,indent=2)+'\n')
PY
echo "wrote $OUT"
