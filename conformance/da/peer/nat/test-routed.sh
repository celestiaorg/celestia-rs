#!/usr/bin/env bash
# Run inside a fresh Linux network namespace with only loopback and no external route.
set -euo pipefail
rust=$(realpath "${1:?expected da-light enabled celestia-rs}")
oracle=$(realpath "${2:?expected Go AutoNAT probe}")
[[ $(ip -o link show | wc -l) -eq 1 && -z $(ip route show default) ]]
ip link set lo up
# Go and Rust see distinct routable addresses. Both terminate on this private loopback.
ip address add 11.0.0.2/32 dev lo
ip address add 11.0.0.1/32 dev lo
ip route replace local 11.0.0.1/32 dev lo table local src 11.0.0.2
work=$(mktemp -d)
node=
peer=
stop() {
    if [[ -n "$node" ]]; then kill -TERM "$node" 2>/dev/null || true; wait "$node" || true; node=; fi
    if [[ -n "$peer" ]]; then kill -TERM "$peer" 2>/dev/null || true; wait "$peer" || true; peer=; fi
}
cleanup() { stop; rm -rf -- "$work"; }
trap cleanup EXIT
for reach in public private; do
    if [[ $reach == private ]]; then
        iptables -I INPUT -i lo -d 11.0.0.2/32 -p tcp --syn -j REJECT --reject-with tcp-reset
    fi
    for role in bridge light; do
        : > "$work/addr"
        "$oracle" >"$work/addr" 2>"$work/go.log" &
        peer=$!
        for _ in {1..100}; do
            kill -0 "$peer"
            if [[ -s "$work/addr" ]]; then break; fi
            sleep 0.1
        done
        addr=$(cat "$work/addr")
        [[ $addr == /ip4/11.0.0.1/tcp/31221/p2p/* ]]
        home="$work/$role-$reach"
        if [[ $role == light ]]; then
            args=(da-light --network private-nat --peer "$addr" --trusted-hash "$(printf '%064d' 1)" --p2p-listen /ip4/11.0.0.2/tcp/32121 --rpc-listen 127.0.0.1:32658)
        else
            args=(start --network local --bridge-bootstrap "$addr" --bridge-p2p-listen 11.0.0.2:32121 --bridge-rpc-listen 127.0.0.1:32658)
        fi
        "$rust" "${args[@]}" --home "$home" >"$work/rust.log" 2>&1 &
        node=$!
        for _ in {1..120}; do
            kill -0 "$node"
            if curl -s --max-time 1 http://127.0.0.1:32658/ >/dev/null; then break; fi
            sleep 0.5
        done
        DA_TEST_ADMIN_TOKEN=$("$rust" da-auth admin --home "$home") "$oracle" "$reach"
        stop
    done
done
