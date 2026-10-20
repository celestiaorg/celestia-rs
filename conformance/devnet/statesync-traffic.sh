#!/usr/bin/env bash
# State sync under CheckTx traffic.
#
# A Go validator V0 and a Rust full node F1 take snapshots every 10 heights.
# Fresh nodes then state-sync while txs are broadcast to them (CheckTx runs
# on the not-yet-restored app): G2 (Go) from V0's snapshots, G3 (Go) from
# F1's (Rust) snapshots, R4 (Rust) from both. Each must then keep up with
# the chain.
#
# Go celestia-appd v10.4.0 does not: the multistore's inter-block cache
# (`inter-block-cache = true`, the default) keeps the values CheckTx read
# from the empty stores before the restore (nil for the consensus params,
# accounts, ...), and the snapshot restore reloads the stores without
# resetting it (cosmos-sdk store v1.1.3-celestia.3, rootmulti
# loadCommitStoreFromParams -> interBlockCache.GetStoreCache), so the first
# block after the snapshot fails with `collections: not found: key 'no_key'
# of type ConsensusParams` and the node panics (it recovers on restart).
# This happens with Go and with Rust snapshots alike (they are byte-for-byte
# the same stream). The Rust node has no such cache and must be unaffected.
#
# usage: [DEVNET_PORT_OFFSET=19000] [DEVNET_BIN=dir] [CELESTIA_RS=path] [APP_V9=1]
#        conformance/devnet/statesync-traffic.sh
# Exit 0 iff the Rust node synced and followed the chain; the Go nodes'
# outcome is reported.
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
# APP_V9=1: the Go nodes run celestia-app v9.0.8 ($DEVNET_BIN/v9), so the
# chain and every snapshot are at app version 9.
[ -n "${APP_V9:-}" ] && export PATH=$BIN/v9:$PATH
OFF=${DEVNET_PORT_OFFSET:-19000}
if [ -z "${CELESTIA_RS:-}" ]; then
  (cd "$ROOT" && cargo build -q --release -p celestia-rs)
  CELESTIA_RS=$ROOT/target/release/celestia-rs
fi
RS=$CELESTIA_RS
W=${SYNC_DIR:-$(mktemp -d)}
FAIL=()
echo "dir: $W"
# shellcheck source=lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"

make_net "$W" sync-traffic-1 "go rs go go rs" "5000000000 0 0 0 0"
for i in 0 1; do
  sed -i -e 's#^snapshot-interval = .*#snapshot-interval = 10#' -e 's#^snapshot-keep-recent = .*#snapshot-keep-recent = 10#' \
    "$W/n$i/config/app.toml"
done
peer() { echo "$(cat "$W/n$1/node_id")@127.0.0.1:$((26656 + OFF + 10 * $1))"; }
start_node "$W" 0
start_node "$W" 1
ensure_up "$W" 0 1
wait_height "$W" 0 25 300 || { echo "chain did not reach height 25"; exit 1; }
TH=$(($(height "$W" 0) - 2))
THASH=$(curl -s "$(rpc "$W" 0)/block?height=$TH" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block_id"]["hash"])')
RPCS="127.0.0.1:$((26657 + OFF)),127.0.0.1:$((26657 + OFF + 10))"
go_statesync() { # <node> <peer node>
  python3 - "$W/n$1/config/config.toml" "$(peer "$2")" "$RPCS" "$TH" "$THASH" <<'PY'
import re, sys
p, peers, rpcs, th, thash = sys.argv[1:]
s = open(p).read()
s = re.sub(r'(?m)^persistent_peers = .*$', f'persistent_peers = "{peers}"', s)
head, sep, rest = s.partition("[statesync]")
body, nxt, tail = rest.partition("\n[")
for k, v in (("enable", "true"), ("rpc_servers", f'"{rpcs}"'), ("trust_height", th),
             ("trust_hash", f'"{thash}"'), ("discovery_time", '"5s"')):
    body = re.sub(rf'(?m)^{k} = .*$', f'{k} = {v}', body, count=1)
open(p, "w").write(head + sep + body + nxt + tail)
PY
}
go_statesync 2 0
go_statesync 3 1
echo "$(peer 0),$(peer 1)" > "$W/n4/peers"
start_node "$W" 2
start_node "$W" 3
RS_EXTRA="--statesync-rpc-servers $RPCS --statesync-trust-height $TH --statesync-trust-hash $THASH --statesync-discovery-secs 5" \
  start_node "$W" 4
# CheckTx traffic to the syncing nodes (a MsgSend of an unknown account:
# rejected, but the ante handler reads params and accounts first).
python3 - "$ROOT/conformance/devnet/soak" "$(rpc "$W" 2)" "$(rpc "$W" 3)" "$(rpc "$W" 4)" <<'PY' &
import sys, time
sys.path.insert(0, sys.argv[1])
import cosmostx as c
k = c.Key.from_seed("statesync-traffic")
tx = c.sign_tx(k, "sync-traffic-1", 0, 0, [c.msg_send(k.address, k.address, 1)], 100000, 2000)
end = time.time() + 60
while time.time() < end:
    for url in sys.argv[2:]:
        try:
            c.broadcast(url, tx, timeout=2)
        except Exception:
            pass
    time.sleep(0.3)
PY
TRAFFIC=$!
wait "$TRAFFIC" || true
sleep 20
TOP=$(height "$W" 0)
rc=0
for i in 2 3 4; do
  h=$(curl -s -m 3 "$(rpc "$W" "$i")/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p' || true)
  restored=$(sed 's/\x1b\[[0-9;]*m//g' "$W/n$i/node.log" | grep -c -i 'snapshot restored\|state sync complete\|restored snapshot' || true)
  panics=$(grep -c 'panic' "$W/n$i/node.log" || true)
  src=$([ "$i" = 2 ] && echo "Go snapshots" || { [ "$i" = 3 ] && echo "Rust snapshots" || echo "Go + Rust snapshots"; })
  echo "n$i ($(cat "$W/n$i/kind"), $src): height ${h:-down} of $TOP, restored: $restored, panics: $panics"
  if [ "$i" = 4 ] && { [ -z "$h" ] || [ "$h" -lt $((TOP - 3)) ] || [ "$panics" != 0 ]; }; then rc=1; fi
done
cleanup
[ "$rc" = 0 ] && echo "OK: the Rust node state-synced under CheckTx traffic" || echo "FAILED: the Rust node did not sync / follow"
exit "$rc"
