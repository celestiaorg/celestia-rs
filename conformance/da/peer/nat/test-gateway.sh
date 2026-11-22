#!/usr/bin/env bash
# Run inside a fresh network namespace. There is no route to a real gateway.
set -euo pipefail
rust=$(realpath "${1:?expected Rust DA binary}")
oracle=$(realpath "${2:?expected Go probe}")
igd_root=$(realpath "${3:?expected extracted miniupnpd-iptables package}")
[[ $(ip -o link show | wc -l) -eq 1 && -z $(ip route show default) ]]
work=$(mktemp -d)
unshare --net sleep infinity & router=$!
unshare --net sleep infinity & external=$!
node= gateway= bootstrap=
cleanup() {
    code=$?
    if ((code)); then tail -30 "$work/gateway.log" "$work/node.log" 2>/dev/null || true; fi
    for pid in "$node" "$gateway" "$bootstrap" "$external" "$router"; do
        if [[ -n $pid ]]; then kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; fi
    done
    rm -rf -- "$work"
}
trap cleanup EXIT
gw=(nsenter -t "$router" -n)
wan=(nsenter -t "$external" -n)
for p in "$router" "$external"; do
    for _ in {1..100}; do
        [[ $(readlink "/proc/$p/ns/net") != "$(readlink /proc/self/ns/net)" ]] && break
        sleep 0.02
    done
    [[ $(readlink "/proc/$p/ns/net") != "$(readlink /proc/self/ns/net)" ]]
done
ip link set lo up
ip link add inside type veth peer name lan0 netns "$router"
ip addr add 192.168.55.2/24 dev inside
ip link set inside up
ip route add default via 192.168.55.1 dev inside
"${gw[@]}" ip link set lo up
"${gw[@]}" ip addr add 192.168.55.1/24 dev lan0
"${gw[@]}" ip link set lan0 up
"${gw[@]}" ip link add wan0 type veth peer name outside netns "$external"
"${gw[@]}" ip addr add 11.0.0.2/24 dev wan0
"${gw[@]}" ip link set wan0 up
"${gw[@]}" sysctl -qw net.ipv4.ip_forward=1
"${wan[@]}" ip link set lo up
"${wan[@]}" ip addr add 11.0.0.1/24 dev outside
"${wan[@]}" ip link set outside up
# No route from the outside host to the private subnet. Only DNAT can reach Rust.
for table in nat filter mangle; do "${gw[@]}" iptables-legacy -t "$table" -N MINIUPNPD; done
"${gw[@]}" iptables-legacy -t nat -N MINIUPNPD-POSTROUTING
"${gw[@]}" iptables-legacy -t nat -A PREROUTING -i wan0 -j MINIUPNPD
"${gw[@]}" iptables-legacy -t nat -A POSTROUTING -o wan0 -j MINIUPNPD-POSTROUTING
"${gw[@]}" iptables-legacy -A FORWARD -i wan0 -j MINIUPNPD
cat >"$work/igd.conf" <<CONF
ext_ifname=wan0
ext_ip=11.0.0.2
listening_ip=lan0
enable_upnp=yes
enable_natpmp=no
secure_mode=yes
allow 32121-32121 192.168.55.2/32 32121-32121
deny 0-65535 0.0.0.0/0 0-65535
CONF
"${gw[@]}" env LD_LIBRARY_PATH="$igd_root/usr/lib/x86_64-linux-gnu:$igd_root/lib/x86_64-linux-gnu" "$igd_root/usr/sbin/miniupnpd" -d -f "$work/igd.conf" -P "$work/igd.pid" >"$work/gateway.log" 2>&1 & gateway=$!
"${wan[@]}" "$oracle" >"$work/seed" & bootstrap=$!
for _ in {1..100}; do [[ -s "$work/seed" ]] && break; sleep 0.1; done
seed=$(cat "$work/seed")
[[ $seed == /ip4/11.0.0.1/tcp/31221/p2p/* ]]
for role in bridge light; do
    if [[ $role == bridge ]]; then
        args=(start --network local --bridge-bootstrap "$seed" --bridge-p2p-listen 192.168.55.2:32121 --bridge-p2p-listen /ip4/192.168.55.2/udp/32121/quic-v1 --bridge-rpc-listen 127.0.0.1:32658)
    else
        args=(da-light --network private-nat --peer "$seed" --trusted-hash "$(printf '%064d' 1)" --p2p-listen /ip4/192.168.55.2/tcp/32121 --p2p-listen /ip4/192.168.55.2/udp/32121/quic-v1 --rpc-listen 127.0.0.1:32658)
    fi
    "$rust" "${args[@]}" --home "$work/$role" >"$work/node.log" 2>&1 & node=$!
    for _ in {1..120}; do
        kill -0 "$node" "$gateway"
        curl -s --max-time 1 http://127.0.0.1:32658/ >/dev/null && break
        sleep 0.5
    done
    DA_TEST_ADMIN_TOKEN=$("$rust" da-auth admin --home "$work/$role") "$oracle" mapped >"$work/mapped"
    while read -r addr; do "${wan[@]}" "$oracle" ping-mapped "$addr"; done <"$work/mapped"
    echo "Go reached the Rust $role through the IGD on TCP and QUIC"
    kill "$node"; wait "$node" || true; node=
done
