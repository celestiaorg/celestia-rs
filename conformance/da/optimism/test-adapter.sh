#!/usr/bin/env bash
# Exercise the OP batch submitter and rollup derivation reader on a fresh private chain.
set -euo pipefail
binary=$(realpath "${1:?expected celestia-rs binary}")
probe=$(realpath "${2:?expected compiled op-batcher test binary}")
work=$(mktemp -d)
pid=
cleanup() {
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" || true; fi
    rm -rf -- "$work"
}
trap cleanup EXIT
DA_TEST_READ_TOKEN=$("$binary" da-auth read --home "$work/node")
DA_TEST_WRITE_TOKEN=$("$binary" da-auth write --home "$work/node")
export DA_TEST_READ_TOKEN DA_TEST_WRITE_TOKEN
"$binary" start --network local --home "$work/node" --bridge-rpc-listen 127.0.0.1:32658 \
    --bridge-signer validator >"$work/node.log" 2>&1 &
pid=$!
for attempt in {1..120}; do
    kill -0 "$pid"
    if curl -fsS --max-time 2 http://127.0.0.1:32658 \
        -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.NetworkHead","params":[]}' 2>/dev/null |
        jq -e '.result.header.chain_id == "celestia-rs-local"' >/dev/null; then break; fi
    if [[ "$attempt" == 120 ]]; then echo 'private Rust bridge did not start' >&2; exit 1; fi
    sleep 1
done
"$probe" -test.v -test.run '^TestRustBridgeRoundTrip$' -test.timeout 3m
