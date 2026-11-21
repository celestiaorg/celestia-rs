#!/usr/bin/env bash
# Run on an isolated host with the default local consensus ports available.
set -euo pipefail
binary=$(realpath "${1:?pass the bridge-enabled Rust binary}")
probe=$(realpath "${2:?pass the compiled Sovereign adapter probe}")
work=$(mktemp -d)
node=
cleanup() {
    if [[ -n "$node" ]]; then kill "$node" 2>/dev/null || true; wait "$node" || true; fi
    rm -rf -- "$work"
}
trap cleanup EXIT
export SOV_CELESTIA_RPC_URL=ws://127.0.0.1:32658 SOV_CELESTIA_GRPC_URL=http://127.0.0.1:9090
SOV_CELESTIA_RPC_AUTH_TOKEN=$("$binary" da-auth read --home "$work/node")
export SOV_CELESTIA_RPC_AUTH_TOKEN
"$binary" start --network local --home "$work/node" --bridge-rpc-listen 127.0.0.1:32658 >"$work/node.log" 2>&1 &
node=$!
for attempt in {1..120}; do
    kill -0 "$node"
    if curl -fsS --max-time 2 http://127.0.0.1:32658 \
        -H "Authorization: Bearer $SOV_CELESTIA_RPC_AUTH_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.NetworkHead","params":[]}' 2>/dev/null |
        jq -e '.result.header.chain_id == "celestia-rs-local"' >/dev/null; then break; fi
    if [[ "$attempt" == 120 ]]; then echo 'private Rust bridge did not start' >&2; exit 1; fi
    sleep 1
done
keys=(--home "$work/node" --keyring-backend test)
"$binary" keys add adapter "${keys[@]}" >/dev/null 2>&1
addr=$("$binary" keys show adapter -a "${keys[@]}")
"$binary" tx bank send validator "$addr" 100000000utia "${keys[@]}" --chain-id celestia-rs-local \
    --node tcp://127.0.0.1:26657 --fees 20000utia | jq -e '.code == 0' >/dev/null
for attempt in {1..60}; do
    if "$binary" query bank balances "$addr" --node tcp://127.0.0.1:26657 --home "$work/node" |
        jq -e '.balances[] | select(.denom == "utia" and .amount == "100000000")' >/dev/null; then break; fi
    if [[ "$attempt" == 60 ]]; then echo 'adapter account was not funded' >&2; exit 1; fi
    sleep 1
done
# The adapter signs locally. Only its freshly generated test account leaves the temporary keyring.
SOV_CELESTIA_SIGNER_KEY=$("$binary" keys export adapter --unsafe --unarmored-hex "${keys[@]}" 2>/dev/null)
export SOV_CELESTIA_SIGNER_KEY
timeout 180 "$probe"
