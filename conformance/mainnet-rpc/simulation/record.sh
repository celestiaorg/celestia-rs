#!/usr/bin/env bash
# Fresh Simulate capture against an isolated release-recipe Go standalone node.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
OUT=${1:?usage: APPD=path conformance/mainnet-rpc/simulation/record.sh EMPTY_OUT_DIR}
APPD=${APPD:?set APPD to the selected release-recipe celestia-appd binary}
APPD=$(realpath "$APPD")
BUILD=${APP_BUILD_MANIFEST:-$APPD.build.json}
OBSERVATION=${MAINNET_OBSERVATION:?set MAINNET_OBSERVATION to a fresh read-only mainnet observation JSON}
ABCI=${SIM_ABCI_PORT:-38658}
GRPC=${SIM_GRPC_PORT:-12090}
API=${SIM_REST_PORT:-4317}
mkdir -p "$OUT"
[ -z "$(ls -A "$OUT")" ] || { echo 'capture output directory must be empty' >&2; exit 1; }
OUT=$(cd "$OUT" && pwd)
WORK=$(mktemp -d)
NODE=$WORK/home
PID=
cleanup() {
  if [ -n "$PID" ]; then kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true; fi
  rm -rf "$WORK"
}
trap cleanup EXIT
python3 - "$APPD" "$BUILD" <<'PY'
import hashlib,json,pathlib,sys
binary,manifest=map(pathlib.Path,sys.argv[1:])
m=json.loads(manifest.read_text())
assert m['source_commit']=='6013e758d5dceb75827ad191c8d739d7c43cfa04'
assert m['module']=='github.com/celestiaorg/celestia-app/v10@v10.5.0'
assert m['build_tags']==['ledger']
assert hashlib.sha256(binary.read_bytes()).hexdigest()==m['binary_sha256']
PY
"$APPD" init simulation --chain-id rs-devnet-1 --home "$NODE" > "$OUT/init.log" 2>&1
cp "$ROOT/conformance/fixtures/devnet/empty/genesis.json" "$NODE/config/genesis.json"
python3 - "$NODE/config/app.toml" "$API" "$GRPC" <<'PY'
import pathlib,re,sys
p=pathlib.Path(sys.argv[1]);s=p.read_text().replace('pruning = "default"','pruning = "nothing"')
for name,port in [('api',sys.argv[2]),('grpc',sys.argv[3])]:
 start=s.index('['+name+']');end=s.find('\n[',start+1);end=len(s) if end<0 else end
 part=s[start:end];part=re.sub(r'^enable = .*$', 'enable = true',part,flags=re.M)
 prefix='tcp://' if name=='api' else ''
 part=re.sub(r'^address = .*$',f'address = "{prefix}127.0.0.1:{port}"',part,flags=re.M)
 s=s[:start]+part+s[end:]
p.write_text(s)
PY
"$APPD" start --force-no-bbr --home "$NODE" --with-comet=false --address "tcp://127.0.0.1:$ABCI" > "$OUT/app.log" 2>&1 &
PID=$!
python3 - "$ABCI" <<'PY'
import socket,sys,time
for attempt in range(100):
 try:
  with socket.create_connection(('127.0.0.1',int(sys.argv[1])),timeout=1): break
 except OSError:
  time.sleep(.1)
else: raise RuntimeError('isolated ABCI listener did not start')
PY
python3 "$ROOT/conformance/mainnet-rpc/simulation/capture-state.py" --address "127.0.0.1:$ABCI" \
  --replay "$ROOT/conformance/fixtures/devnet/empty/abci.rec" \
  --genesis "$ROOT/conformance/fixtures/devnet/empty/genesis.json" --out "$OUT/go-state.json"
python3 "$ROOT/conformance/mainnet-rpc/simulation/generate-requests.py" \
  --genesis "$ROOT/conformance/fixtures/devnet/empty/genesis.json" --out "$OUT/requests.json"
(cd "$ROOT/conformance/rpccorpus/recorder" && go build -buildvcs=false -o "$WORK/simulation-errors" ./cmd/simulation-errors)
"$WORK/simulation-errors" -address "127.0.0.1:$GRPC" -requests "$OUT/requests.json" -out "$OUT/go-errors.json"
cp "$BUILD" "$OUT/oracle-build.json"
cp "$OBSERVATION" "$OUT/mainnet-observation.json"
go version -m "$APPD" > "$OUT/go-build-info.txt"
(cd "$ROOT/conformance/rpccorpus/recorder" && go mod download -json \
 github.com/celestiaorg/celestia-core@v0.42.3 github.com/celestiaorg/cosmos-sdk@v0.52.12) > "$WORK/dependencies.raw"
python3 - "$WORK/dependencies.raw" "$OUT/dependencies.json" <<'PYDEP'
import json,pathlib,sys
raw=pathlib.Path(sys.argv[1]).read_text();decoder=json.JSONDecoder();deps=[]
while raw.strip():
 item,end=decoder.raw_decode(raw.lstrip());raw=raw.lstrip()[end:]
 deps.append({k:item[k] for k in ('Path','Version','Sum','GoModSum','Origin')})
pathlib.Path(sys.argv[2]).write_text(json.dumps(deps,indent=2)+'\n')
PYDEP
python3 - "$ROOT" "$OUT" <<'PY'
import datetime,hashlib,json,pathlib,subprocess,sys
root,out=map(pathlib.Path,sys.argv[1:])
inputs=['conformance/fixtures/devnet/empty/genesis.json','conformance/fixtures/devnet/empty/abci.rec',
 'conformance/mainnet-rpc/simulation/record.sh','conformance/mainnet-rpc/simulation/capture-state.py',
 'conformance/mainnet-rpc/simulation/generate-requests.py','conformance/rpccorpus/recorder/cmd/simulation-errors/main.go',
 'conformance/rpccorpus/recorder/go.mod','conformance/rpccorpus/recorder/go.sum']
meta={'captured_at':datetime.datetime.now(datetime.timezone.utc).isoformat(),
 'method':'fresh Go gRPC Simulate calls after replaying public ABCI request inputs into an isolated standalone app',
 'source_sha256':{p:hashlib.sha256((root/p).read_bytes()).hexdigest() for p in inputs},
 'artifacts_sha256':{p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in out.iterdir() if p.suffix in ('.json','.txt')},
 'exclusions':['Go diagnostic source annotations in error text','multiplexer and historical pre-v10 execution','nonempty protobuf status details','raw HTTP/2 header/trailer placement'],
 'regenerate':'MAINNET_OBSERVATION=<fresh observation> APPD=<release-recipe binary> conformance/mainnet-rpc/simulation/record.sh <empty output directory>'}
(out/'metadata.json').write_text(json.dumps(meta,indent=2)+'\n')
PY
