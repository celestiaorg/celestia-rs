#!/usr/bin/env bash
# Fetches a public network's genesis and its first blocks over CometBFT
# JSON-RPC for `cargo run -p devnet-conformance --example genesis_replay`:
#
#   <out>/genesis.json            the genesis doc (from /genesis_chunked)
#   <out>/block/<h>.json          /block            h = 1..N+1
#   <out>/results/<h>.json        /block_results    h = 1..N
#   <out>/validators/<h>.json     /validators       h = 1..N (all pages)
#
# usage: conformance/devnet/fetch-chain.sh <rpc-url> <out-dir> <N> [first]
set -euo pipefail
RPC=$1 OUT=$2 N=$3 FIRST=${4:-1}
mkdir -p "$OUT/block" "$OUT/results" "$OUT/validators"
get() { # <path> <file>: retries rate-limited / failed requests
  local i
  for i in 1 2 3 4 5 6; do
    if curl -sS -m 60 "$RPC$1" -o "$2.tmp" && grep -q '"result"' "$2.tmp"; then
      mv "$2.tmp" "$2"
      return 0
    fi
    sleep $((i * 2))
  done
  echo "failed: $1" >&2
  return 1
}
if [ ! -s "$OUT/genesis.json" ]; then
  total=$(curl -sS -m 60 "$RPC/genesis_chunked?chunk=0" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["total"])')
  # Each chunk is base64 of a slice of the genesis bytes.
  : >"$OUT/genesis.part"
  for c in $(seq 0 $((total - 1))); do
    get "/genesis_chunked?chunk=$c" "$OUT/chunk.json"
    python3 -c 'import base64,json,sys; sys.stdout.buffer.write(base64.b64decode(json.load(open(sys.argv[1]))["result"]["data"]))' \
      "$OUT/chunk.json" >>"$OUT/genesis.part"
  done
  mv "$OUT/genesis.part" "$OUT/genesis.json"
  rm -f "$OUT/chunk.json"
fi
for h in $(seq "$FIRST" $((N + 1))); do
  [ -s "$OUT/block/$h.json" ] || get "/block?height=$h" "$OUT/block/$h.json"
  [ "$h" -le "$N" ] || continue
  [ -s "$OUT/results/$h.json" ] || get "/block_results?height=$h" "$OUT/results/$h.json"
  if [ ! -s "$OUT/validators/$h.json" ]; then
    get "/validators?height=$h&per_page=100&page=1" "$OUT/validators/$h.json"
    pages=$(python3 -c 'import json,sys; t=int(json.load(open(sys.argv[1]))["result"]["total"]); print((t+99)//100)' "$OUT/validators/$h.json")
    for p in $(seq 2 "$pages"); do
      get "/validators?height=$h&per_page=100&page=$p" "$OUT/validators/$h.p$p.json"
    done
  fi
done
echo "fetched $OUT (heights $FIRST..$N)"
