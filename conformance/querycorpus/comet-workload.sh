#!/usr/bin/env bash
# Workload of the `comet-queries` corpus (record-comet.sh): a few bank
# sends, a multi-send and pay-for-blob txs (blob txs in the block data),
# so the celestia-core-backed queries (blocks, tx index, tx status) have
# transactions to find.
#
# Env (from record-comet.sh): HOME_DIR, CHAIN_ID, NODE; celestia-appd on PATH.
set -euo pipefail
trap 'echo "comet-workload.sh: failed at line $LINENO" >&2' ERR
source "$(dirname "$0")/../devnet/workloads/lib.sh"

U=$(addr user0)
V=$(addr val0)
celestia-appd keys add extra0 "${K[@]}" >/dev/null 2>&1 || true
E=$(addr extra0)
tx user0 bank send "$U" "$E" 1000000utia
tx user0 bank send "$U" "$V" 2500utia --note memo-1
tx extra0 bank send "$E" "$U" 10utia

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
python3 - "$WORK/blobs.json" <<'PY'
import json, sys
json.dump({"Blobs": [
    {"namespaceID": "0x" + "0a".ljust(20, "0"), "blob": "0x" + "ab" * 700},
    {"namespaceID": "0x" + "0b".ljust(20, "0"), "blob": "0x" + "cd" * 40},
]}, open(sys.argv[1], "w"))
PY
out=$(celestia-appd tx blob pay-for-blob --input-file "$WORK/blobs.json" --share-version 0 \
  --from user0 "${K[@]}" --chain-id "$CHAIN_ID" --node "$NODE" --gas 300000 --fees 6000utia \
  -y -o json 2>&1) || { echo "pay-for-blob failed: $out" >&2; exit 1; }
hash=$(echo "$out" | tail -1 | json 'd["txhash"]') || { echo "pay-for-blob: $out" >&2; exit 1; }
echo "pay-for-blob $hash"
# The block carries the BlobTx, whose hash is not the tx hash: wait for
# blocks instead of scanning for it.
wait_blocks 3
tx user0 bank send "$U" "$E" 7utia
wait_blocks 2
