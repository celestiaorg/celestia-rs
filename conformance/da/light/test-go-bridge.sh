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
light=
follower=
link=
router=
core_proxy=
cleanup() {
    local status=$?
    for pid in "$link" "$follower" "$core_proxy" "$router" "$light" "$bridge" "$consensus"; do
        if [[ -n "$pid" ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" || true; fi
    done
    if (( status != 0 )); then
        # Initialization logs contain generated key material. Only report daemon errors.
        for log in "$work/app.log" "$work/bridge.log" "$work/follower.log" "$work/light.log" "$work/light-wallet.log"; do
            if [[ -f "$log" ]]; then grep -Ei '(^Error:|panicked at|panic:|disk space|address already in use)' "$log" | tail -10 || true; fi
        done
        df -h "$work"
    fi
    rm -rf -- "$work"
}
trap cleanup EXIT
(cd "$app_src" && go build -o "$app" ./cmd/celestia-appd)
(cd "$node_src" && go build -o "$da" ./cmd/celestia)
home_arg=(--home "$work/consensus")
key_arg=(--keyring-backend test "${home_arg[@]}")
chain=da-oracle
"$rust" keys add light --no-backup --keyring-backend test --home "$work/rust-light" >/dev/null 2>&1
"$rust" keys add da-secondary --no-backup --keyring-backend test --home "$work/rust-light" >/dev/null 2>&1
DA_TEST_SECOND_SIGNER=$("$rust" keys show da-secondary --address --keyring-backend test --home "$work/rust-light")
export DA_TEST_SECOND_SIGNER
light_wallet=$("$rust" keys show light -a --keyring-backend test --home "$work/rust-light")
{
    "$app" init oracle --chain-id "$chain" "${home_arg[@]}"
    "$app" keys add validator "${key_arg[@]}"
    address=$("$app" keys show validator -a "${key_arg[@]}")
    "$app" keys add second "${key_arg[@]}"
    second=$("$app" keys show second -a "${key_arg[@]}")
    "$app" genesis add-genesis-account "$second" 100000000utia "${home_arg[@]}"
    "$app" genesis add-genesis-account "$address" 1000000000000000utia "${home_arg[@]}"
    "$app" genesis add-genesis-account "$light_wallet" 12345000utia "${home_arg[@]}"
    "$app" genesis gentx validator 5000000000utia --chain-id "$chain" --fees 500utia \
        --commission-rate 0.05 --commission-max-rate 1 --commission-max-change-rate 1 "${key_arg[@]}"
    "$app" genesis collect-gentxs "${home_arg[@]}"
} >"$work/init.log" 2>&1
# The pinned bridge uses the app gRPC socket below. Keep unrelated Go consensus
# gRPC listeners from occupying the standard operator ports in the control run.
sed -i -E 's/^pex = .*/pex = false/; s/^external_address = .*/external_address = "127.0.0.1:38656"/' "$work/consensus/config/config.toml"
sed -i -E 's/^(priv_validator_grpc_laddr|grpc_laddr) = .*/\1 = ""/' "$work/consensus/config/config.toml"

"$app" start --force-no-bbr "${home_arg[@]}" --p2p.laddr tcp://127.0.0.1:36656 --rpc.laddr tcp://127.0.0.1:36657 \
    --grpc.enable --grpc.address 127.0.0.1:39090 --grpc-web.enable=false --minimum-gas-prices 0.002utia >"$work/app.log" 2>&1 &
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
# Run this private Go bridge alongside the forward-direction stock light fixture.
sed -i -E 's@/(tcp|udp)/2121([/",])@/\1/39221\2@g; s@/tcp/2122/@/tcp/39222/@g' "$work/bridge/config.toml"
start_bridge() {
    "$da" bridge start --node.store "$work/bridge" --core.ip 127.0.0.1 --core.port 39090 >"$work/bridge.log" 2>&1 &
    bridge=$!
}
start_bridge
for attempt in {1..120}; do
    kill -0 "$bridge"
    peer=$(sed -n 's@.*/ip4/[0-9.]*\(/tcp/[0-9]*/p2p/[[:alnum:]]*\).*@/ip4/127.0.0.1\1@p' "$work/bridge.log" | head -1)
    if [[ -n "$peer" ]]; then break; fi
    sleep 1
done
: "${peer:?missing private Go bridge address}"
before=$(curl -fsS "$rpc/status" | jq -er '.result.sync_info.latest_block_height | tonumber')
data_hex=$(python3 -c 'print("c0ffee" * 4000)')
"$app" tx blob pay-for-blob 0x00000000000000000103 "0x$data_hex" \
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
sample_fixture=$(realpath "$(dirname "$0")/sample_provider.go")
(cd "$node_src" && go build -o "$work/sample-provider" "$sample_fixture")
namespace_src=$(realpath "$(dirname "$0")/namespace_only.go")
(cd "$node_src" && go build -o "$work/namespace-probe" "$namespace_src")
"$(dirname "$0")/sample-restart.sh" "$rust" "$work/sample-provider" "$work/unavailable" "$chain" "$peer" "$hash" "$height" "$work/namespace-probe" "$data_hex"

if [[ ${4:-} == --sampling-recovery-only ]]; then exit 0; fi
# A new Rust consensus peer joins with its own identity and zero voting power.
# Copy only the public genesis from the validator.
follower_home="$work/follower"
"$rust" init observer --home "$follower_home" --chain-id "$chain" >"$work/follower-init.log" 2>&1
cp "$work/consensus/config/genesis.json" "$follower_home/config/genesis.json"
"$rust" config set config rpc.laddr tcp://127.0.0.1:37657 --home "$follower_home"
"$rust" config set app grpc.address 127.0.0.1:37990 --home "$follower_home"
"$rust" config set config rpc.grpc_laddr tcp://127.0.0.1:37998 --home "$follower_home"
validator_id=$("$app" comet show-node-id "${home_arg[@]}")
start_link() {
    rm -f "$work/link.ready"
    python3 "$(dirname "$0")/consensus_link.py" "$work/link.ready" >"$work/link.log" 2>&1 &
    link=$!
    for attempt in {1..50}; do
        kill -0 "$link"
        if [[ -e "$work/link.ready" ]]; then return; fi
        sleep 0.1
    done
    return 1
}
start_link
RUST_LOG=celestia_rs::bridge::peer=info "$rust" start --home "$follower_home" \
    --p2p.laddr 127.0.0.1:37656 --p2p.persistent_peers "$validator_id@127.0.0.1:38656" \
    --bridge-rpc-listen 127.0.0.1:37658 --bridge-p2p-listen 127.0.0.1:37221 \
    >"$work/follower.log" 2>&1 &
follower=$!
# Include an unreachable test identity to exercise discovery of the available bridge.
offline=/ip4/127.0.0.1/tcp/1/p2p/12D3KooWLiGKA7tFRoMSfcztssRgUzthfpJP1cAm16Fhztkn46rk
args=(da-light --home "$work/rust-light" --network "$chain" --peer "$offline" --peer "$peer" --trusted-hash "$hash")
# Give the bridge access to the block, then run each bound in a separate Rust process.
sleep 5
timeout 120 "$rust" "${args[@]}" --until "$height" | tee "$work/first.log"
grep -Fq "sampled height $height (16 verified shares)" "$work/first.log"
timeout 120 "$rust" "${args[@]}" --until "$((height+2))" | tee "$work/resume.log"
grep -Fq "sampled height $((height+2)) " "$work/resume.log"
if grep -Fq "sampled height $height " "$work/resume.log"; then exit 1; fi

# Independently anchor and sample through the Go bridge's UDP listener.
quic_peer="/ip4/127.0.0.1/udp/39221/quic-v1/p2p/${peer##*/}"
timeout 120 "$rust" da-light --home "$work/quic-light" --network "$chain" --peer "$quic_peer" \
    --trusted-hash "$hash" --until "$height" >"$work/quic-samples.log"
grep -Fq "sampled height $height (16 verified shares), hash $hash" "$work/quic-samples.log"
echo "Rust verified the signed paid header and 16 samples from the Go QUIC peer"

# A new store may bootstrap at a trusted Go peer's signed head without a fixed hash.
auto_home="$work/trusted-head"
timeout 90 "$rust" da-light --home "$auto_home" --network "$chain" --peer "$peer" --until 1 >"$work/trusted.log"
first=$(sed -n 's/^sampled height \([0-9]*\).*hash \([[:xdigit:]]*\)$/\1 \2/p' "$work/trusted.log")
read -r trusted_height trusted_hash <<<"$first"
[[ -n "$trusted_height" && $(block "$trusted_height" | jq -er '.result.block_id.hash') == "$trusted_hash" ]]
# Persisted trust can be reopened offline; a new explicit anchor must agree.
timeout 15 "$rust" da-light --home "$auto_home" --network "$chain" --peer "$offline" --until "$trusted_height"
wrong_hash=$(printf '%064d' 0)
if timeout 15 "$rust" da-light --home "$auto_home" --network "$chain" --peer "$offline" --trusted-hash "$wrong_hash" --until "$trusted_height" >"$work/wrong-anchor.log" 2>&1; then exit 1; fi
grep -Fq 'different checkpoint' "$work/wrong-anchor.log"
echo "Rust initialized at trusted Go peer height $trusted_height, retained its checkpoint offline, and rejected a different anchor"

# Disconnect the sole reachable provider before reopening the sampled store.
kill "$bridge"
wait "$bridge" || true
bridge=

DA_TEST_READ_TOKEN=$("$rust" da-auth read --home "$work/rust-light")
export DA_TEST_READ_TOKEN
endpoint=http://127.0.0.1:39658
balance_src=$(realpath "$(dirname "$0")/core_balance.go")
(cd "$node_src" && go build -o "$work/balance-probe" "$balance_src")
# Use a private test CA for the consensus gRPC proxy.
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj /CN=private-core-ca \
    -keyout "$work/ca.key" -out "$work/ca.pem" 2>/dev/null
openssl req -new -newkey rsa:2048 -nodes -subj /CN=localhost \
    -keyout "$work/core.key" -out "$work/core.csr" 2>/dev/null
printf '%s\n' 'subjectAltName=IP:127.0.0.1' 'extendedKeyUsage=serverAuth' >"$work/extensions"
openssl x509 -req -in "$work/core.csr" -CA "$work/ca.pem" -CAkey "$work/ca.key" \
    -CAcreateserial -days 1 -extfile "$work/extensions" -out "$work/core.pem" 2>/dev/null
CORE_TEST_TOKEN=$(openssl rand -hex 24)
printf '{"x-token":"%s"}\n' "$CORE_TEST_TOKEN" >"$work/xtoken.json"
CORE_TEST_TOKEN=$CORE_TEST_TOKEN "$work/balance-probe" proxy 127.0.0.1:39090 "$work/proxy-port" "$work/core.pem" "$work/core.key" &
core_proxy=$!
for attempt in {1..30}; do
    if [[ -s "$work/proxy-port" ]]; then break; fi
    kill -0 "$core_proxy"
    sleep 1
done
SSL_CERT_FILE="$work/ca.pem" RUST_LOG=celestia_rs::bridge::peer=info "$rust" "${args[@]}" --core.ip 127.0.0.1 --core.port "$(cat "$work/proxy-port")" --core.tls --core.xtoken.path "$work" --keyring.keyname light --keyring.keyname da-secondary --keyring.backend test --p2p-listen /ip4/127.0.0.1/tcp/39621 --p2p-listen /ip4/127.0.0.1/udp/39621/quic-v1 --rpc-listen 127.0.0.1:39658 >"$work/light.log" 2>&1 &
light=$!
for attempt in {1..60}; do
    kill -0 "$light"
    if curl -fsS --max-time 2 "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.LocalHead","params":[]}' 2>/dev/null |
        jq -e '.result.header.height != null' >/dev/null; then break; fi
    if [[ "$attempt" == 60 ]]; then echo 'light RPC unavailable' >&2; exit 1; fi
    sleep 1
done
expected=$("$app" query bank balances "$address" "${home_arg[@]}" --height "$((height+1))" --node tcp://127.0.0.1:36657 --output json | jq -er '.balances[] | select(.denom == "utia") | .amount')
"$work/balance-probe" "$endpoint" "$address" "$expected" "$light_wallet"
probe=$(realpath "$(dirname "$0")/read.go")
(cd "$node_src" && go run "$probe" "$endpoint" "$height" "$hash")
light_peer=$(sed -n 's/.*DA light peer listening.*peer=\([[:alnum:]]*\).*/\1/p' "$work/light.log" | head -1)
: "${light_peer:?missing light peer identity}"
exchange_probe=$(realpath "$(dirname "$0")/read_exchange.go")
(cd "$node_src" && go run "$exchange_probe" "$chain" "/ip4/127.0.0.1/tcp/39621/p2p/$light_peer" "$height" "$hash")
(cd "$node_src" && go run "$exchange_probe" "$chain" "/ip4/127.0.0.1/udp/39621/quic-v1/p2p/$light_peer" "$height" "$hash")
tuple_probe=$(realpath "$(dirname "$0")/../subscriptions/check.go")
(cd "$node_src" && go run "$tuple_probe" "$endpoint" blobstream "$height" "$((height+3))")
auth_probe=$(realpath "$(dirname "$0")/../auth/check.go")
DA_TEST_ADMIN_TOKEN=$("$rust" da-auth admin --home "$work/rust-light")
export DA_TEST_ADMIN_TOKEN
(cd "$node_src" && DA_TEST_LOG="$work/light.log" go run "$auth_probe" "$endpoint" Light)
admin_probe=$(realpath "$(dirname "$0")/../peer/read_peer.go")
(cd "$node_src" && DA_TEST_ADMIN_URL="$endpoint" go run "$admin_probe" "$chain" "/ip4/127.0.0.1/udp/39621/quic-v1/p2p/$light_peer" --light-admin)
unset DA_TEST_ADMIN_TOKEN
curl -fsS "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"share.GetEDS\",\"params\":[$height]}" |
    jq -e '.error.message | contains("no connected DA peer")' >/dev/null
# The same Rust process must reconnect and authenticate a new header once its provider returns.
start_bridge
for attempt in {1..90}; do
    kill -0 "$bridge"
    kill -0 "$light"
    if curl -fsS --max-time 2 "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"header.LocalHead","params":[]}' |
        jq -e --argjson old "$((height+2))" '(.result.header.height | tonumber) > $old' >/dev/null; then
        echo 'Rust resumed sampling after the private Go provider restarted'
        break
    fi
    if [[ "$attempt" == 90 ]]; then echo 'light node did not reconnect' >&2; exit 1; fi
    sleep 1
done
(cd "$node_src" && go run "$probe" "$endpoint" "$height" "$hash" online)
data_probe=$(realpath "$(dirname "$0")/../read_bridge.go")
(cd "$node_src" && go run "$data_probe" "$endpoint" "$height" 0103 "$data_hex")
light_read_token=$DA_TEST_READ_TOKEN
# Compare the non-validator's committed history directly with the sole validator.
kill -0 "$follower"
follower_rpc=http://127.0.0.1:37657
for attempt in {1..120}; do
    kill -0 "$follower"
    if curl -fsS --max-time 2 "$follower_rpc/block?height=$((height+1))" 2>/dev/null | jq -e '.result.block' >/dev/null; then break; fi
    if [[ "$attempt" == 120 ]]; then echo 'Rust observer did not sync the paid block' >&2; exit 1; fi
    sleep 1
done
curl -fsS "$follower_rpc/status" | jq -e '(.result.validator_info.voting_power | tonumber) == 0' >/dev/null
for h in "$height" "$((height+1))"; do
    expected=$(block "$h" | jq -cer '[.result.block_id.hash,.result.block.header.app_hash]')
    observed=$(curl -fsS "$follower_rpc/block?height=$h" | jq -cer '[.result.block_id.hash,.result.block.header.app_hash]')
    [[ "$observed" == "$expected" ]]
done
DA_TEST_READ_TOKEN=$("$rust" da-auth read --home "$follower_home")
(cd "$node_src" && go run "$data_probe" http://127.0.0.1:37658 "$height" 0103 "$data_hex")
follower_id=$(sed -n 's/.*DA peer listening.*peer=\([[:alnum:]]*\).*/\1/p' "$work/follower.log" | head -1)
: "${follower_id:?missing Rust observer DA identity}"
peer_probe=$(realpath "$(dirname "$0")/../peer/read_peer.go")
(cd "$node_src" && go run "$peer_probe" "$chain" "/ip4/127.0.0.1/tcp/37221/p2p/$follower_id" "$height" 0103 "$data_hex")
echo "Go validator and non-validating Rust bridge matched block and app hashes at $height and $((height+1))"
if [[ -r /proc/self/stat ]]; then
    (cd "$node_src" && go build -o "$work/read-bridge" "$data_probe")
    GO_READ_TOKEN=$("$da" bridge auth read --node.store "$work/bridge" | tail -1)
    export GO_READ_TOKEN
    python3 "$(dirname "$0")/measure_observer.py" "$work/read-bridge" "$height" "$data_hex" \
        "$work" "$consensus" "$bridge" "$follower"
fi

# Break only the observer's consensus link while both Rust roles and the Go validator run.
kill "$link"
wait "$link" || true
link=
consensus_height() { curl -fsS --max-time 3 "$1/status" | jq -er '.result.sync_info.latest_block_height | tonumber'; }
disconnected_from=$(consensus_height "$rpc")
for attempt in {1..90}; do
    validator_head=$(consensus_height "$rpc")
    observer_head=$(consensus_height "$follower_rpc")
    if (( validator_head >= disconnected_from + 3 && validator_head - observer_head >= 2 )); then break; fi
    if [[ "$attempt" == 90 ]]; then echo 'observer did not fall behind during the partition' >&2; exit 1; fi
    sleep 1
done
start_link
for attempt in {1..120}; do
    kill -0 "$follower"
    if [[ $(consensus_height "$follower_rpc") -ge "$validator_head" ]]; then break; fi
    if [[ "$attempt" == 120 ]]; then echo 'observer failed to recover from the partition' >&2; exit 1; fi
    sleep 1
done
expected=$(block "$validator_head" | jq -er '.result.block_id.hash')
observed=$(curl -fsS "$follower_rpc/block?height=$validator_head" | jq -er '.result.block_id.hash')
[[ "$observed" == "$expected" ]]
(cd "$node_src" && go run "$peer_probe" "$chain" "/ip4/127.0.0.1/tcp/37221/p2p/$follower_id" "$height" 0103 "$data_hex")
echo "Rust observer rejoined the Go validator at height $validator_head after falling behind at $observer_head"
DA_TEST_READ_TOKEN=$light_read_token


ws_endpoint=${endpoint/http:/ws:}
(cd "$node_src" && go run "$tuple_probe" "$ws_endpoint" "$height" 0103 "$data_hex" light)

# The only configured peer in this fresh home has no headers or SHREX handler.
# The nonempty checkpoint and its proofs must come from the discovered provider.
router_src=$(realpath "$(dirname "$0")/provider_router.go")
(cd "$node_src" && go build -o "$work/provider-router" "$router_src")
"$work/provider-router" "$chain" "$peer" >"$work/router.addr" 2>"$work/router.log" &
router=$!
for attempt in {1..100}; do
    kill -0 "$router"
    if [[ -s "$work/router.addr" ]]; then break; fi
    sleep 0.1
done
seed=$(cat "$work/router.addr")
[[ "$seed" == */p2p/* ]]
timeout 120 "$rust" da-light --home "$work/discovered" --network "$chain" --peer "$seed" \
    --trusted-hash "$hash" --until "$height" >"$work/discovered.log"
grep -Fq "sampled height $height (16 verified shares), hash $hash" "$work/discovered.log"
echo "Rust verified the paid checkpoint from a Go provider learned only through a routing peer"

# Reopen the funded light account against the actual Go core transaction services.
kill -INT "$light"
wait "$light"
light=
"$rust" "${args[@]}" --core.ip 127.0.0.1 --core.port 39090 --keyring.keyname light --keyring.keyname da-secondary --keyring.backend test \
    --rpc-listen 127.0.0.1:39658 >"$work/light-wallet.log" 2>&1 &
light=$!
for attempt in {1..60}; do
    kill -0 "$light"
    if curl -fsS --max-time 2 "$endpoint" -H "Authorization: Bearer $DA_TEST_READ_TOKEN" \
        -d '{"jsonrpc":"2.0","id":1,"method":"state.AccountAddress","params":[]}' 2>/dev/null |
        jq -e --arg wallet "$light_wallet" '.result == $wallet' >/dev/null; then break; fi
    if [[ "$attempt" == 60 ]]; then echo 'light wallet unavailable' >&2; exit 1; fi
    sleep 1
done
DA_TEST_WRITE_TOKEN=$("$rust" da-auth write --home "$work/rust-light")
export DA_TEST_WRITE_TOKEN
submit_probe=$(realpath "$(dirname "$0")/../submit/check.go")
(cd "$node_src" && go run "$submit_probe" "$endpoint")

validator_json=$(realpath "$(dirname "$0")/../../../crates/tier-5-node/celestia-rs/tests/fixtures/generate-only/validator.json")
tx=$("$app" tx staking create-validator "$validator_json" --from second "${key_arg[@]}" --chain-id "$chain" \
    --node tcp://127.0.0.1:36657 --fees 50000utia --gas 500000 --output json --yes)
jq -e '.code == 0' <<<"$tx" >/dev/null
src=$("$app" keys show validator --bech val -a "${key_arg[@]}")
dst=$("$app" keys show second --bech val -a "${key_arg[@]}")
for attempt in {1..45}; do
    if "$app" query staking validator "$dst" "${home_arg[@]}" --node tcp://127.0.0.1:36657 --output json >/dev/null 2>&1; then break; fi
    if [[ "$attempt" == 45 ]]; then echo 'second validator was not registered' >&2; exit 1; fi
    sleep 1
done
staking_probe=$(realpath "$(dirname "$0")/../staking/check.go")
escrow_probe=$(realpath "$(dirname "$0")/../fibre/escrow.go")
(cd "$node_src" && go run "$staking_probe" "$endpoint" "$dst" "$src")
(cd "$node_src" && go run "$escrow_probe" "$endpoint")
