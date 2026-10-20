#!/usr/bin/env bash
# Mempool interop: the Rust CAT mempool + reactor (crate `mempool`) peering
# with a Go celestia-core v0.42.3 validator (celestia-appd v10.4.0).
#
# Topology (one machine):
#   - Go validator V (celestia-appd): produces every block.
#   - Rust node R (`cargo run -p mempool --example mempool_interop`): the
#     Rust state machine in process, the Rust CAT mempool calling it in
#     process (CheckTx New/Recheck, QuerySequence), a Rust P2P switch with
#     the CAT reactor dialing V, and a small RPC (broadcast_tx_sync, ...;
#     anything else is proxied to V so the CLI can sign through R).
#   - Go follower F (celestia-core over ABCI): only drives block execution
#     of R's app (R has no consensus yet); its own mempool is refused.
#
# Phase A (Go -> Rust): the blob workload (PFBs, workloads/blob.sh) plus a
#   burst of sequential bank sends is submitted to V. Every tx V includes in
#   a block must have been gossiped to R (SeenTx -> WantTx -> Txs) and
#   accepted by the Rust CheckTx before.
# Phase B (Rust -> Go): bank sends (a burst with consecutive sequences,
#   signed offline, JSON-RPC broadcast_tx_sync) and PFBs (celestia-appd CLI
#   with --node R) are submitted to R only. Each must be accepted by the
#   Rust mempool, reach V's mempool by gossip and be included in a
#   V-produced block.
# Finally both mempools must drain, V must still list R as a peer and V's
# log must not show it dropping R.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/mempool-interop.sh
# Exit 0 iff all checks pass.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-3000}
CHAIN_ID=rs-mempool-1
W=$(mktemp -d)
V=$W/val; F=$W/fol; R=$W/rust
RUST="$ROOT/target/debug/examples/mempool_interop"
KR=(--keyring-backend test --home "$V")

(cd "$ROOT" && cargo build -q -p mempool --example mempool_interop)

A="celestia-appd --home $V"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
for k in val0 user0 rust0 rust1; do
  $A keys add $k --keyring-backend test >/dev/null 2>&1
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1

shift_ports() { # file delta
  python3 - "$1" "$2" <<'PY'
import re, sys
p, d = sys.argv[1], int(sys.argv[2])
s = open(p).read()
s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + d), s)
open(p, "w").write(s)
PY
}
shift_ports "$V/config/config.toml" "$OFF"
shift_ports "$V/config/app.toml" "$OFF"
sed -i.bak -e 's#^addr_book_strict = true#addr_book_strict = false#' \
       -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' "$V/config/config.toml"
rm -f "$V/config/config.toml.bak"
GO_P2P=$((26656 + OFF)); GO_RPC=$((26657 + OFF))
R_P2P=$((26656 + OFF + 200)); R_RPC=$((26657 + OFF + 200)); R_ABCI=$((26658 + OFF + 200))

# Follower: celestia-core home with the validator's genesis (CometBFT form).
mkdir -p "$F/config" "$F/data"
cometbft init --home "$F" >/dev/null 2>&1 || true
python3 - "$V" "$F" <<'PY'
import json, sys
v, f = sys.argv[1:]
g = json.load(open(v + "/config/genesis.json"))
c = {"genesis_time": g["genesis_time"], "chain_id": g["chain_id"],
     "initial_height": str(g["initial_height"]),
     "consensus_params": g["consensus"]["params"], "validators": [],
     "app_hash": "", "app_state": g["app_state"]}
json.dump(c, open(f + "/config/genesis.json", "w"))
PY
cp "$V/config/config.toml" "$F/config/config.toml"
shift_ports "$F/config/config.toml" 100
GO_ID=$(celestia-appd comet show-node-id --home "$V" 2>/dev/null || cometbft show_node_id --home "$V")
R_ID=$("$RUST" --home "$R" --print-id)
sed -i.bak "s#^persistent_peers = .*#persistent_peers = \"$GO_ID@127.0.0.1:$GO_P2P\"#" "$F/config/config.toml"
rm -f "$F/config/config.toml.bak"
echo "go=$GO_ID rust=$R_ID"

PIDS=(); cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; wait "${PIDS[@]}" 2>/dev/null || true; }; trap cleanup EXIT
celestia-appd start --force-no-bbr --home "$V" > "$W/val.log" 2>&1 & PIDS+=($!)
"$RUST" --home "$R" --chain-id $CHAIN_ID --abci "127.0.0.1:$R_ABCI" --listen "127.0.0.1:$R_P2P" \
  --dial "$GO_ID@127.0.0.1:$GO_P2P" --rpc "127.0.0.1:$R_RPC" --go-rpc "127.0.0.1:$GO_RPC" \
  > "$W/rust.log" 2>&1 & RPID=$!; PIDS+=($RPID)
sleep 1
cometbft start --home "$F" --proxy_app "tcp://127.0.0.1:$R_ABCI" > "$W/fol.log" 2>&1 & PIDS+=($!)

gh() { curl -s "localhost:$GO_RPC/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
rs() { curl -s "localhost:$R_RPC/rust_status"; }
rh() { rs | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["height"])' 2>/dev/null || echo 0; }
for _ in $(seq 1 180); do [ "$(rh)" -ge 2 ] 2>/dev/null && break; sleep 1; done
echo "go at height $(gh), rust app at height $(rh): $(rs)"

FAIL=()
START=$(gh)

# ---------------------------------------------------------------- phase A
echo "== phase A: txs submitted to the Go validator"
HOME_DIR=$V CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$GO_RPC bash "$ROOT/conformance/devnet/workloads/blob.sh" > "$W/blob.log" 2>&1 || true
grep -E "^(pfb|crafted|blob txs included)" "$W/blob.log" | tail -4 || true
# a burst of consecutive sequences from one account (the Rust reactor must
# queue the future SeenTx announcements and fill the gap in order)
USER0=$($A keys show user0 -a --keyring-backend test)
VAL0=$($A keys show val0 -a --keyring-backend test)
acct() { # addr -> "number sequence" (from the Go node)
  celestia-appd query auth account "$1" --node "tcp://127.0.0.1:$GO_RPC" -o json | python3 -c '
import json, sys
d = json.load(sys.stdin); a = d.get("account", d)
a = a.get("value", a); a = a.get("base_account", a)
print(a.get("account_number", "0"), a.get("sequence", "0"))'
}

# burst <key> <rpc port> <n> <keys file> <label>: signs n bank sends with
# consecutive sequences offline, then broadcasts them back to back
# (JSON-RPC broadcast_tx_sync), so they reach the peer out of block order.
burst() {
  local key=$1 port=$2 n=$3 keys=$4 label=$5 addr num seq i b64 res
  addr=$($A keys show "$key" -a --keyring-backend test)
  read -r num seq < <(acct "$addr")
  local txs=()
  for i in $(seq 0 $((n - 1))); do
    celestia-appd tx bank send "$addr" "$VAL0" "$((1000 + i))utia" --from "$key" "${KR[@]}" --chain-id $CHAIN_ID \
      --fees 20000utia --gas 100000 --generate-only -o json > "$W/$label-$i.json" 2>/dev/null
    celestia-appd tx sign "$W/$label-$i.json" --from "$key" "${KR[@]}" --chain-id $CHAIN_ID --offline \
      --account-number "$num" --sequence "$((seq + i))" -o json > "$W/$label-$i.signed.json" 2>/dev/null
    txs+=("$(celestia-appd tx encode "$W/$label-$i.signed.json" 2>/dev/null)")
  done
  for i in $(seq 0 $((n - 1))); do
    b64=${txs[$i]}
    res=$(curl -s -X POST -H 'Content-Type: application/json' "localhost:$port/" \
      -d "{\"jsonrpc\":\"2.0\",\"id\":$i,\"method\":\"broadcast_tx_sync\",\"params\":{\"tx\":\"$b64\"}}")
    echo "$label: broadcast_tx_sync seq $((seq + i)) -> ${res:0:160}"
    python3 -c 'import base64,hashlib,sys; print(hashlib.sha256(base64.b64decode(sys.argv[1])).hexdigest().upper())' "$b64" >> "$keys"
    echo "$res" | grep -q '"code":0' || FAIL+=("$label: seq $((seq + i)) rejected: ${res:0:200}")
  done
}
: > "$W/burstA.keys"
burst user0 "$GO_RPC" 5 "$W/burstA.keys" go-burst
sleep 8
END_A=$(gh)

python3 - "$GO_RPC" "$R_RPC" "$START" "$END_A" "$W/burstA.keys" > "$W/phaseA.txt" <<'PY' || FAIL+=("phase A: Go-included txs missing from the Rust mempool")
import base64, hashlib, json, sys, urllib.request
go, rust, start, end = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
burst = {l.strip().upper() for l in open(sys.argv[5])} if len(sys.argv) > 5 else set()
get = lambda port, p: json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}{p}"))["result"]
def varint(b, i):
    n = s = 0
    while True:
        c = b[i]; i += 1; n |= (c & 0x7F) << s; s += 7
        if c < 0x80: return n, i
def key(tx):
    if tx.endswith(b"\x1a\x04BLOB") and tx[:1] == b"\x0a":
        n, i = varint(tx, 1)
        return hashlib.sha256(tx[i:i + n]).hexdigest().upper(), True
    return hashlib.sha256(tx).hexdigest().upper(), False
log = get(rust, "/tx_log")
checks = log["checks"]
missing, blobs, total, included = [], 0, 0, set()
for h in range(start + 1, end + 1):
    for t in get(go, f"/block?height={h}")["block"]["data"].get("txs") or []:
        k, is_blob = key(base64.b64decode(t))
        total += 1; blobs += is_blob; included.add(k)
        c = checks.get(k)
        if not c or c["code"] != 0:
            missing.append((h, k, c))
print(f"blocks {start + 1}..{end}: {total} txs ({blobs} blob txs) included by Go; "
      f"rust CheckTx ran on {len(checks)} txs, accepted {sum(1 for c in checks.values() if c['code'] == 0)}")
for m in missing:
    print("MISSING in rust mempool:", m)
print(f"bank send burst: {len(burst & included)}/{len(burst)} included")
if missing or total == 0 or blobs == 0 or not burst or not burst <= included:
    sys.exit(1)
PY
cat "$W/phaseA.txt"

# ---------------------------------------------------------------- phase B
echo "== phase B: txs submitted to the Rust node only"
START_B=$(gh)
: > "$W/phaseB.keys"
burst rust0 "$R_RPC" 4 "$W/phaseB.keys" rust-burst
# PFBs through the CLI with --node pointing at the Rust node.
WORK=$W/blobs; mkdir -p "$WORK"
RUST1=$($A keys show rust1 -a --keyring-backend test)
read -r _ SEQ0 < <(acct "$RUST1")
for spec in "0b:2000" "0c:60000"; do
  ns=${spec%%:*}; size=${spec##*:}
  python3 - "$WORK/$ns.json" "$ns" "$size" <<'PY'
import hashlib, json, sys
out, ns, size = sys.argv[1], sys.argv[2], int(sys.argv[3])
data, seed = b"", out.encode()
while len(data) < size:
    seed = hashlib.sha256(seed).digest(); data += seed
json.dump({"Blobs": [{"namespaceID": "0x" + ns.ljust(20, "0"), "blob": "0x" + data[:size].hex()}]}, open(out, "w"))
PY
  GAS=$((200000 + size * 10))
  OUT=$(celestia-appd tx blob pay-for-blob --input-file "$WORK/$ns.json" --from rust1 "${KR[@]}" --chain-id $CHAIN_ID \
    --node "tcp://127.0.0.1:$R_RPC" --gas "$GAS" --fees "$((GAS / 100 + 1))utia" -y -o json 2>&1) || true
  echo "rust pfb $spec: ${OUT:0:300}"
  CODE=$(echo "$OUT" | python3 -c 'import json,sys; print(json.load(sys.stdin).get("code", -1))' 2>/dev/null || echo err)
  [ "$CODE" = 0 ] || FAIL+=("phase B: rust rejected pfb $spec: ${OUT:0:200}")
  echo "$OUT" | python3 -c 'import json,sys; print(json.load(sys.stdin)["txhash"])' >> "$W/phaseB.keys" 2>/dev/null || true
  # The CLI reads the signer's sequence from committed state (proxied to V):
  # wait for this PFB to land before signing the next one.
  for _ in $(seq 1 60); do
    read -r _ RSEQ < <(acct "$RUST1")
    [ "$RSEQ" -gt "$SEQ0" ] && break
    sleep 1
  done
  SEQ0=$RSEQ
done

python3 - "$GO_RPC" "$R_RPC" "$START_B" "$W/phaseB.keys" > "$W/phaseB.txt" <<'PY' || FAIL+=("phase B: Rust-submitted txs not included by Go")
import base64, hashlib, json, sys, time, urllib.request
go, rust, start, keysf = sys.argv[1], sys.argv[2], int(sys.argv[3]), sys.argv[4]
get = lambda port, p: json.load(urllib.request.urlopen(f"http://127.0.0.1:{port}{p}"))["result"]
want = {l.strip().upper() for l in open(keysf) if l.strip()}
def varint(b, i):
    n = s = 0
    while True:
        c = b[i]; i += 1; n |= (c & 0x7F) << s; s += 7
        if c < 0x80: return n, i
def key(tx):
    if tx.endswith(b"\x1a\x04BLOB") and tx[:1] == b"\x0a":
        n, i = varint(tx, 1)
        return hashlib.sha256(tx[i:i + n]).hexdigest().upper()
    return hashlib.sha256(tx).hexdigest().upper()
found, h, deadline = {}, start + 1, time.time() + 90
while want - found.keys() and time.time() < deadline:
    if h > int(get(go, "/status")["sync_info"]["latest_block_height"]):
        time.sleep(0.5); continue
    for t in get(go, f"/block?height={h}")["block"]["data"].get("txs") or []:
        k = key(base64.b64decode(t))
        if k in want: found[k] = h
    h += 1
log = get(rust, "/tx_log")
rpc = set(log["rpc_keys"])
for k in sorted(want):
    c = log["checks"].get(k, {})
    print(f"{k[:16]}: rust rpc={k in rpc} rust code={c.get('code')} included at go height {found.get(k)}")
if len(want) < 6 or want - found.keys() or not want <= rpc:
    sys.exit(1)
PY
cat "$W/phaseB.txt"

# ---------------------------------------------------------------- final
for _ in $(seq 1 60); do
  GN=$(curl -s "localhost:$GO_RPC/num_unconfirmed_txs" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["n_txs"])')
  RN_=$(curl -s "localhost:$R_RPC/num_unconfirmed_txs" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["n_txs"])')
  [ "$GN" = 0 ] && [ "$RN_" = 0 ] && break
  sleep 1
done
echo "mempools: go=$GN rust=$RN_"
[ "$GN" = 0 ] && [ "$RN_" = 0 ] || FAIL+=("mempools did not drain (go=$GN rust=$RN_)")
STATUS=$(rs)
echo "rust status: $STATUS"
echo "$STATUS" | grep -q '"go_peer_connected":true' || FAIL+=("Go peer not connected to the Rust node")
curl -s "localhost:$GO_RPC/net_info" | grep -q "$R_ID" || FAIL+=("Go /net_info does not list the Rust node")
kill -0 $RPID 2>/dev/null || FAIL+=("Rust node exited")
sed 's/\x1b\[[0-9;]*m//g' "$W/val.log" > "$W/val.clean.log"
BAD=$(grep -E "Stopping peer for error|rejected|filtered" "$W/val.clean.log" | grep "$R_ID" | grep -v "err=EOF" || true)
[ -z "$BAD" ] || FAIL+=("Go dropped the Rust peer: $(echo "$BAD" | head -2)")
if grep -q "wrong Block.Header.AppHash\|CONSENSUS FAILURE" "$W/fol.log"; then FAIL+=("follower app hash mismatch"); fi

if [ ${#FAIL[@]} -eq 0 ]; then
  echo "mempool interop OK (go height $(gh), rust app height $(rh))"
  trap - EXIT; cleanup; rm -rf "$W"
  exit 0
fi
printf 'FAILED: %s\n' "${FAIL[@]}"
echo "logs: $W"
tail -n 5 "$W/rust.log"
trap - EXIT; cleanup
exit 1
