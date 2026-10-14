#!/usr/bin/env bash
# State-sync interop devnet (ADR 0002):
#
#  1. Go -> Rust: a Go celestia-appd validator takes snapshots; a fresh
#     follower (Go celestia-core + `celestia-rs abci --home`) state-syncs from
#     them, i.e. the Rust app restores Go-produced chunks, then follows the
#     chain with celestia-core checking every app hash.
#  2. Restart: the Rust follower (app and celestia-core) is killed and
#     restarted; the ABCI handshake resumes it from its persistent state.
#  3. Rust -> Go: the Rust follower takes its own snapshots; a fresh Go
#     celestia-appd node whose ONLY peer is the Rust follower state-syncs from
#     them and then follows the chain (Go verifies the restored app hash and
#     every later block).
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/devnet/statesync.sh [blocks-after-sync] [workload.sh]
# The optional workload (see workloads/) runs on the validator before the
# first snapshot that is synced, so the snapshot carries non-trivial state.
# Exit 0 iff all three phases succeed.
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
EXTRA=${1:-15}; WORKLOAD=${2:-}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-6000}
CHAIN_ID=rs-statesync-1
INTERVAL=10
W=$(mktemp -d)
V=$W/val; R=$W/rust; G=$W/go
RS="$ROOT/target/debug/celestia-rs"
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
wait_height() { # rpc-port height log [pid]
  for _ in $(seq 1 300); do
    local x; x=$(h "$1" || true)
    [ "${x:-0}" -ge "$2" ] 2>/dev/null && return 0
    if [ -n "${4:-}" ] && ! kill -0 "$4" 2>/dev/null; then return 1; fi
    if grep -q "wrong Block.Header.AppHash\|CONSENSUS FAILURE\|panic:" "$3" 2>/dev/null; then return 1; fi
    sleep 1
  done
  return 1
}
trust() { # -> "height hash" of a recent validator block
  local th=$(( $(h "$(port 26657 0)") - 2 ))
  echo "$th $(curl -s "$VAL_RPC/block?height=$th" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block_id"]["hash"])')"
}
statesync_config() { # config.toml trust-height trust-hash
  set_toml "$1" statesync enable true
  set_toml "$1" statesync rpc_servers "\"$VAL_RPC,$VAL_RPC\""
  set_toml "$1" statesync trust_height "$2"
  set_toml "$1" statesync trust_hash "\"$3\""
  set_toml "$1" statesync discovery_time "\"5s\""
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

# --- Phase 1: Go snapshot -> Rust app -------------------------------------
mkdir -p "$R/config" "$R/data"
cometbft init --home "$R" >/dev/null 2>&1 || true
cp "$V/config/genesis.json" "$R/config/genesis.json.sdk"
python3 - "$V" "$R" <<'PY'
import json, sys
v, f = sys.argv[1:]
g = json.load(open(v + "/config/genesis.json"))
c = {"genesis_time": g["genesis_time"], "chain_id": g["chain_id"],
     "initial_height": str(g["initial_height"]),
     "consensus_params": g["consensus"]["params"], "validators": [],
     "app_hash": "", "app_state": g["app_state"]}
json.dump(c, open(f + "/config/genesis.json", "w"))
PY
cp "$V/config/config.toml" "$R/config/config.toml"
shift_ports "$R/config/config.toml" 100
sed -i "s#^persistent_peers = .*#persistent_peers = \"$VAL_P2P\"#" "$R/config/config.toml"
read -r TH THASH < <(trust)
statesync_config "$R/config/config.toml" "$TH" "$THASH"
start_rust() {
  "$RS" abci --addr "127.0.0.1:$(port 26658 100)" --home "$R/app" --chain-id $CHAIN_ID \
    --snapshot-interval $INTERVAL --pruning-keep-recent 20 >> "$W/rust.log" 2>&1 & RPID=$!; PIDS+=($RPID)
  sleep 1
  cometbft start --home "$R" --proxy_app "tcp://127.0.0.1:$(port 26658 100)" >> "$W/rustcore.log" 2>&1 & RCPID=$!; PIDS+=($RCPID)
}
start_rust
for _ in $(seq 1 120); do
  grep -q "restored state-sync snapshot" "$W/rust.log" && break
  kill -0 $RCPID 2>/dev/null || fail "rust follower's celestia-core exited during state sync"
  sleep 1
done
grep -q "restored state-sync snapshot" "$W/rust.log" || fail "rust app did not restore a Go snapshot"
SYNC_H=$(sed 's/\x1b\[[0-9;]*m//g' "$W/rust.log" | grep 'restored state-sync snapshot' | grep -o 'height=[0-9]*' | tail -1 || true)
echo "phase 1: Rust app restored Go snapshot (${SYNC_H:-?})"
T1=$(( $(h "$(port 26657 0)") + EXTRA ))
wait_height "$(port 26657 100)" "$T1" "$W/rustcore.log" "$RCPID" || fail "rust follower did not follow to $T1"
echo "phase 1: Rust follower verified blocks up to $(h "$(port 26657 100)")"

# --- Phase 2: restart from disk --------------------------------------------
kill $RCPID; wait $RCPID 2>/dev/null || true
kill $RPID; wait $RPID 2>/dev/null || true
BEFORE=$(grep -c "loaded persistent state" "$W/rust.log" || true)
set_toml "$R/config/config.toml" statesync enable false
start_rust
sed 's/\x1b\[[0-9;]*m//g' "$W/rust.log" | grep "loaded persistent state" | tail -1 || true
T2=$(( $(h "$(port 26657 0)") + EXTRA ))
wait_height "$(port 26657 100)" "$T2" "$W/rustcore.log" "$RCPID" || fail "restarted rust follower did not follow to $T2"
[ "$(grep -c "loaded persistent state" "$W/rust.log")" -gt "$BEFORE" ] || fail "no restart"
echo "phase 2: restarted Rust follower resumed from disk and reached $(h "$(port 26657 100)")"

# --- Phase 3: Rust snapshot -> Go app --------------------------------------
for _ in $(seq 1 120); do
  ls "$R"/app/data/snapshots/*/3/metadata >/dev/null 2>&1 && break; sleep 1
done
RSNAP=$(ls -d "$R"/app/data/snapshots/*/3/metadata 2>/dev/null | sed 's#.*/snapshots/\([0-9]*\)/.*#\1#' | sort -n | tail -1)
[ -n "$RSNAP" ] || fail "rust follower produced no snapshot"
echo "phase 3: Rust snapshots at heights: $(ls "$R"/app/data/snapshots | grep -E '^[0-9]+$' | sort -n | tr '\n' ' ')"
celestia-appd init go0 --chain-id $CHAIN_ID --home "$G" >/dev/null 2>&1
cp "$V/config/genesis.json" "$G/config/genesis.json"
cp "$V/config/config.toml" "$G/config/config.toml"
cp "$V/config/app.toml" "$G/config/app.toml"
shift_ports "$G/config/config.toml" 200
shift_ports "$G/config/app.toml" 200
set_toml "$G/config/app.toml" state-sync snapshot-interval 0
RUST_ID=$(cometbft show_node_id --home "$R" 2>/dev/null | tail -1)
sed -i "s#^persistent_peers = .*#persistent_peers = \"$RUST_ID@127.0.0.1:$(port 26656 100)\"#" "$G/config/config.toml"
set_toml "$G/config/config.toml" p2p pex false
read -r TH THASH < <(trust)
statesync_config "$G/config/config.toml" "$TH" "$THASH"
celestia-appd start --home "$G" > "$W/go.log" 2>&1 & GPID=$!; PIDS+=($GPID)
T3=$(( $(h "$(port 26657 0)") + EXTRA ))
wait_height "$(port 26657 200)" "$T3" "$W/go.log" "$GPID" || fail "go node did not sync from the rust node"
grep -E "Snapshot restored|Applied snapshot chunk|snapshot restored" "$W/go.log" | tail -2 || true
grep -q "Snapshot restored\|snapshot restored" "$W/go.log" || fail "go node did not restore a snapshot"
echo "phase 3: Go node (only peer: Rust follower) restored a Rust snapshot and reached $(h "$(port 26657 200)")"
echo "OK: logs in $W"
trap - EXIT; cleanup; exit 0
