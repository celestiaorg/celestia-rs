#!/usr/bin/env bash
# The caller supplies its private release bridge and paid block.
set -euo pipefail
rust=$1
provider=$2
work=$3
chain=$4
upstream=$5
hash=$6
height=$7
light=
peer=
cleanup() {
    for pid in "$light" "$peer"; do
        if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    done
}
trap cleanup EXIT
mkdir "$work"
mode() {
    echo "$1" >"$work/mode.new"
    mv "$work/mode.new" "$work/mode"
}
mode missing
"$provider" "$chain" "$upstream" "$work" >"$work/addr" &
peer=$!
for attempt in {1..60}; do
    kill -0 "$peer"
    if [[ -s "$work/addr" ]]; then break; fi
    sleep 0.25
done
args=(da-light --home "$work/home" --network "$chain" --peer "$(cat "$work/addr")" --trusted-hash "$hash" --until "$height")
"$rust" "${args[@]}" >"$work/first.log" &
light=$!
failed_sample() {
    for attempt in {1..60}; do
        kill -0 "$light"
        if [[ -f "$work/requests" ]] && awk -v phase="$1" '$1==phase {count++} END{exit count<2}' "$work/requests"; then return; fi
        sleep 0.5
    done
    echo "missing repeated requests for $1 sample" >&2
    exit 1
}
failed_sample missing
mode neighbor
failed_sample neighbor
kill -INT "$light"
wait "$light"
light=
if grep -q 'sampled height' "$work/first.log"; then echo 'unavailable block marked sampled' >&2; exit 1; fi
mode available
timeout 120 "$rust" "${args[@]}" >"$work/resumed.log"
grep -Fq "sampled height $height (16 verified shares), hash $hash" "$work/resumed.log"
awk '$1!="available" {if (!cell) cell=$2; if ($2!=cell) exit 1}
     $1=="available" && !resumed {if ($2!=cell) exit 1; resumed=1}
     END {if (!resumed) exit 1}' "$work/requests"
echo "Rust retried the same missing cell, rejected the neighboring proof, and verified all 16 shares after restart"

# This private peer serves namespace rows and has no EDS protocol handler.
endpoint=http://127.0.0.1:39659
DA_TEST_READ_TOKEN=$("$rust" da-auth read --home "$work/home")
export DA_TEST_READ_TOKEN
"$rust" da-light --home "$work/home" --network "$chain" --peer "$(cat "$work/addr")" \
    --trusted-hash "$hash" --rpc-listen 127.0.0.1:39659 >"$work/namespace.log" &
light=$!
for attempt in {1..60}; do
    kill -0 "$light"
    if curl -fsS --max-time 2 "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.LocalHead","params":[]}' 2>/dev/null | jq -e '.result.header.height' >/dev/null; then break; fi
    sleep 0.25
done
"$8" "$endpoint" "$height" "$9" "$work/wrong-namespace"
