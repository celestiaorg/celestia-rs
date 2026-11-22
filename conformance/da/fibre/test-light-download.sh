#!/usr/bin/env bash
# Invoked by the Go FSP suite while its validator, escrow and storage are live.
set -euo pipefail
work=$(mktemp -d)
light=
cleanup() {
    if [[ -n "$light" ]]; then kill "$light" 2>/dev/null || true; wait "$light" 2>/dev/null || true; fi
    rm -rf -- "$work"
}
trap cleanup EXIT
core=${1#http://}
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
rpc=http://127.0.0.1:$port
peer=/ip4/127.0.0.1/tcp/1/p2p/12D3KooWLiGKA7tFRoMSfcztssRgUzthfpJP1cAm16Fhztkn46rk
DA_TEST_READ_TOKEN=$("$DA_FIBRE_NODE" da-auth read --home "$work")
export DA_TEST_READ_TOKEN
"$DA_FIBRE_NODE" da-light --home "$work" --network "$2" --peer "$peer" \
    --core.ip "${core%:*}" --core.port "${core##*:}" --rpc-listen "127.0.0.1:$port" >"$work/light.log" 2>&1 &
light=$!
for attempt in {1..40}; do
    if ! kill -0 "$light" 2>/dev/null; then tail -6 "$work/light.log" >&2; exit 1; fi
    if curl -sS --max-time 1 -o /dev/null "$rpc/" 2>/dev/null; then break; fi
    sleep 0.25
done
"$DA_FIBRE_GO_CLIENT" "$rpc" "$3" "$4"
