#!/usr/bin/env bash
# Capture hook run by devnet.sh while every isolated local listener is live.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
for name in CAPTURE_DIR HOME_DIR CHAIN_ID COMET_RPC SDK_GRPC REST_API; do
  [ -n "${!name:-}" ] || { echo "missing $name" >&2; exit 1; }
done
# devnet.sh offsets every listener by the same amount. Derive celestia-core's
# loopback-only gRPC port from the supplied Comet RPC port without changing the
# shared capture-hook interface (26657 and 9098 are the respective base ports).
COMET_PORT=${COMET_RPC##*:}
COMET_PORT=${COMET_PORT%%/*}
CORE_GRPC=127.0.0.1:$((COMET_PORT - 26657 + 9098))
INPUTS=$CAPTURE_DIR/inputs
mkdir -p "$INPUTS"
APP=(celestia-appd --home "$HOME_DIR")
KEYS=(--keyring-backend test)
USER=$(${APP[@]} keys show user0 -a "${KEYS[@]}")
RECIPIENT=$(${APP[@]} keys show val0 -a "${KEYS[@]}")

curl --fail --silent --show-error \
  "$REST_API/cosmos/auth/v1beta1/accounts/$USER" > "$INPUTS/account-rest.json"
"${APP[@]}" query auth account "$USER" --node "$COMET_RPC" \
  --output json > "$INPUTS/account.json"
read -r ACCOUNT_NUMBER SEQUENCE < <(python3 - "$INPUTS/account.json" <<'PY'
import json,sys
value=json.load(open(sys.argv[1]))
def find(value):
 if isinstance(value,dict):
  if 'account_number' in value:
   return value['account_number'],value.get('sequence','0')
  for child in value.values():
   found=find(child)
   if found:return found
 if isinstance(value,list):
  for child in value:
   found=find(child)
   if found:return found
 return None
found=find(value)
if not found: raise SystemExit('base account number and sequence not found')
print(*found)
PY
)

encode_signed() { # name sequence; unsigned command follows
  local name=$1 sequence=$2; shift 2
  if ! "${APP[@]}" "$@" --generate-only --chain-id "$CHAIN_ID" "${KEYS[@]}" \
    --output json > "$INPUTS/$name.unsigned.json"; then
    # v10.5's PFB command emits a complete unsigned JSON document and then
    # attempts an empty post even with --generate-only. Retain only a complete
    # nonempty document; every other command failure remains fatal.
    [ "$name" = blob ] && [ -s "$INPUTS/$name.unsigned.json" ] || return 1
  fi
  "${APP[@]}" tx sign "$INPUTS/$name.unsigned.json" --from user0 --offline \
    --account-number "$ACCOUNT_NUMBER" --sequence "$sequence" --chain-id "$CHAIN_ID" \
    "${KEYS[@]}" --output json > "$INPUTS/$name.signed.json"
  "${APP[@]}" tx encode "$INPUTS/$name.signed.json" | base64 --decode > "$INPUTS/$name.tx"
  rm "$INPUTS/$name.unsigned.json" "$INPUTS/$name.signed.json"
}

bank() { # name sequence amount [gas]
  local name=$1 sequence=$2 amount=$3 gas=${4:-200000}
  encode_signed "$name" "$sequence" tx bank send user0 "$RECIPIENT" "$amount" \
    --gas "$gas" --fees 2000utia --note "live-tx-$name"
}

python3 - "$INPUTS/no-message.tx" <<'PY'
import pathlib,sys
def field(number,value):
 return bytes([number<<3|2,len(value)])+value
body=field(2,b'no messages')
pathlib.Path(sys.argv[1]).write_bytes(field(1,body)+field(2,b''))
PY
bank duplicate "$SEQUENCE" 1utia
cp "$INPUTS/duplicate.tx" "$INPUTS/bad-signature.tx"
python3 - "$INPUTS/bad-signature.tx" <<'PY'
import pathlib,sys
p=pathlib.Path(sys.argv[1]);b=bytearray(p.read_bytes());b[-1]^=1;p.write_bytes(b)
PY
bank low-gas "$SEQUENCE" 1utia 1
bank future-sequence $((SEQUENCE+50)) 1utia
bank rest-async $((SEQUENCE+1)) 2utia
bank cosmos-sync $((SEQUENCE+2)) 3utia
bank cosmos-async $((SEQUENCE+3)) 4utia
bank core-grpc $((SEQUENCE+4)) 5utia
bank comet-async $((SEQUENCE+5)) 6utia
bank comet-commit $((SEQUENCE+6)) 7utia
bank cancel $((SEQUENCE+7)) 8utia
bank cleanup $((SEQUENCE+8)) 9utia
bank execution-failure $((SEQUENCE+9)) 2000000000000utia 300000
python3 - "$INPUTS" "$ACCOUNT_NUMBER" "$SEQUENCE" "$USER" "$RECIPIENT" <<'PY'
import hashlib,json,pathlib,sys
root=pathlib.Path(sys.argv[1])
data={'account_number':sys.argv[2],'initial_sequence':sys.argv[3],
      'sender':sys.argv[4],'recipient':sys.argv[5],
      'transactions':{p.name:hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted(root.glob('*.tx'))}}
(root/'manifest.json').write_text(json.dumps(data,indent=2)+'\n')
PY

live-tx -rpc "$COMET_RPC" -rest "$REST_API" -sdk-grpc "$SDK_GRPC" \
  -core-grpc "$CORE_GRPC" -inputs "$INPUTS" -chain-id "$CHAIN_ID" \
  -out "$CAPTURE_DIR/transactions.json"

# The v10.5 PFB command attempts an empty RPC post even with --generate-only,
# which terminates a standalone ABCI socket. Submit the representative blob
# normally after the raw transport matrix, then retain the exact CLI response,
# Comet tx result and raw BlobTx bytes from the indexed inclusion.
curl --fail --silent --show-error "$COMET_RPC/status" > "$CAPTURE_DIR/blob-before-status.json"
curl --fail --silent --show-error "$COMET_RPC/num_unconfirmed_txs" > "$CAPTURE_DIR/blob-before-unconfirmed.json"
"${APP[@]}" tx blob pay-for-blob \
  0x00010203040506070809 0x63656c65737469612d72732d6c6976652d7478 \
  --from user0 --node "$COMET_RPC" --chain-id "$CHAIN_ID" "${KEYS[@]}" \
  --gas 500000 --fees 2000utia --note live-tx-blob --yes --output json \
  > "$CAPTURE_DIR/blob-cli-response.json"
BLOB_HASH=$(python3 -c 'import json,sys; print(json.load(sys.stdin)["txhash"])' \
  < "$CAPTURE_DIR/blob-cli-response.json")
for _ in $(seq 1 100); do
  curl --silent --show-error "$COMET_RPC/tx?hash=0x$BLOB_HASH&prove=false" \
    > "$CAPTURE_DIR/blob-inclusion.json"
  python3 - "$CAPTURE_DIR/blob-inclusion.json" <<'PY' && break
import json,sys
body=json.load(open(sys.argv[1]));raise SystemExit(0 if body.get('result') else 1)
PY
  sleep 0.2
done
python3 - "$CAPTURE_DIR/blob-inclusion.json" "$INPUTS/blob.tx" <<'PY'
import base64,json,pathlib,sys
body=json.load(open(sys.argv[1]));result=body.get('result')
if not result:raise SystemExit('blob transaction was not included')
pathlib.Path(sys.argv[2]).write_bytes(base64.b64decode(result['tx'],validate=True))
PY
curl --fail --silent --show-error "$COMET_RPC/status" > "$CAPTURE_DIR/blob-after-status.json"
curl --fail --silent --show-error "$COMET_RPC/num_unconfirmed_txs" > "$CAPTURE_DIR/blob-after-unconfirmed.json"
curl --fail --silent --show-error "$COMET_RPC/abci_info" > "$CAPTURE_DIR/final-abci-info.json"
python3 - "$CAPTURE_DIR" "$INPUTS/manifest.json" <<'PY'
import hashlib,json,pathlib,sys
cap,manifest=map(pathlib.Path,sys.argv[1:]);tx=cap/'inputs/blob.tx'
data={'transport':'celestia-appd CLI using Comet broadcast_tx_sync','mode':'sync',
 'tx_file':'inputs/blob.tx','tx_sha256':hashlib.sha256(tx.read_bytes()).hexdigest(),
 'cli_response':json.loads((cap/'blob-cli-response.json').read_text()),
 'inclusion':json.loads((cap/'blob-inclusion.json').read_text()),
 'before':{'status':json.loads((cap/'blob-before-status.json').read_text()),
           'unconfirmed':json.loads((cap/'blob-before-unconfirmed.json').read_text())},
 'after':{'status':json.loads((cap/'blob-after-status.json').read_text()),
          'unconfirmed':json.loads((cap/'blob-after-unconfirmed.json').read_text())}}
(cap/'blob-submission.json').write_text(json.dumps(data,indent=2)+'\n')
m=json.loads(manifest.read_text());m['transactions']['blob.tx']=data['tx_sha256']
manifest.write_text(json.dumps(m,indent=2)+'\n')
PY
