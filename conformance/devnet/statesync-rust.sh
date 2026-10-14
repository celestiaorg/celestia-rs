#!/usr/bin/env bash
# Rust state-sync node interop (ADR 0003, `light` + `statesync` crates):
#
#  1. Go -> Rust: a Go celestia-appd validator takes snapshots. A Rust node
#     (`statesync_node` example: p2p Switch + statesync reactor on 0x60/0x61
#     + light client against the validator's RPC + the in-process Rust app)
#     dials it, discovers its snapshots, verifies headers H, H+1, H+2 and the
#     consensus params with the light client, fetches the chunks from the Go
#     peer, restores them into the Rust app, verifies the restored app hash
#     against the light-client-verified header and bootstraps the State +
#     Commit. The script independently compares the restored app hash with
#     the validator's block H+1 header.
#  2. Rust -> Go: the Rust node snapshots the restored state and serves it
#     over its statesync reactor; a fresh Go celestia-appd node whose ONLY
#     peer is the Rust node state-syncs from it (Go verifies the app hash
#     against its own light client).
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/statesync-rust.sh [workload.sh]
# Exit 0 iff both phases succeed.
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
WORKLOAD=${1:-}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-7000}
CHAIN_ID=rs-statesync-rust-1
INTERVAL=10
SERVE=${SERVE_SECS:-150}
W=$(mktemp -d)
V=$W/val; R=$W/rust; G=$W/go
RUST="$ROOT/target/debug/examples/statesync_node"

(cd "$ROOT" && cargo build -q -p statesync --example statesync_node)

A="celestia-appd --home $V"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A keys add user0 --keyring-backend test >/dev/null 2>&1
for k in val0 user0; do
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1

port() { echo $(( $1 + OFF + $2 )); }   # base, node index (0 validator, 100 rust, 200 go)
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

PIDS=(); cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }; trap cleanup EXIT
fail() { echo "FAILED: $*"; echo "logs: $W"; trap - EXIT; cleanup; exit 1; }
h() { curl -s "localhost:$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
wait_height() { # rpc-port height log
  for _ in $(seq 1 300); do
    local x; x=$(h "$1" || true)
    [ "${x:-0}" -ge "$2" ] 2>/dev/null && return 0
    if grep -q "CONSENSUS FAILURE\|panic:" "$3" 2>/dev/null; then return 1; fi
    sleep 1
  done
  return 1
}
trust() { # -> "height hash" of a recent validator block
  local th=$(( $(h "$(port 26657 0)") - 2 ))
  echo "$th $(curl -s "$VAL_RPC/block?height=$th" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block_id"]["hash"])')"
}
header_app_hash() { # height -> app_hash of that header
  curl -s "$VAL_RPC/block?height=$1" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block"]["header"]["app_hash"])'
}

celestia-appd start --home "$V" > "$W/val.log" 2>&1 & PIDS+=($!)
wait_height "$(port 26657 0)" 2 "$W/val.log" || fail "validator did not start"
if [ -n "$WORKLOAD" ]; then
  HOME_DIR=$V CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$(port 26657 0) bash "$WORKLOAD" > "$W/workload.log" 2>&1 || true
  echo "workload $(basename "$WORKLOAD") done at height $(h "$(port 26657 0)")"
fi
NEXT=$(( ($(h "$(port 26657 0)") / INTERVAL + 1) * INTERVAL + 3 ))
[ "$NEXT" -ge $((2 * INTERVAL + 3)) ] || NEXT=$((2 * INTERVAL + 3))
wait_height "$(port 26657 0)" "$NEXT" "$W/val.log" || fail "validator stalled"
echo "validator at $(h "$(port 26657 0)"), snapshots at multiples of $INTERVAL"

# --- Phase 1: Go snapshot -> Rust state-sync node --------------------------
read -r TH THASH < <(trust)
RUST_PORT=$(port 26656 100)
"$RUST" --home "$R" --chain-id $CHAIN_ID --listen "127.0.0.1:$RUST_PORT" --peer "$VAL_P2P" \
  --rpc "$VAL_RPC" --rpc "$VAL_RPC" --trust-height "$TH" --trust-hash "$THASH" \
  --snapshot-interval "$INTERVAL" --serve-secs "$SERVE" > "$W/rust.json" 2> "$W/rust.log" & RPID=$!; PIDS+=($RPID)
for _ in $(seq 1 180); do
  grep -q "serving snapshot" "$W/rust.log" && break
  kill -0 $RPID 2>/dev/null || fail "rust state-sync node exited: $(cat "$W/rust.json")"
  sleep 1
done
grep -q "serving snapshot" "$W/rust.log" || fail "rust node did not finish state sync"
plain() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }
SYNC_H=$(plain "$W/rust.log" | grep 'Snapshot restored' | grep -o 'height=[0-9]*' | tail -1 | cut -d= -f2)
CHUNKS=$(plain "$W/rust.log" | grep -c 'Applied snapshot chunk to ABCI app' || true)
[ -n "$SYNC_H" ] || fail "no 'Snapshot restored' in the rust log"
RESTORED=$(plain "$W/rust.log" | grep 'Verified ABCI app' | grep -o 'app_hash=[0-9A-F]*' | tail -1 | cut -d= -f2)
WANT=$(header_app_hash $((SYNC_H + 1)))
[ "$RESTORED" = "$WANT" ] || fail "rust restored app hash $RESTORED != validator header $((SYNC_H + 1)) app hash $WANT"
echo "phase 1: Rust node restored the Go snapshot at height $SYNC_H ($CHUNKS chunks applied), app hash $RESTORED matches header $((SYNC_H + 1))"

# --- Phase 2: Rust reactor serves its snapshot -> Go node ------------------
RUST_ID=$("$RUST" --home "$R" --print-id)
celestia-appd init go0 --chain-id $CHAIN_ID --home "$G" >/dev/null 2>&1
cp "$V/config/genesis.json" "$G/config/genesis.json"
cp "$V/config/config.toml" "$G/config/config.toml"
cp "$V/config/app.toml" "$G/config/app.toml"
shift_ports "$G/config/config.toml" 200
shift_ports "$G/config/app.toml" 200
set_toml "$G/config/app.toml" state-sync snapshot-interval 0
sed -i "s#^persistent_peers = .*#persistent_peers = \"$RUST_ID@127.0.0.1:$RUST_PORT\"#" "$G/config/config.toml"
set_toml "$G/config/config.toml" p2p pex false
set_toml "$G/config/config.toml" statesync enable true
set_toml "$G/config/config.toml" statesync rpc_servers "\"$VAL_RPC,$VAL_RPC\""
set_toml "$G/config/config.toml" statesync trust_height "$TH"
set_toml "$G/config/config.toml" statesync trust_hash "\"$THASH\""
set_toml "$G/config/config.toml" statesync discovery_time "\"5s\""
celestia-appd start --home "$G" > "$W/go.log" 2>&1 & GPID=$!; PIDS+=($GPID)
for _ in $(seq 1 120); do
  grep -q "Snapshot restored" "$W/go.log" && break
  kill -0 $GPID 2>/dev/null || fail "go node exited"
  sleep 1
done
grep -q "Snapshot restored" "$W/go.log" || fail "go node did not restore the Rust node's snapshot"
GO_H=$(plain "$W/go.log" | grep 'Snapshot restored' | grep -o 'height=[0-9]*' | tail -1 | cut -d= -f2)
GO_CHUNKS=$(plain "$W/go.log" | grep -c 'Applied snapshot chunk' || true)
echo "phase 2: Go node (only peer: the Rust node) restored the Rust-served snapshot at height $GO_H ($GO_CHUNKS chunks)"
plain "$W/go.log" | grep -q "Verified ABCI app" || fail "go node did not verify its restored app"

# --- Rust node summary ------------------------------------------------------
wait $RPID || fail "rust node reported failure: $(cat "$W/rust.json")"
python3 - "$W/rust.json" "$WANT" <<'PY' || fail "rust summary check failed: $(cat "$W/rust.json")"
import json, sys
s = json.load(open(sys.argv[1]))
assert s["ok"], s["failures"]
assert s["restored_app_hash"] == s["trusted_app_hash"] == sys.argv[2], s
assert s["chunks_served"] > 0, "no chunks served"
print("rust summary:", json.dumps({k: s[k] for k in ("snapshot_height", "sync_seconds", "restored_app_hash", "chunks_served", "chunk_bytes_served", "snapshot_list_requests", "served_snapshot")}))
PY
echo "OK: logs in $W"
trap - EXIT; cleanup
[ -n "${KEEP:-}" ] || rm -rf "$W"
exit 0
