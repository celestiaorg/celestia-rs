#!/usr/bin/env bash
# Reuse a private validator home across a feature rollout and rollback.
set -euo pipefail
baseline=$(realpath "${1:?expected a binary compiled without bridge features}")
enabled=$(realpath "${2:?expected a binary built with bridge-p2p}")
work=$(mktemp -d)
pid=
stop() {
    if [[ -n "$pid" ]]; then kill -TERM "$pid" 2>/dev/null || true; wait "$pid" || true; pid=; fi
}
cleanup() { stop; rm -rf -- "$work"; }
trap cleanup EXIT
home_arg=(--home "$work/node")
start() {
    "$1" start --network local "${home_arg[@]}" "${@:2}" >"$work/node.log" 2>&1 &
    pid=$!
}
block() { curl -fsS --max-time 2 "http://127.0.0.1:26657/block?height=$1" 2>/dev/null; }
wait_height() {
    for attempt in {1..90}; do
        kill -0 "$pid"
        if block "$1" | jq -e --arg h "$1" '.result.block.header.height == $h' >/dev/null; then return; fi
        sleep 1
    done
    echo "consensus did not reach height $1" >&2
    return 1
}
wait_for_new_blocks() {
    wait_height 1
    local latest
    latest=$(curl -fsS --max-time 2 http://127.0.0.1:26657/status | jq -er '.result.sync_info.latest_block_height | tonumber')
    wait_height "$((latest+2))"
}
listeners_off() {
    python3 - <<'PY'
import socket
for port in (32658, 32121):
    with socket.socket() as sock:
        sock.settimeout(1)
        if sock.connect_ex(("127.0.0.1", port)) == 0:
            raise SystemExit(f"unexpected DA listener on port {port}")
PY
}
if "$baseline" start --help | grep -e '--bridge-' >/dev/null; then exit 1; fi
start "$baseline"
wait_height 3
listeners_off
root=$(block 3 | jq -er '.result.block_id.hash')
stop

start "$enabled"
wait_for_new_blocks
listeners_off
[[ ! -e "$work/node/config/da_peer.key" && ! -e "$work/node/config/da-jwt.key" ]]
stop

start "$enabled" --bridge-rpc-listen 127.0.0.1:32658 --bridge-p2p-listen 127.0.0.1:32121
wait_for_new_blocks
token=$("$enabled" da-auth read "${home_arg[@]}")
hash=$(curl -fsS -H "Authorization: Bearer $token" http://127.0.0.1:32658 \
    -d '{"jsonrpc":"2.0","id":1,"method":"header.GetByHeight","params":[3]}' | jq -er '.result.commit.block_id.hash')
[[ "$hash" == "$root" && -s "$work/node/config/da_peer.key" ]]
stop

start "$baseline"
wait_for_new_blocks
listeners_off
[[ $(block 3 | jq -er '.result.block_id.hash') == "$root" ]]
echo "Consensus continued on the original chain after rollback; height 3 hash $root"
