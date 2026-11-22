#!/usr/bin/env bash
# Called by the Rust integration test with its own disposable validator home.
set -euo pipefail
binary=$1
work=$2
phase=$3
pid=
stop() {
    if [[ -n "$pid" ]]; then kill -TERM "$pid" 2>/dev/null || true; wait "$pid" || true; pid=; fi
}
trap stop EXIT
start() {
    "$binary" start --network local --home "$work/node" "$@" >"$work/node.log" 2>&1 &
    pid=$!
}
height() {
    curl -fsS --max-time 1 http://127.0.0.1:26657/status 2>/dev/null | jq -er '.result.sync_info.latest_block_height|tonumber'
}
wait_consensus() {
    for attempt in {1..90}; do
        kill -0 "$pid"
        if [[ $(height || echo 0) -ge $1 ]]; then return; fi
        sleep 1
    done
    return 1
}
if [[ "$phase" == seed ]]; then
    start
    wait_consensus 2
    "$binary" tx blob pay-for-blob 72657461696e c0ffee --home "$work/node" --chain-id celestia-rs-local \
        --from validator --node tcp://127.0.0.1:26657 --keyring-backend test --fees 50000utia --wait >"$work/receipt"
    paid=$(jq -er 'select(.code == 0) | .height | tonumber | select(. > 0)' "$work/receipt")
    wait_consensus "$((paid + 4))"
    stop
    [[ ! -e "$work/node/data/da-bridge" ]]
fi
paid=$(jq -er '.height|tonumber' "$work/receipt")
start --bridge-rpc-listen 127.0.0.1:32658 --bridge-archival
auth=()
if "$binary" da-auth --help >/dev/null 2>&1; then
    token=$("$binary" da-auth read --home "$work/node")
    auth=(-H "Authorization: Bearer $token")
fi
rpc() {
    curl -fsS --max-time 4 "${auth[@]}" http://127.0.0.1:32658 \
        -d "$(jq -nc --arg method "$1" --argjson params "$2" '{jsonrpc:"2.0",id:1,$method,$params}')" 2>/dev/null | jq -ec '.result'
}
target=$((paid+3))
if [[ "$phase" == restart ]]; then target=$(($(cat "$work/seed-head")+1)); fi
for attempt in {1..90}; do
    kill -0 "$pid"
    if rpc header.LocalHead '[]' | jq -e --argjson h "$target" '(.header.height|tonumber)>=$h' >/dev/null; then break; fi
    if [[ $attempt == 90 ]]; then echo 'DA index failed to catch up' >&2; exit 1; fi
    sleep 1
done
rpc header.GetByHeight "[$paid]" >"$work/header-$phase"
rpc share.GetEDS "[$paid]" >"$work/square-$phase"
if [[ "$phase" == seed ]]; then
    rpc header.LocalHead '[]' | jq -er '.header.height|tonumber' >"$work/seed-head"
    kill -KILL "$pid"
    wait "$pid" || true
    pid=
else
    cmp "$work/header-seed" "$work/header-restart"
    cmp "$work/square-seed" "$work/square-restart"
    echo "Reopened the paid square at height $paid after consensus pruning and abrupt DA shutdown"
fi
