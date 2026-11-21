#!/usr/bin/env bash
# Run on an isolated host with the default local consensus ports available.
set -euo pipefail

binary=$(realpath "${1:?pass the path to the bridge-enabled binary}")
go_tree=$(realpath "${2:?pass the path to the pinned celestia-node checkout}")
probe=$(realpath "$(dirname "$0")/read_bridge.go")
revision=$(git -C "$go_tree" rev-parse HEAD)
case "$revision" in
    d5dfb21a0885622828e30e8e2daf50e5d1bb72b2|af9897bb471959660a4179048656694f3013b2f5) ;;
    *) echo "unexpected celestia-node revision $revision" >&2; exit 1 ;;
esac
work=$(mktemp -d)
pid=
cleanup() {
    if [[ -n "$pid" ]]; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" || true
    fi
    rm -rf -- "$work"
}
trap cleanup EXIT

# Keep the fresh validator keys and the startup log inside the temporary home.
"$binary" start --network local --home "$work/node" \
    --bridge-rpc-listen 127.0.0.1:32658 >"$work/node.log" 2>&1 &
pid=$!
for attempt in {1..120}; do
    kill -0 "$pid"
    if curl -fsS --max-time 2 http://127.0.0.1:32658 \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.LocalHead","params":[]}' 2>/dev/null |
        jq -e '.result.header.chain_id == "celestia-rs-local"' >/dev/null; then
        break
    fi
    if [[ "$attempt" == 120 ]]; then
        echo 'local bridge did not commit a block' >&2
        exit 1
    fi
    sleep 1
done

namespace=726f6c6c7570
data=68656c6c6f2d6461
"$binary" tx blob pay-for-blob "$namespace" "$data" \
    --home "$work/node" --chain-id celestia-rs-local --from validator \
    --keyring-backend test --fees 50000utia --node tcp://127.0.0.1:26657 --wait >"$work/tx.json"
height=$(jq -er 'select(.code == 0) | .height | tonumber | select(. > 0)' "$work/tx.json")
kill -0 "$pid"
echo "celestia-node $revision; chain celestia-rs-local; paid blob at height $height"
sha256sum "$binary" "$work/node/config/genesis.json"
(cd "$go_tree" && go run "$probe" http://127.0.0.1:32658 "$height" "$namespace" "$data")
