#!/usr/bin/env bash
set -euo pipefail

readonly WORK=/workspace/scratch/rpc-181/proxy-tls
cd "$WORK"
PIDS=()

cleanup() {
  for pid in "${PIDS[@]}"; do
    kill "$pid" 2>/dev/null || true
  done
  for pid in "${PIDS[@]}"; do
    wait "$pid" 2>/dev/null || true
  done
  PIDS=()
}
trap cleanup EXIT INT TERM

start_node() {
  local name=$1 rpc=$2 p2p=$3 api=$4 grpc=$5 mode=$6
  ./start-source.sh "$name" "$rpc" "$p2p" "$api" "$grpc" "$mode" \
    >"$name-node.log" 2>&1 &
  PIDS+=("$!")
}

wait_port() {
  local port=$1
  for _ in $(seq 1 120); do
    if python3 - "$port" <<'PY'
import socket, sys
try:
    with socket.create_connection(("127.0.0.1", int(sys.argv[1])), timeout=0.2):
        pass
except OSError:
    raise SystemExit(1)
PY
    then return 0; fi
    sleep 0.25
  done
  echo "port $port did not open" >&2
  return 1
}

wait_height() {
  local endpoint=$1 tls=${2:-false}
  for _ in $(seq 1 120); do
    if python3 - "$endpoint" "$tls" <<'PY'
import json, pathlib, ssl, sys, urllib.request
context = None
if sys.argv[2] == "true":
    context = ssl.create_default_context(cafile="/workspace/scratch/rpc-181/proxy-tls/tls/localhost.crt")
try:
    with urllib.request.urlopen(sys.argv[1] + "/status", timeout=1, context=context) as response:
        height = int(json.load(response)["result"]["sync_info"]["latest_block_height"])
except Exception:
    raise SystemExit(1)
raise SystemExit(0 if height >= 1 else 1)
PY
    then return 0; fi
    sleep 0.25
  done
  echo "$endpoint did not produce a block" >&2
  return 1
}

wait_tls_failure() {
  local log=$1
  for _ in $(seq 1 120); do
    if grep -q 'Error serving server with TLS' "$log" 2>/dev/null; then return 0; fi
    sleep 0.25
  done
  echo "TLS failure did not appear in $log" >&2
  return 1
}

freeze_public() {
  local name=$1
  gzip -n -c "$name-home/config/config.toml" >"$name-config.toml.gz"
  gzip -n -c "$name-home/config/app.toml" >"$name-app.toml.gz"
  gzip -n -c "$name-home/config/genesis.json" >"$name-genesis.json.gz"
}

probe() {
  local case_name=$1 node_name=$2 port=$3 expected=$4
  python3 config-probe-source.py \
    --name "$case_name" \
    --port "$port" \
    --expected "$expected" \
    --config "$node_name-home/config/config.toml" \
    --log "$node_name-node.log" \
    --out "$case_name.json"
}

# Main capture pair: one half-configured plaintext server and one relative-path TLS server.
start_node cert-only 6340 6341 6342 6343 cert-only
start_node relative 6350 6351 6352 6353 relative
wait_port 6340
wait_port 6350
wait_port 6352
wait_height http://127.0.0.1:6340 false
wait_height https://localhost:6350 true
python3 capture-source.py
python3 observe-local-source.py
probe config-cert-only-plaintext cert-only 6340 plain
probe config-relative-path-tls relative 6350 tls
freeze_public cert-only
freeze_public relative
cleanup

# The other half-config and an absolute-path valid pair.
start_node key-only 6360 6361 6362 6363 key-only
start_node absolute 6370 6371 6372 6373 absolute
wait_port 6360
wait_port 6370
probe config-key-only-plaintext key-only 6360 plain
probe config-absolute-path-tls absolute 6370 tls
freeze_public key-only
freeze_public absolute
cleanup

# Both fields present but unusable: RPC must fail closed, never fall back to plaintext.
start_node missing 6380 6381 6382 6383 missing
wait_tls_failure missing-node.log
probe config-missing-pair-no-plaintext-fallback missing 6380 unavailable
freeze_public missing
cleanup

start_node mismatch 6390 6391 6392 6393 mismatch
wait_tls_failure mismatch-node.log
probe config-mismatched-pair-no-plaintext-fallback mismatch 6390 unavailable
freeze_public mismatch
cleanup

python3 aggregate-source.py
