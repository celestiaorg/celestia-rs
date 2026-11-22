#!/usr/bin/env bash
# Use an isolated host with the local Rust consensus and gRPC ports free.
set -euo pipefail
binary=$(realpath "${1:?pass the da-fibre-enabled Rust node}")
node=$(realpath "${2:?pass the pinned celestia-node checkout}")
case "$(git -C "$node" rev-parse HEAD)" in
    d5dfb21a0885622828e30e8e2daf50e5d1bb72b2|af9897bb471959660a4179048656694f3013b2f5) ;;
    *) echo 'unexpected celestia-node release' >&2; exit 1 ;;
esac
source_file=$(realpath "$(dirname "$0")/bridge_download.go")
work=$(mktemp -d)
pid=
cleanup() {
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    rm -rf -- "$work"
}
trap cleanup EXIT
(cd "$node" && go build -p 2 -o "$work/go-provider" "$source_file")
endpoint=http://127.0.0.1:32658
DA_TEST_READ_TOKEN=$("$binary" da-auth read --home "$work/node")
export DA_TEST_READ_TOKEN
"$binary" start --network local --home "$work/node" --bridge-rpc-listen 127.0.0.1:32658 >"$work/node.log" 2>&1 &
pid=$!
for attempt in {1..90}; do
    kill -0 "$pid"
    if curl -fsS --max-time 2 "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.NetworkHead","params":[]}' 2>/dev/null |
        jq -e '.result.header.chain_id == "celestia-rs-local"' >/dev/null; then break; fi
    if [[ "$attempt" == 90 ]]; then echo 'private Rust bridge did not produce a header' >&2; exit 1; fi
    sleep 1
done
if ! "$work/go-provider" "$work/node" 127.0.0.1:9090 "$endpoint" >"$work/provider.log" 2>&1; then
    grep -E '^panic:|^Go ' "$work/provider.log" >&2 || true
    exit 1
fi
grep '^Go ' "$work/provider.log"
