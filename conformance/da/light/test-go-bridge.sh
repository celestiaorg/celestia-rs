#!/usr/bin/env bash
# Reverse-direction interop. Run on an isolated host after the RPC test has exited.
set -euo pipefail
rust=$(realpath "${1:?path to a da-light build}")
app_src=$(realpath "${2:?pinned Go app checkout}")
node_src=$(realpath "${3:?pinned Go node checkout}")
[[ $(git -C "$app_src" rev-parse HEAD) == 58be8ae90d00518800df5506b3a1e988f009c1c6 ]]
case "$(git -C "$node_src" rev-parse HEAD)" in
    d5dfb21a0885622828e30e8e2daf50e5d1bb72b2|af9897bb471959660a4179048656694f3013b2f5) ;;
    *) exit 1 ;;
esac
work=$(mktemp -d)
app="$work/app"
da="$work/da"
consensus=
bridge=
cleanup() {
    for pid in "$bridge" "$consensus"; do
        if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" || true; fi
    done
    rm -rf -- "$work"
}
trap cleanup EXIT
(cd "$app_src" && go build -o "$app" ./cmd/celestia-appd)
(cd "$node_src" && go build -o "$da" ./cmd/celestia)
home_arg=(--home "$work/consensus")
key_arg=(--keyring-backend test "${home_arg[@]}")
chain=da-oracle
{
    "$app" init oracle --chain-id "$chain" "${home_arg[@]}"
    "$app" keys add validator "${key_arg[@]}"
    address=$("$app" keys show validator -a "${key_arg[@]}")
    "$app" genesis add-genesis-account "$address" 1000000000000000utia "${home_arg[@]}"
    "$app" genesis gentx validator 5000000000utia --chain-id "$chain" --fees 500utia \
        --commission-rate 0.05 --commission-max-rate 1 --commission-max-change-rate 1 "${key_arg[@]}"
    "$app" genesis collect-gentxs "${home_arg[@]}"
} >"$work/init.log" 2>&1

"$app" start "${home_arg[@]}" --p2p.laddr tcp://127.0.0.1:36656 --rpc.laddr tcp://127.0.0.1:36657 \
    --grpc.enable --grpc.address 127.0.0.1:39090 --grpc-web.enable=false >"$work/app.log" 2>&1 &
consensus=$!
rpc=http://127.0.0.1:36657
block() { curl -fsS --max-time 3 "$rpc/block?height=$1" 2>/dev/null; }
for attempt in {1..120}; do
    kill -0 "$consensus"
    if hash=$(block 1 | jq -er '.result.block_id.hash'); then break; fi
    sleep 1
done
export CELESTIA_CUSTOM="$chain:${hash:?no genesis block}"
"$da" bridge init --node.store "$work/bridge" --core.ip 127.0.0.1 --core.port 39090 >"$work/bridge-init.log" 2>&1
"$da" bridge start --node.store "$work/bridge" --core.ip 127.0.0.1 --core.port 39090 >"$work/bridge.log" 2>&1 &
bridge=$!
for attempt in {1..120}; do
    kill -0 "$bridge"
    peer=$(sed -n 's@.*/ip4/[0-9.]*\(/tcp/[0-9]*/p2p/[[:alnum:]]*\).*@/ip4/127.0.0.1\1@p' "$work/bridge.log" | head -1)
    if [[ -n "$peer" ]]; then break; fi
    sleep 1
done
: "${peer:?missing private Go bridge address}"
before=$(curl -fsS "$rpc/status" | jq -er '.result.sync_info.latest_block_height | tonumber')
"$app" tx blob pay-for-blob 0x00000000000000000103 0x68656c6c6f2d6461 \
    --from validator "${key_arg[@]}" --fees 50000utia --gas 500000 --chain-id "$chain" \
    --node tcp://127.0.0.1:36657 --output json --yes | jq -e '.code == 0' >/dev/null
height=$((before+1))
for attempt in {1..120}; do
    kill -0 "$consensus"
    if data=$(block "$height" | jq -ec '.result | select(.block != null)'); then
        if jq -e '.block.data.txs | length > 0' <<<"$data" >/dev/null; then break; fi
        height=$((height+1))
    fi
    sleep 1
done
hash=$(jq -er 'select(.block.data.txs | length > 0) | .block_id.hash' <<<"$data")
args=(da-light --home "$work/rust-light" --network "$chain" --peer "$peer" --trusted-hash "$hash")
# Give the bridge access to the block, then run each bound in a separate Rust process.
sleep 5
timeout 120 "$rust" "${args[@]}" --until "$height" | tee "$work/first.log"
grep -Fq "sampled height $height (16 verified shares)" "$work/first.log"
timeout 120 "$rust" "${args[@]}" --until "$((height+2))" | tee "$work/resume.log"
grep -Fq "sampled height $((height+2)) " "$work/resume.log"
if grep -Fq "sampled height $height " "$work/resume.log"; then exit 1; fi
