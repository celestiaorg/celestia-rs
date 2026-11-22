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
peer_args=()
if [[ "${3:-}" == --peer || "${3:-}" == --stock-light || "${3:-}" == --validator-load ]]; then
    peer_args=(--prometheus-listen-addr 127.0.0.1:32660 --bridge-p2p-listen 127.0.0.1:32121 --bridge-p2p-listen /ip4/127.0.0.1/udp/32121/quic-v1)
fi
pid=
bootstrap_pid=
light_pid=
cleanup() {
    if [[ -n "${subscriber:-}" ]]; then kill "$subscriber" 2>/dev/null || true; wait "$subscriber" || true; fi
    if [[ -n "$bootstrap_pid" ]]; then
        kill "$bootstrap_pid" 2>/dev/null || true
        wait "$bootstrap_pid" || true
    fi
    if [[ -n "$light_pid" ]]; then
        kill "$light_pid" 2>/dev/null || true
        wait "$light_pid" || true
    fi
    if [[ -n "$pid" ]]; then
        kill "$pid" 2>/dev/null || true
        wait "$pid" || true
    fi
    rm -rf -- "$work"
}
trap cleanup EXIT

if (( ${#peer_args[@]} )); then
    (cd "$go_tree" && go build -o "$work/bootstrap-probe" "$(dirname "$probe")/bootstrap/check.go")
    "$work/bootstrap-probe" "$work/bootstrap.addr" celestia-rs-local &
    bootstrap_pid=$!
    for attempt in {1..30}; do
        if [[ -s "$work/bootstrap.addr" ]]; then break; fi
        kill -0 "$bootstrap_pid"
        sleep 1
    done
    peer_args+=(--bridge-bootstrap "$(cat "$work/bootstrap.addr")")
fi
DA_TEST_READ_TOKEN=$("$binary" da-auth read --home "$work/node")
export DA_TEST_READ_TOKEN
# Keep the fresh validator keys and the startup log inside the temporary home.
rust_token=$DA_TEST_READ_TOKEN
"$binary" keys add da-secondary --no-backup --keyring-backend test --home "$work/node" >/dev/null 2>&1
DA_TEST_SECOND_SIGNER=$("$binary" keys show da-secondary --address --keyring-backend test --home "$work/node")
export DA_TEST_SECOND_SIGNER
start_rust() {
    local first= head
    RUST_LOG=celestia_rs::bridge::peer=info "$binary" start --network local --home "$work/node" \
        --bridge-rpc-listen 127.0.0.1:32658 --bridge-signer validator --bridge-signer da-secondary "${peer_args[@]}" >"$work/node.log" 2>&1 &
    pid=$!
    for attempt in {1..120}; do
        kill -0 "$pid"
        if head=$(curl -fsS --max-time 2 http://127.0.0.1:32658 \
            -H "Authorization: Bearer $rust_token" \
            -d '{"jsonrpc":"2.0","id":1,"method":"header.LocalHead","params":[]}' 2>/dev/null |
            jq -er '.result.header | select(.chain_id == "celestia-rs-local") | .height | tonumber'); then
            # A restarted node can serve its previous head before it finalizes a new block.
            if [[ -n "$first" ]] && (( head > first )); then break; fi
            first=${first:-$head}
        fi
        if [[ "$attempt" == 120 ]]; then
            echo 'local bridge did not commit a block' >&2
            exit 1
        fi
        sleep 1
    done
}
start_rust

if [[ -n "$bootstrap_pid" ]]; then
    wait "$bootstrap_pid"
    bootstrap_pid=
fi

namespace=726f6c6c7570
# Span four ODS rows to exercise the per-row commitment and inclusion proofs.
data=$(python3 -c 'print("c0ffee" * 4000)')
pay_for_blob() {
    "$binary" tx blob pay-for-blob "$namespace" "$data" \
        --home "$work/node" --chain-id celestia-rs-local --from validator \
        --keyring-backend test --fees 50000utia --node tcp://127.0.0.1:26657 --wait >"$work/tx.json"
    height=$(jq -er 'select(.code == 0) | .height | tonumber | select(. > 0)' "$work/tx.json")
}
if (( ${#peer_args[@]} )); then
    peer_id=$(sed -n 's/.*DA peer listening.*peer=\([[:alnum:]]*\).*/\1/p' "$work/node.log" | head -1)
    (cd "$go_tree" && go build -o "$work/notification" "$(dirname "$probe")/notifications/check.go")
    "$work/notification" celestia-rs-local "/ip4/127.0.0.1/tcp/32121/p2p/$peer_id" "$work/subscribed" >"$work/announcement" &
    subscriber=$!
    for attempt in {1..60}; do
        if [[ -s "$work/subscribed" ]]; then break; fi
        kill -0 "$subscriber"
        sleep 1
    done
    test -s "$work/subscribed"
fi
pay_for_blob
if [[ -n "${subscriber:-}" ]]; then
    wait "$subscriber"
    subscriber=
    grep -F "via Rust relay $peer_id at height $height" "$work/announcement"
    grep -F "signed EDS notification at height $height from $peer_id" "$work/announcement"
fi
kill -0 "$pid"
echo "celestia-node $revision; chain celestia-rs-local; paid blob at height $height"
sha256sum "$binary" "$work/node/config/genesis.json"
(cd "$go_tree" && go run "$probe" http://127.0.0.1:32658 "$height" "$namespace" "$data")
(cd "$go_tree" && go run "$probe" http://127.0.0.1:32658 --next-height)
(cd "$go_tree" && go run "$probe" http://127.0.0.1:32658 --genesis-tail)
(cd "$go_tree" && go run "$(dirname "$probe")/subscriptions/check.go" ws://127.0.0.1:32658 "$height" "$namespace" "$data")
DA_TEST_WRITE_TOKEN=$("$binary" da-auth write --home "$work/node")
export DA_TEST_WRITE_TOKEN
(cd "$go_tree" && go run "$(dirname "$probe")/submit/check.go" http://127.0.0.1:32658)
# Create a second operator so redelegation can be validated on chain.
key_args=(--home "$work/node" --keyring-backend test)
tx_args=("${key_args[@]}" --chain-id celestia-rs-local --fees 50000utia --gas 500000 --node tcp://127.0.0.1:26657 --wait)
"$binary" keys add destination --no-backup "${key_args[@]}" >/dev/null 2>&1
destination=$("$binary" keys show destination --address "${key_args[@]}")
operator=$("$binary" keys show destination --address --bech val "${key_args[@]}")
"$binary" tx bank send validator "$destination" 3000000utia "${tx_args[@]}" | jq -e '.code == 0' >/dev/null
validator_json="$(dirname "$probe")/../../crates/tier-5-node/celestia-rs/tests/fixtures/generate-only/validator.json"
created=$("$binary" tx staking create-validator "$validator_json" --from destination "${tx_args[@]}" | jq -er 'select(.code == 0) | .height')
(cd "$go_tree" && go run "$(dirname "$probe")/staking/check.go" http://127.0.0.1:32658 "$operator")
(cd "$go_tree" && go run "$(dirname "$probe")/fibre/escrow.go" http://127.0.0.1:32658)
(cd "$go_tree" && go run "$(dirname "$probe")/staking/validator_set.go" http://127.0.0.1:32658 "$created" "$(jq -er '.pubkey.key' "$validator_json")")
unset DA_TEST_WRITE_TOKEN
DA_TEST_ADMIN_TOKEN=$("$binary" da-auth admin --home "$work/node")
export DA_TEST_ADMIN_TOKEN
(cd "$go_tree" && DA_TEST_LOG="$work/node.log" go run "$(dirname "$probe")/auth/check.go" http://127.0.0.1:32658)
if (( ${#peer_args[@]} )); then
    peer_id=$(sed -n 's/.*DA peer listening.*peer=\([[:alnum:]]*\).*/\1/p' "$work/node.log" | head -1)
    [[ -n "$peer_id" ]] || { echo 'missing DA peer identity' >&2; exit 1; }
    peer_probe=$(dirname "$probe")/peer/read_peer.go
    (cd "$go_tree" && DA_TEST_ADMIN_URL=http://127.0.0.1:32658 go run "$peer_probe" celestia-rs-local "/ip4/127.0.0.1/tcp/32121/p2p/$peer_id" "$height" "$namespace" "$data")
    (cd "$go_tree" && DA_TEST_ADMIN_URL=http://127.0.0.1:32658 go run "$peer_probe" celestia-rs-local "/ip4/127.0.0.1/udp/32121/quic-v1/p2p/$peer_id" "$height" "$namespace" "$data")
    verify_count() {
        local protocol=$1 outcome=$2 minimum=$3
        awk -v series="protocol=\"$protocol\",outcome=\"$outcome\"" -v minimum="$minimum" '
            /_da_stream_requests_total\{/ && index($0, series) && $NF >= minimum {found=1}
            END {exit !found}' "$work/metrics"
    }
    curl -fsS http://127.0.0.1:32660/metrics >"$work/metrics"
    for kind in header eds row sample range namespace; do verify_count "$kind" served 1; done
    verify_count sample served 16
    verify_count eds not_found 1
    echo "Verified Rust peer $peer_id served all six protocols, 16 samples and a missing EDS response"
fi
unset DA_TEST_ADMIN_TOKEN
if [[ "${3:-}" == --validator-load ]]; then
    (cd "$go_tree" && go build -o "$work/rpc-read" "$probe" &&
        go build -o "$work/peer-read" "$peer_probe" &&
        go build -o "$work/commits" "$(dirname "$probe")/validator/commits.go")
    python3 "$(dirname "$probe")/validator/measure.py" "$pid" "$work" \
        "/ip4/127.0.0.1/tcp/32121/p2p/$peer_id" "$height" "$namespace" "$data"
fi
if [[ "${3:-}" == --stock-light ]]; then
    (cd "$go_tree" && go build -o "$work/go-da" ./cmd/celestia)
    genesis=$(curl -fsS 'http://127.0.0.1:26657/block?height=1' | jq -er '.result.block_id.hash')
    addr="/ip4/127.0.0.1/tcp/32121/p2p/$peer_id"
    export CELESTIA_CUSTOM="celestia-rs-local:$genesis:$addr"
    "$work/go-da" light init --node.store "$work/light" >"$work/init.log" 2>&1
    "$work/go-da" light start --node.store "$work/light" --headers.trusted-peers "$addr" \
        --rpc.port 32659 >"$work/light.log" 2>&1 &
    light_pid=$!
    for attempt in {1..120}; do
        kill -0 "$light_pid"
        if curl -sS --max-time 1 -o /dev/null http://127.0.0.1:32659 2>/dev/null; then break; fi
        sleep 1
    done
    DA_TEST_READ_TOKEN=$("$work/go-da" light auth read --node.store "$work/light" | tail -1)
    export DA_TEST_READ_TOKEN
    (cd "$go_tree" && go build -o "$work/read" "$probe")
    read_from_light() {
        for attempt in {1..10}; do
            if "$work/read" http://127.0.0.1:32659 "$height" "$namespace" "$data" >"$work/read.log" 2>&1; then
                cat "$work/read.log"
                return
            fi
            kill -0 "$light_pid"
            sleep 3
        done
        cat "$work/read.log" >&2
        echo 'Go light node did not recover the paid blob' >&2
        return 1
    }
    read_from_light
    kill -TERM "$pid"
    wait "$pid" || true
    pid=
    kill -0 "$light_pid"
    if curl -s --max-time 1 -o /dev/null http://127.0.0.1:32658; then
        echo "Rust bridge RPC is still reachable after shutdown" >&2
        exit 1
    fi
    start_rust
    recovered_id=$(sed -n 's/.*DA peer listening.*peer=\([[:alnum:]]*\).*/\1/p' "$work/node.log" | head -1)
    [[ "$recovered_id" == "$peer_id" ]]
    data=$(python3 -c 'print("a4b2c6" * 4000)')
    pay_for_blob
    read_from_light
    # Restart reset the registry and the explicit peer probe exited before the
    # first shutdown. Only the stock light process can issue these SHREX reads.
    curl -fsS http://127.0.0.1:32660/metrics >"$work/metrics"
    verify_count namespace served 1
    echo "Stock Go light read the restarted Rust peer; namespace requests:"
    awk '/_da_stream_requests_total\{/ && /protocol="namespace",outcome="served"/' "$work/metrics"
    echo "The same Go light process recovered a new paid blob at $height from restarted Rust peer $peer_id"
fi
