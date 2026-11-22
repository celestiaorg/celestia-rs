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
light=
cleanup() {
    if [[ -n "$light" ]]; then kill "$light" 2>/dev/null || true; wait "$light" 2>/dev/null || true; fi
    if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    rm -rf -- "$work"
}
trap cleanup EXIT
(cd "$node" && go build -p 2 -o "$work/go-provider" "$source_file")
endpoint=http://127.0.0.1:32658
DA_TEST_READ_TOKEN=$("$binary" da-auth read --home "$work/node")
DA_TEST_WRITE_TOKEN=$("$binary" da-auth write --home "$work/node")
export DA_TEST_READ_TOKEN DA_TEST_WRITE_TOKEN
"$binary" start --network local --home "$work/node" --bridge-signer validator --bridge-rpc-listen 127.0.0.1:32658 >"$work/node.log" 2>&1 &
pid=$!
for attempt in {1..90}; do
    kill -0 "$pid"
    if curl -fsS --max-time 2 "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.NetworkHead","params":[]}' 2>/dev/null |
        jq -e '.result.header.chain_id == "celestia-rs-local"' >/dev/null; then break; fi
    if [[ "$attempt" == 90 ]]; then echo 'private Rust bridge did not produce a header' >&2; exit 1; fi
    sleep 1
done
if [[ ${3:-} == --light ]]; then
    home="$work/light"
    "$binary" keys add light --home "$home" --no-backup --keyring-backend test >/dev/null 2>&1
    addr=$("$binary" keys show light -a --home "$home" --keyring-backend test)
    "$binary" tx bank send validator "$addr" 100000000utia --home "$work/node" --keyring-backend test \
        --chain-id celestia-rs-local --node tcp://127.0.0.1:26657 --fees 20000utia | jq -e '.code == 0' >/dev/null
    for attempt in {1..60}; do
        if "$binary" query bank balances "$addr" --node tcp://127.0.0.1:26657 --home "$work/node" |
            jq -e '.balances[] | select(.denom == "utia" and .amount == "100000000")' >/dev/null; then break; fi
        if [[ "$attempt" == 60 ]]; then echo 'light account was not funded' >&2; exit 1; fi
        sleep 1
    done
    endpoint=http://127.0.0.1:39659
    DA_TEST_READ_TOKEN=$("$binary" da-auth read --home "$home")
    DA_TEST_WRITE_TOKEN=$("$binary" da-auth write --home "$home")
    "$binary" da-light --home "$home" --network celestia-rs-local \
        --peer /ip4/127.0.0.1/tcp/1/p2p/12D3KooWLiGKA7tFRoMSfcztssRgUzthfpJP1cAm16Fhztkn46rk \
        --core.ip 127.0.0.1 --core.port 9090 --keyring.keyname light --keyring.backend test \
        --rpc-listen 127.0.0.1:39659 >"$work/light.log" 2>&1 &
    light=$!
fi
provider_args=()
if [[ ${3:-} == --provider-loss ]]; then
    "$binary" init provider --chain-id celestia-rs-local --home "$work/provider" >/dev/null 2>&1
    keys=(--home "$work/node" --keyring-backend test)
    "$binary" keys add provider --no-backup "${keys[@]}" >/dev/null 2>&1
    account=$("$binary" keys show provider -a "${keys[@]}")
    tx_args=("${keys[@]}" --chain-id celestia-rs-local --node tcp://127.0.0.1:26657 --gas 500000 --fees 50000utia --wait)
    "$binary" tx bank send validator "$account" 420001000000utia "${tx_args[@]}" | jq -e '.code == 0' >/dev/null
    template=$(dirname "$source_file")/../../../crates/tier-5-node/celestia-rs/tests/fixtures/generate-only/validator.json
    jq --slurpfile key "$work/provider/config/priv_validator_key.json" \
        '.pubkey.key = $key[0].pub_key.value | .amount = "420000000000utia"' "$template" >"$work/validator.json"
    "$binary" tx staking create-validator "$work/validator.json" --from provider "${tx_args[@]}" |
        jq -e '.code == 0' >/dev/null
    provider_args+=("$work/provider")
fi
if ! "$work/go-provider" "$work/node" 127.0.0.1:9090 "$endpoint" "${provider_args[@]}" >"$work/provider.log" 2>&1; then
    grep -E '^panic:|^Go ' "$work/provider.log" >&2 || true
    exit 1
fi
grep '^Go ' "$work/provider.log"
