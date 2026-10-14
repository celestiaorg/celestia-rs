#!/usr/bin/env bash
# Blob (PayForBlobs) workload for devnet.sh / mixed.sh.
#
# Exercises square construction: PFBs from several accounts submitted in
# parallel so blocks carry several blob txs, blob sizes from 1 byte to a few
# hundred KiB (squares grow to 64x64), multiple blobs per tx with shared
# and distinct namespaces, share version 0 and share version 1 (signer),
# plus txs CheckTx rejects: low fee, low gas, a stale sequence, a
# non-canonical BlobTx and a re-broadcast of an included PFB.
#
# BlobTxs whose validation fails with a Go error (no blobs, invalid
# namespace, undecodable inner tx, commitment mismatch) are not sent: the
# standalone-ABCI app answers them with ResponseException, which makes
# celestia-core stop. They are covered by unit tests instead.
#
# Env (from devnet.sh): HOME_DIR, CHAIN_ID, NODE; celestia-appd on PATH.
set -euo pipefail

RPC=${NODE/tcp:/http:}
K=(--keyring-backend test --home "$HOME_DIR")
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
N=6

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
height() { curl -s "$RPC/status" | json 'd["result"]["sync_info"]["latest_block_height"]' 2>/dev/null || echo 0; }
wait_blocks() { # <n>: wait (at most 60 s) until n more blocks are committed
  local target=$(($(height) + $1)) i
  for i in $(seq 1 200); do
    [ "$(height)" -ge "$target" ] && return 0
    sleep 0.3
  done
  echo "node stopped producing blocks" >&2
  return 1
}

USER0=$(celestia-appd keys show user0 -a "${K[@]}")
ADDRS=()
for i in $(seq 1 $N); do
  celestia-appd keys add "blob$i" "${K[@]}" >/dev/null 2>&1 || true
  ADDRS+=("$(celestia-appd keys show "blob$i" -a "${K[@]}")")
done
celestia-appd tx bank multi-send "$USER0" "${ADDRS[@]}" 50000000000utia --from user0 "${K[@]}" \
  --chain-id "$CHAIN_ID" --node "$NODE" --fees 40000utia --gas 400000 -y -o json >/dev/null
wait_blocks 2

# blobs_file <out> <share-version> <ns:size>...: writes a --input-file JSON
# with deterministic pseudo-random data; prints the gas the PFB needs.
blobs_file() {
  python3 - "$@" <<'PY'
import hashlib, json, sys
out, sv, specs = sys.argv[1], int(sys.argv[2]), sys.argv[3:]
blobs, shares = [], 0
for i, spec in enumerate(specs):
    ns, size = spec.split(":")
    size = int(size)
    data = b""
    seed = f"{out}/{i}".encode()
    while len(data) < size:
        seed = hashlib.sha256(seed).digest()
        data += seed
    data = data[:size]
    blobs.append({"namespaceID": "0x" + ns.ljust(20, "0"), "blob": "0x" + data.hex()})
    first = 478 - (20 if sv == 1 else 0)
    shares += 1 if size <= first else 1 + -(-(size - first) // 482)
json.dump({"Blobs": blobs}, open(out, "w"))
print(int((120000 + 70 * 10 * len(specs) + shares * 512 * 8) * 1.2))
PY
}

# pfb <account> <share-version> <fee-per-gas x1000> <gas-delta> <ns:size>...
# Submits without waiting for inclusion; prints the CheckTx code.
pfb() {
  local from=$1 sv=$2 price=$3 delta=$4
  shift 4
  local f="$WORK/$from-$RANDOM$RANDOM.json" gas out
  gas=$(blobs_file "$f" "$sv" "$@")
  gas=$((gas + delta))
  out=$(celestia-appd tx blob pay-for-blob --input-file "$f" --share-version "$sv" \
    --from "$from" "${K[@]}" --chain-id "$CHAIN_ID" --node "$NODE" \
    --gas "$gas" --fees "$((gas * price / 1000 + 1))utia" -y -o json 2>&1) || true
  echo "pfb $from v$sv $* -> code $(echo "$out" | json 'd.get("code", 0)' 2>/dev/null || echo "err: ${out:0:120}")"
}

round() { # runs the given "pfb ..." lines in parallel, then waits a block
  local pids=()
  while read -r line; do
    [ -n "$line" ] || continue
    eval "$line" &
    pids+=($!)
  done
  wait "${pids[@]}"
  wait_blocks 2
}

# 1. tiny blobs, boundary sizes of the first sparse share (478 / 458 with signer)
round <<EOF
pfb blob1 0 10 0 01:1
pfb blob2 0 10 0 02:478
pfb blob3 0 10 0 03:479
pfb blob4 1 10 0 04:458
pfb blob5 1 10 0 05:459
pfb blob6 0 10 0 ffffffffffffffffffff:10
EOF

# 2. multiple blobs per tx: shared and distinct namespaces, both share versions
round <<EOF
pfb blob1 0 10 0 0a:100 0b:2000 0a:300
pfb blob2 1 10 0 0c:5000 01:1 0c:960
pfb blob3 0 10 0 0d:1 0d:1 0d:1 0d:1 0d:1
pfb blob4 1 12 0 0e:20000
pfb blob5 0 15 0 0f:8000 10:8000
EOF

# 3. medium blobs: the square grows past 16x16
round <<EOF
pfb blob1 0 10 0 20:60000
pfb blob2 1 10 0 21:45000 22:15000
pfb blob3 0 11 0 23:30000 24:700 25:1
pfb blob4 0 20 0 26:90000
EOF

# 4. large blobs: 64x64 squares
round <<EOF
pfb blob1 0 10 0 30:250000
pfb blob2 1 10 0 31:300000
pfb blob3 0 10 0 32:120000 33:120000
EOF

# 5. rejected by CheckTx: fee below the minimum gas price, gas below the
#    PFB requirement, and a stale sequence (two txs from one account racing).
round <<EOF
pfb blob1 0 1 0 40:100
pfb blob2 0 10 -400000 41:50000
pfb blob3 0 10 0 42:10
pfb blob3 0 10 0 43:10
EOF

# 6. crafted BlobTx bytes, broadcast raw: non-canonical (unknown field),
#    an included PFB re-broadcast (stale sequence) and the same PFB with a
#    duplicated type id (non-canonical).
python3 - "$RPC" <<'PY'
import base64, json, sys, urllib.request
rpc = sys.argv[1]
def varint(n):
    out = b""
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out += bytes([b | 0x80])
        else:
            return out + bytes([b])
def field(num, wt, payload):
    tag = varint(num << 3 | wt)
    return tag + (varint(len(payload)) + payload if wt == 2 else payload)
NS = bytes(18) + b"\x07" + bytes(9)
blob = field(1, 2, NS[1:]) + field(2, 2, b"x")
cases = {
    "non-canonical": field(1, 2, b"\x0a\x00") + field(2, 2, blob) + field(3, 2, b"BLOB") + field(15, 0, varint(1)),
}
get = lambda p: json.load(urllib.request.urlopen(rpc + p))["result"]
h = int(get("/status")["sync_info"]["latest_block_height"])
for i in range(h, 0, -1):
    txs = [base64.b64decode(t) for t in get(f"/block?height={i}")["block"]["data"].get("txs") or []]
    included = [t for t in txs if t.endswith(b"\x1a\x04BLOB")]
    if included:
        t = included[0]
        cases["replayed"] = t
        cases["duplicate-type-id"] = t + b"\x1a\x04BLOB"
        break
for name, tx in cases.items():
    try:
        r = json.load(urllib.request.urlopen(f"{rpc}/broadcast_tx_sync?tx=0x{tx.hex()}"))
        res = r.get("result") or r.get("error")
        print(f"crafted {name}: {json.dumps(res)[:200]}")
    except Exception as e:  # noqa: BLE001
        print(f"crafted {name}: {e}")
PY
wait_blocks 2

# 7. one more parallel round so the last blocks also carry blob txs
round <<EOF
pfb blob4 0 10 0 60:3000 61:3000 62:3000
pfb blob5 1 10 0 63:70000
pfb blob6 0 10 0 64:150000
EOF

# Summary: blob txs per block.
python3 - "$RPC" <<'PY'
import base64, json, urllib.request
rpc = __import__("sys").argv[1]
get = lambda p: json.load(urllib.request.urlopen(rpc + p))["result"]
h = int(get("/status")["sync_info"]["latest_block_height"])
total = 0
for i in range(1, h + 1):
    b = get(f"/block?height={i}")["block"]["data"]
    txs = b.get("txs") or []
    blobs = sum(1 for t in txs if base64.b64decode(t)[-6:] == b"\x1a\x04BLOB")
    if txs:
        print(f"h={i} txs={len(txs)} blob_txs={blobs} square={b.get('square_size')}")
    total += blobs
print("blob txs included:", total)
PY
