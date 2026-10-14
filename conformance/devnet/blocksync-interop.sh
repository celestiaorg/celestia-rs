#!/usr/bin/env bash
# Block sync / state sync interop (ADR 0003, crates `blocksync`, `evidence`,
# `statesync`, wired into `celestia-rs start`):
#
# Topology (one machine): a Go celestia-appd v10.4.0 validator V produces
# blocks (with the blob workload) and takes snapshots every $INTERVAL blocks.
#
#  A. Go -> Rust block sync: a fresh `celestia-rs start` full node R (its
#     only peer is V) block-syncs $BLOCKS+ blocks over channel 0x40 —
#     verifying each block with the next block's commit, ProcessProposal and
#     executing it in the in-process Rust app (any app-hash mismatch fails
#     `validateBlock` of the next block) — then keeps following V. The
#     script compares R's app hash after every height with V's next header.
#  B. Rust -> Go block sync: a fresh Go celestia-appd node G whose ONLY
#     peer is R block-syncs from R's block store (R serves blocks, extended
#     commits and status) and must reach R's height with V's block hashes.
#  C. State sync -> block sync -> follow: a fresh Rust node S with state
#     sync enabled (light client against V's RPC) restores V's snapshot
#     over 0x60/0x61, bootstraps its stores, block-syncs from the snapshot
#     height and follows; app hashes are checked like in A.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] [BLOCKS=200] conformance/devnet/blocksync-interop.sh
# Exit 0 iff all phases pass.
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-16000}
BLOCKS=${BLOCKS:-200}
INTERVAL=${INTERVAL:-50}
CHAIN_ID=rs-blocksync-1
W=$(mktemp -d)
V=$W/val; R=$W/rust; G=$W/go; S=$W/rust-ss
RS="$ROOT/target/debug/celestia-rs"

(cd "$ROOT" && cargo build -q -p celestia-rs)

A="celestia-appd --home $V"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
for k in val0 user0; do
  $A keys add $k --keyring-backend test >/dev/null 2>&1
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1

port() { echo $(( $1 + OFF + $2 )); }   # base, node offset (0 V, 100 R, 200 G, 300 S)
shift_ports() { # file delta
  python3 - "$1" "$2" <<'PY'
import re, sys
p, d = sys.argv[1], int(sys.argv[2])
s = open(p).read()
s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + d), s)
open(p, "w").write(s)
PY
}
set_toml() { # file section key value
  python3 - "$@" <<'PY'
import re, sys
p, sec, key, val = sys.argv[1:]
s = open(p).read()
i = s.index("[" + sec + "]")
j = s.find("\n[", i + 1); j = len(s) if j < 0 else j
body = re.sub(r'(?m)^' + re.escape(key) + r' = .*$', key + ' = ' + val, s[i:j], count=1)
open(p, "w").write(s[:i] + body + s[j:])
PY
}
shift_ports "$V/config/config.toml" "$OFF"
shift_ports "$V/config/app.toml" "$OFF"
sed -i -e 's#^addr_book_strict = true#addr_book_strict = false#' -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' "$V/config/config.toml"
set_toml "$V/config/app.toml" state-sync snapshot-interval "$INTERVAL"
VAL_ID=$(celestia-appd comet show-node-id --home "$V" 2>/dev/null)
VAL_P2P="$VAL_ID@127.0.0.1:$(port 26656 0)"
VAL_RPC="127.0.0.1:$(port 26657 0)"

PIDS=(); cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; wait "${PIDS[@]}" 2>/dev/null || true; }; trap cleanup EXIT
fail() { echo "FAILED: $*"; echo "logs: $W"; tail -n 20 "$W"/*.log 2>/dev/null | tail -n 60; trap - EXIT; cleanup; exit 1; }
plain() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }
gh() { curl -s "localhost:$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
wait_go() { # rpc-port height log [secs]
  for _ in $(seq 1 "${4:-600}"); do
    local x; x=$(gh "$1" || true)
    [ "${x:-0}" -ge "$2" ] 2>/dev/null && return 0
    if grep -q "CONSENSUS FAILURE\|panic:" "$3" 2>/dev/null; then return 1; fi
    sleep 1
  done
  return 1
}
# rust node height from its "synced block" / "committed block" log lines
rh() { plain "$1" | grep -E 'synced block|committed block' | grep -o ' height=[0-9]*' | tail -1 | cut -d= -f2; }
wait_rust() { # log height pid [secs]
  for _ in $(seq 1 "${4:-900}"); do
    local x; x=$(rh "$1" || true)
    [ "${x:-0}" -ge "$2" ] 2>/dev/null && return 0
    kill -0 "$3" 2>/dev/null || return 1
    sleep 1
  done
  return 1
}
# compare the app hash a rust node logged after each height with V's next header
check_app_hashes() { # log from to
  plain "$1" | grep 'synced block' > "$W/synced.txt" || true
  python3 - "$VAL_RPC" "$W/synced.txt" "$2" "$3" <<'PY'
import json, re, sys, urllib.request
rpc, f, lo, hi = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4])
got = {}
for line in open(f):
    h = re.search(r' height=(\d+)', line); a = re.search(r'app_hash=([0-9A-F]+)', line); b = re.search(r' hash=([0-9A-F]+)', line)
    if h and a: got[int(h.group(1))] = (a.group(1), b.group(1) if b else "")
get = lambda p: json.load(urllib.request.urlopen(f"http://{rpc}{p}"))["result"]
bad, checked = [], 0
for h in range(lo, hi + 1):
    if h not in got:
        bad.append((h, "missing")); continue
    nxt = get(f"/block?height={h + 1}")["block"]["header"]["app_hash"]
    bid = get(f"/block?height={h}")["block_id"]["hash"]
    if got[h][0] != nxt or got[h][1] != bid:
        bad.append((h, got[h], nxt, bid))
    checked += 1
print(f"checked {checked} heights {lo}..{hi}: {len(bad)} mismatches")
for b in bad[:5]: print("  ", b)
sys.exit(1 if bad else 0)
PY
}
blob_blocks() { # from to -> number of blocks with blob txs on V
  python3 - "$VAL_RPC" "$1" "$2" <<'PY'
import base64, json, sys, urllib.request
rpc, lo, hi = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
n = 0
for h in range(lo, hi + 1):
    txs = json.load(urllib.request.urlopen(f"http://{rpc}/block?height={h}"))["result"]["block"]["data"].get("txs") or []
    n += any(base64.b64decode(t).endswith(b"\x1a\x04BLOB") for t in txs)
print(n)
PY
}
rust_home() { # dir moniker
  celestia-appd init "$2" --chain-id $CHAIN_ID --home "$1" >/dev/null 2>&1
  cp "$V/config/genesis.json" "$1/config/genesis.json"
}

celestia-appd start --home "$V" > "$W/val.log" 2>&1 & PIDS+=($!)
wait_go "$(port 26657 0)" 2 "$W/val.log" 120 || fail "validator did not start"
T0=$(date +%s)
HOME_DIR=$V CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$(port 26657 0) bash "$ROOT/conformance/devnet/workloads/blob.sh" > "$W/workload.log" 2>&1 || true
echo "blob workload done at height $(gh "$(port 26657 0)")"
wait_go "$(port 26657 0)" $((BLOCKS + 5)) "$W/val.log" 1800 || fail "validator did not reach $((BLOCKS + 5))"
echo "validator at $(gh "$(port 26657 0)") after $(( $(date +%s) - T0 ))s"

# --- Phase A: Go -> Rust block sync, then follow ---------------------------
rust_home "$R" rust0
: > "$W/rust.log"
R_P2P=$(port 26656 100)
"$RS" start --home "$R" --p2p-laddr "127.0.0.1:$R_P2P" --persistent-peers "$VAL_P2P" \
  --allow-duplicate-ip --moniker rust0 > "$W/rust.log" 2>&1 & RPID=$!; PIDS+=($RPID)
T1=$(date +%s)
wait_rust "$W/rust.log" "$BLOCKS" $RPID 900 || fail "rust node did not block-sync to $BLOCKS (at $(rh "$W/rust.log"))"
SYNC_S=$(( $(date +%s) - T1 ))
VH=$(gh "$(port 26657 0)")
echo "phase A: rust node block-synced to $(rh "$W/rust.log") in ${SYNC_S}s (validator at $VH)"
# follow: the rust node must keep up with new blocks
wait_rust "$W/rust.log" $((VH + 5)) $RPID 120 || fail "rust node does not follow (at $(rh "$W/rust.log"), validator $(gh "$(port 26657 0)"))"
RH=$(rh "$W/rust.log")
check_app_hashes "$W/rust.log" 1 "$RH" || fail "app hash mismatch in phase A"
NB=$(blob_blocks 1 "$RH")
[ "$NB" -gt 0 ] || fail "no blob blocks among the synced blocks"
plain "$W/rust.log" | grep -q "Error in validation" && fail "rust node rejected a block from Go"
echo "phase A OK: rust synced and followed to $RH ($NB blocks with blob txs), app hashes match"

# --- Phase B: Rust -> Go block sync ----------------------------------------
R_ID=$(celestia-appd comet show-node-id --home "$R" 2>/dev/null)
celestia-appd init go0 --chain-id $CHAIN_ID --home "$G" >/dev/null 2>&1
cp "$V/config/genesis.json" "$G/config/genesis.json"
cp "$V/config/config.toml" "$G/config/config.toml"
cp "$V/config/app.toml" "$G/config/app.toml"
shift_ports "$G/config/config.toml" 200
shift_ports "$G/config/app.toml" 200
set_toml "$G/config/app.toml" state-sync snapshot-interval 0
sed -i "s#^persistent_peers = .*#persistent_peers = \"$R_ID@127.0.0.1:$R_P2P\"#" "$G/config/config.toml"
set_toml "$G/config/config.toml" p2p pex false
celestia-appd start --home "$G" > "$W/go.log" 2>&1 & GPID=$!; PIDS+=($GPID)
TARGET=$(( $(rh "$W/rust.log") - 2 ))
wait_go "$(port 26657 200)" "$TARGET" "$W/go.log" 600 || fail "go node did not block-sync from the rust node to $TARGET (at $(gh "$(port 26657 200)"))"
GH=$(gh "$(port 26657 200)")
python3 - "$VAL_RPC" "127.0.0.1:$(port 26657 200)" "$GH" <<'PY' || fail "go node's blocks differ from the validator's"
import json, sys, urllib.request
v, g, hi = sys.argv[1], sys.argv[2], int(sys.argv[3])
get = lambda rpc, p: json.load(urllib.request.urlopen(f"http://{rpc}{p}"))["result"]
bad = [h for h in range(1, hi + 1) if get(v, f"/block?height={h}")["block_id"]["hash"] != get(g, f"/block?height={h}")["block_id"]["hash"]]
peers = [p["node_info"]["moniker"] for p in get(g, "/net_info")["peers"]]
print(f"go node: {hi} blocks match the validator; peers {peers}")
sys.exit(1 if bad or peers != ["rust0"] else 0)
PY
plain "$W/go.log" | grep -i "stopping peer for error" | grep -q "$R_ID" && fail "go node dropped the rust peer: $(plain "$W/go.log" | grep -i 'stopping peer for error' | head -2)"
echo "phase B OK: go node block-synced $GH blocks from the rust node only"
kill $GPID 2>/dev/null || true

# --- Phase C: state sync -> block sync -> follow ---------------------------
rust_home "$S" rust1
: > "$W/rust-ss.log"
TH=$(( $(gh "$(port 26657 0)") - 2 ))
THASH=$(curl -s "$VAL_RPC/block?height=$TH" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block_id"]["hash"])')
"$RS" start --home "$S" --p2p-laddr "127.0.0.1:$(port 26656 300)" --persistent-peers "$VAL_P2P" \
  --allow-duplicate-ip --moniker rust1 --statesync-rpc-servers "$VAL_RPC,$VAL_RPC" \
  --statesync-trust-height "$TH" --statesync-trust-hash "$THASH" --statesync-discovery-secs 5 \
  > "$W/rust-ss.log" 2>&1 & SPID=$!; PIDS+=($SPID)
for _ in $(seq 1 300); do
  plain "$W/rust-ss.log" | grep -q "state sync complete" && break
  kill -0 $SPID 2>/dev/null || fail "state-sync node exited"
  sleep 1
done
plain "$W/rust-ss.log" | grep -q "state sync complete" || fail "rust node did not state sync"
SNAP=$(plain "$W/rust-ss.log" | grep "state sync complete" | grep -o ' height=[0-9]*' | tail -1 | cut -d= -f2)
VH=$(gh "$(port 26657 0)")
wait_rust "$W/rust-ss.log" $((VH + 5)) $SPID 300 || fail "state-synced node did not block-sync and follow (at $(rh "$W/rust-ss.log"))"
SH=$(rh "$W/rust-ss.log")
check_app_hashes "$W/rust-ss.log" $((SNAP + 1)) "$SH" || fail "app hash mismatch in phase C"
echo "phase C OK: rust node state-synced at $SNAP, block-synced and followed to $SH"

kill -0 $RPID 2>/dev/null || fail "rust node R exited"
echo "OK: blocksync interop passed (logs: $W)"
trap - EXIT; cleanup
[ -n "${KEEP:-}" ] || rm -rf "$W"
exit 0
