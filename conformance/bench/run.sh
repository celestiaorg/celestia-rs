#!/usr/bin/env bash
# B1 benchmarks, step 2: Go celestia-appd vs celestia-rs on the chain
# recorded by gen-chain.sh, on the same machine, one measurement at a time.
#
#   A. app-only replay (abci_bench over the ABCI socket, identical driver
#      and transport for both apps): FinalizeBlock + Commit per block,
#      PrepareProposal / ProcessProposal (repeated; max-size squares),
#      CheckTx throughput; app RSS and state size at the end.
#   B. block sync: a fresh full node block-syncs the whole chain from the
#      recorded validator (Go celestia-core + celestia-appd); blocks/s.
#   C. startup: the synced full node is stopped (SIGTERM) and restarted;
#      time until RPC answers and until it commits a new height.
#   D. state sync: a fresh full node restores the validator's latest
#      snapshot (light client over the validator's RPC); time to restored
#      state and to the first committed height after it.
#   RSS (/proc VmRSS, VmHWM) and disk (`du` of data/) of every node at the
#   end of each phase.
#
# Writes $BENCH_DIR/results/*.json and results.md.
#
# usage: [BENCH_DIR=target/bench] [BENCH_PHASES="A B C D"] [CELESTIA_RS=path]
#        [ABCI_BENCH=path] [DEVNET_BIN=dir] [DEVNET_PORT_OFFSET=21000]
#        conformance/bench/run.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-21000}
B=${BENCH_DIR:-$ROOT/target/bench}
C=$B/chain
OUTD=$B/results
W=$B/work
PHASES=${BENCH_PHASES:-A B C D}
KINDS=${BENCH_KINDS:-go rs}
RS=${CELESTIA_RS:-$ROOT/target/release/celestia-rs}
# Optional second Rust build for phase A (e.g. the system allocator:
# `--no-default-features --features rocksdb`), reported as rs-alt.
RS_ALT=${CELESTIA_RS_ALT:-}
AB=${ABCI_BENCH:-$ROOT/target/release/examples/abci_bench}
[ -x "$RS" ] || (cd "$ROOT" && cargo build -q --release -p celestia-rs)
[ -x "$AB" ] || (cd "$ROOT" && cargo build -q --release -p devnet-conformance --example abci_bench)
[ -f "$C/meta.json" ] || { echo "run gen-chain.sh first"; exit 1; }
# Phase A needs the recording, B-D the recorded validator home.
case $PHASES in *A*) [ -f "$C/abci.rec" ] || { echo "no $C/abci.rec"; exit 1; } ;; esac
mkdir -p "$OUTD" "$W"
CHAIN_ID=$(python3 -c "import json; print(json.load(open('$C/meta.json'))['chain_id'])")
LAST=$(python3 -c "import json; print(json.load(open('$C/meta.json'))['last_height'])")
# Validator ports (gen-chain.sh layout).
V_RPC=$((26657 + OFF)); V_P2P=$((26656 + OFF)); V_APP=$((36658 + OFF))
APP_PORT=$((36700 + OFF))

PIDS=()
cleanup() { for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done; wait 2>/dev/null || true; }
trap cleanup EXIT
plain() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }
rss() { awk '/^VmRSS:/{r=$2} /^VmHWM:/{h=$2} END{printf "{\"rss_mb\": %.0f, \"hwm_mb\": %.0f}", r/1024, h/1024}' "/proc/$1/status"; }
dumb() { du -sm "$1" | cut -f1; }
height() { curl -s "localhost:$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
now() { date +%s.%N; }
since() { python3 -c "print(round($(now) - $1, 2))"; }
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
wait_port() { for _ in $(seq 1 120); do (exec 3<>"/dev/tcp/127.0.0.1/$1") 2>/dev/null && return 0; sleep 0.25; done; return 1; }

# ------------------------------------------------------------------ A
phase_a() {
  local kind=$1 pid; local h=$W/replay-$kind
  rm -rf "$h"
  if [ "$kind" = go ]; then
    celestia-appd init bench --chain-id "$CHAIN_ID" --home "$h" >/dev/null 2>&1
    cp "$C/genesis.json" "$h/config/genesis.json"
    # Same pruning as the Rust run: keep 100 versions, prune every 10.
    sed -i.bak -e 's#^pruning = .*#pruning = "custom"#' -e 's#^pruning-keep-recent = .*#pruning-keep-recent = "100"#' \
      -e 's#^pruning-interval = .*#pruning-interval = "10"#' -e 's#^snapshot-interval = .*#snapshot-interval = 0#' \
      -e "s#localhost:9090#localhost:$((9190 + OFF))#" -e "s#0.0.0.0:9090#0.0.0.0:$((9190 + OFF))#" \
      -e "s#tcp://localhost:1317#tcp://localhost:$((1417 + OFF))#" "$h/config/app.toml"
    rm -f "$h/config/app.toml.bak"
    celestia-appd start --home "$h" --with-comet=false --address "tcp://127.0.0.1:$APP_PORT" > "$W/replay-$kind.log" 2>&1 &
  else
    mkdir -p "$h"
    local bin=$RS; [ "$kind" = rs-alt ] && bin=$RS_ALT
    RUST_LOG=warn "$bin" abci --home "$h" --addr "127.0.0.1:$APP_PORT" --pruning-keep-recent 100 --pruning-interval 10 \
      --grpc-address "" --api-address "" > "$W/replay-$kind.log" 2>&1 &
  fi
  pid=$!; PIDS+=($pid)
  wait_port $APP_PORT || { tail "$W/replay-$kind.log"; return 1; }
  echo "A/$kind: replaying $(du -sh "$C/abci.rec" | cut -f1) recording (loadavg $(cut -d" " -f1-3 /proc/loadavg))"
  "$AB" "$C/abci.rec" "127.0.0.1:$APP_PORT" "$OUTD/replay-$kind.json" \
    --reps "${BENCH_REPS:-3}" --heavy-reps "${BENCH_HEAVY_REPS:-10}" > "$W/replay-$kind.out"
  local mem; mem=$(rss $pid)
  kill $pid; wait $pid 2>/dev/null || true
  python3 - "$OUTD/replay-$kind.json" "$mem" "$(dumb "$h")" <<'PY'
import json, sys
p = sys.argv[1]; d = json.load(open(p))
d["summary"]["end_memory"] = json.loads(sys.argv[2]); d["summary"]["state_disk_mb"] = int(sys.argv[3])
json.dump(d, open(p, "w"))
s = d["summary"]
print(f"  {s['heights']} heights, {s['txs']} txs: exec {s['exec_blocks_per_s']:.1f} blocks/s, "
      f"finalize+commit p50 {s['finalize_commit']['p50_ms']:.1f} ms p99 {s['finalize_commit']['p99_ms']:.1f} ms; "
      f"RSS {s['end_memory']}, state {s['state_disk_mb']} MB; app hash mismatches {s['app_hash_mismatches']}")
PY
  rm -rf "$h"
}

# ------------------------------------------------------------- source node
start_source() {
  local H=$C/home
  celestia-appd start --home "$H" --with-comet=false --address "tcp://127.0.0.1:$V_APP" >> "$W/src-app.log" 2>&1 &
  SRC_APP=$!; PIDS+=($SRC_APP); wait_port $V_APP
  cometbft start --home "$H/core" --proxy_app "tcp://127.0.0.1:$V_APP" >> "$W/src-core.log" 2>&1 &
  SRC_CORE=$!; PIDS+=($SRC_CORE)
  for _ in $(seq 1 120); do [ -n "$(height $V_RPC)" ] && break; sleep 1; done
  SRC_ID=$(cometbft show-node-id --home "$H/core" 2>/dev/null | tail -1)
  echo "source validator up at height $(height $V_RPC) ($SRC_ID)"
}
stop_source() { kill "$SRC_CORE" 2>/dev/null; wait "$SRC_CORE" 2>/dev/null; kill "$SRC_APP" 2>/dev/null; wait "$SRC_APP" 2>/dev/null || true; }

# full_home <dir> <port-delta>: a full node home peering only with the source.
full_home() {
  rm -rf "$1"
  celestia-appd init full --chain-id "$CHAIN_ID" --home "$1" >/dev/null 2>&1
  cp "$C/genesis.json" "$1/config/genesis.json"
  shift_ports "$1/config/config.toml" $((OFF + $2))
  shift_ports "$1/config/app.toml" $((OFF + $2))
  sed -i.bak -e 's#^addr_book_strict = true#addr_book_strict = false#' -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' \
    -e "s#^persistent_peers = .*#persistent_peers = \"$SRC_ID@127.0.0.1:$V_P2P\"#" -e 's#^pex = true#pex = false#' "$1/config/config.toml"
  rm -f "$1/config/config.toml.bak"
  set_toml "$1/config/app.toml" state-sync snapshot-interval 0
}
start_full() { # kind home log [extra...]
  local kind=$1 home=$2 log=$3; shift 3
  if [ "$kind" = go ]; then
    celestia-appd start --home "$home" "$@" >> "$log" 2>&1 &
  else
    # [p2p] laddr (`tcp://0.0.0.0:<port>`; [rpc] laddr is on 127.0.0.1).
    local p2p; p2p=127.0.0.1:$(sed -n 's#^laddr = "tcp://0.0.0.0:\([0-9]*\)".*#\1#p' "$home/config/config.toml" | head -1)
    RUST_LOG=${RUST_LOG:-info} "$RS" start --home "$home" --p2p-laddr "$p2p" --persistent-peers "$SRC_ID@127.0.0.1:$V_P2P" \
      --allow-duplicate-ip --moniker full "$@" >> "$log" 2>&1 &
  fi
  FULL_PID=$!; PIDS+=($FULL_PID)
}
rpc_of() { sed -n 's#^laddr = "tcp://127.0.0.1:\([0-9]*\)".*#\1#p' "$1/config/config.toml" | head -1; }

# ---------------------------------------------------------- B + C
phase_bc() {
  local kind=$1 delta port t0 secs mem; local h=$W/full-$kind
  delta=$([ "$kind" = go ] && echo 200 || echo 300)
  full_home "$h" $delta
  port=$(rpc_of "$h")
  : > "$W/full-$kind.log"
  t0=$(now)
  start_full "$kind" "$h" "$W/full-$kind.log"
  until [ "$(height "$port" || echo 0)" -ge "$LAST" ] 2>/dev/null; do
    kill -0 $FULL_PID 2>/dev/null || { tail "$W/full-$kind.log"; return 1; }
    sleep 0.5
  done
  secs=$(since "$t0")
  mem=$(rss $FULL_PID)
  echo "B/$kind: block-synced $LAST heights in ${secs}s; $mem; disk $(dumb "$h/data") MB"
  echo "{\"kind\": \"$kind\", \"heights\": $LAST, \"secs\": $secs, \"memory\": $mem, \"disk_mb\": $(dumb "$h/data")}" > "$OUTD/blocksync-$kind.json"
  # C: restart (handshake on the synced stores).
  sleep 5
  kill -TERM $FULL_PID; wait $FULL_PID 2>/dev/null || true
  local hb ready=""; hb=$(height $V_RPC)
  t0=$(now)
  start_full "$kind" "$h" "$W/full-$kind.log"
  until [ -n "$(height "$port")" ]; do kill -0 $FULL_PID || return 1; sleep 0.05; done
  ready=$(since "$t0")
  until [ "$(height "$port" || echo 0)" -gt "$hb" ] 2>/dev/null; do kill -0 $FULL_PID || return 1; sleep 0.1; done
  secs=$(since "$t0")
  mem=$(rss $FULL_PID)
  echo "C/$kind: restart: RPC up after ${ready}s, new height after ${secs}s; $mem"
  echo "{\"kind\": \"$kind\", \"rpc_up_s\": $ready, \"first_new_height_s\": $secs, \"memory\": $mem}" > "$OUTD/startup-$kind.json"
  kill -TERM $FULL_PID; wait $FULL_PID 2>/dev/null || true
  rm -rf "$h"
}

# ---------------------------------------------------------------- D
phase_d() {
  local kind=$1 delta port t0 th thash restored secs mem snap; local h=$W/ss-$kind
  delta=$([ "$kind" = go ] && echo 400 || echo 500)
  full_home "$h" $delta
  port=$(rpc_of "$h")
  snap=$(ls "$C/home/data/snapshots" 2>/dev/null | grep -E '^[0-9]+$' | sort -n | tail -1)
  th=$((snap > 10 ? snap - 10 : 1))
  thash=$(curl -s "localhost:$V_RPC/block?height=$th" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block_id"]["hash"])')
  : > "$W/ss-$kind.log"
  t0=$(now)
  if [ "$kind" = go ]; then
    set_toml "$h/config/config.toml" statesync enable true
    set_toml "$h/config/config.toml" statesync rpc_servers "\"127.0.0.1:$V_RPC,127.0.0.1:$V_RPC\""
    set_toml "$h/config/config.toml" statesync trust_height "$th"
    set_toml "$h/config/config.toml" statesync trust_hash "\"$thash\""
    set_toml "$h/config/config.toml" statesync discovery_time "\"5s\""
    start_full go "$h" "$W/ss-$kind.log"
  else
    start_full rs "$h" "$W/ss-$kind.log" --statesync-rpc-servers "127.0.0.1:$V_RPC,127.0.0.1:$V_RPC" \
      --statesync-trust-height "$th" --statesync-trust-hash "$thash" --statesync-discovery-secs 5
  fi
  until plain "$W/ss-$kind.log" | grep -qiE 'snapshot restored|state sync complete'; do
    kill -0 $FULL_PID 2>/dev/null || { tail "$W/ss-$kind.log"; return 1; }
    sleep 0.2
  done
  restored=$(since "$t0")
  until [ "$(height "$port" || echo 0)" -gt "$snap" ] 2>/dev/null; do kill -0 $FULL_PID || return 1; sleep 0.2; done
  secs=$(since "$t0")
  mem=$(rss $FULL_PID)
  echo "D/$kind: state sync of snapshot $snap: restored after ${restored}s, first block after ${secs}s; $mem; disk $(dumb "$h/data") MB"
  echo "{\"kind\": \"$kind\", \"snapshot_height\": $snap, \"restored_s\": $restored, \"first_height_s\": $secs, \"memory\": $mem, \"disk_mb\": $(dumb "$h/data")}" > "$OUTD/statesync-$kind.json"
  kill -TERM $FULL_PID; wait $FULL_PID 2>/dev/null || true
  rm -rf "$h"
}

for p in $PHASES; do
  case $p in
    A) for k in $KINDS; do phase_a "$k"; done; [ -z "$RS_ALT" ] || phase_a rs-alt ;;
  esac
done
if echo "$PHASES" | grep -qE 'B|C|D'; then
  start_source
  for p in $PHASES; do
    case $p in
      B|C) [ "${DONE_BC:-}" ] || { for k in $KINDS; do phase_bc "$k"; done; DONE_BC=1; } ;;
      D) for k in $KINDS; do phase_d "$k"; done ;;
    esac
  done
  stop_source
fi
python3 "$ROOT/conformance/bench/report.py" "$OUTD" | tee "$OUTD/results.md"
