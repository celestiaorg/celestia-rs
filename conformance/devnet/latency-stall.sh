#!/usr/bin/env bash
# A validator behind high link latency, then healed: does it keep up?
#
# Four validators (kinds from $LAT_KINDS, default all Go) with every P2P
# link through soak/netproxy.py. Node 3's links get $LAT_MS of one-way
# latency for $LAT_SECS, then the latency is removed and node 3 must reach
# the chain's height within $WAIT_S. Repeated $ATTEMPTS times.
#
# Background: in the long soak a Rust validator under 700 ms of latency
# stopped at a height in the commit step without the committed block's
# parts, until a reconnect. The propagation reactor's per-peer requests
# are only reset when the node commits (Prune) or the peer is removed, so a
# want that reached its peers after they had committed (normal wants are
# only answered for the current height/round) is never retried: retryWants
# subtracts the outstanding requests from every peer. celestia-core v0.42.3
# has the same logic; this script checks whether a Go node gets stuck the
# same way.
#
# usage: [LAT_KINDS="go go go go"] [LAT_MS=1500] [LAT_SECS=60] [WAIT_S=90] [ATTEMPTS=5]
#        [DEVNET_PORT_OFFSET=19000] [DEVNET_BIN=dir] [CELESTIA_RS=path]
#        conformance/devnet/latency-stall.sh
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-19000}
KINDS=${LAT_KINDS:-go go go go}
if [[ " $KINDS " == *" rs "* ]] && [ -z "${CELESTIA_RS:-}" ]; then
  (cd "$ROOT" && cargo build -q --release -p celestia-rs)
  CELESTIA_RS=$ROOT/target/release/celestia-rs
fi
RS=${CELESTIA_RS:-}
W=${LAT_DIR:-$(mktemp -d)}
FAIL=()
echo "dir: $W"
# shellcheck source=lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"

read -r -a K <<<"$KINDS"
N=${#K[@]}
STAKES=$(for _ in "${K[@]}"; do printf '5000000000 '; done)
make_net "$W" lat-1 "$KINDS" "$STAKES"
# Every link through the proxy (soak.py's port layout).
python3 - "$W" "$OFF" "$N" <<'PY'
import json, re, sys
w, off, n = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
ids = [open(f"{w}/n{i}/node_id").read().strip() for i in range(n)]
proxy = lambda i, j: 26656 + off + 500 + 10 * i + j
routes = [{"listen": proxy(i, j), "target": 26656 + off + 10 * j, "src": i, "dst": j}
          for i in range(n) for j in range(n) if i != j]
json.dump(routes, open(f"{w}/routes.json", "w"))
json.dump({"cut": [], "delay_ms": {}}, open(f"{w}/control.json", "w"))
for i in range(n):
    peers = ",".join(f"{ids[j]}@127.0.0.1:{proxy(i, j)}" for j in range(n) if j != i)
    open(f"{w}/n{i}/peers", "w").write(peers)
    p = f"{w}/n{i}/config/config.toml"
    s = open(p).read()
    s = re.sub(r'(?m)^persistent_peers = .*$', f'persistent_peers = "{peers}"', s)
    open(p, "w").write(s)
PY
python3 "$ROOT/conformance/devnet/soak/netproxy.py" "$W/routes.json" "$W/control.json" > "$W/proxy.log" 2>&1 &
PIDS+=($!)
sleep 1
for i in $(seq 0 $((N - 1))); do start_node "$W" "$i"; done
ensure_up "$W" $(seq 0 $((N - 1)))
wait_height "$W" 0 5 300 || { echo "network did not start"; exit 1; }
control() { # <json>
  echo "$1" > "$W/control.json.tmp" && mv "$W/control.json.tmp" "$W/control.json"
}
h() { curl -s -m 3 "$(rpc "$W" "$1")/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p' || true; }
stuck=0
for a in $(seq 1 "${ATTEMPTS:-5}"); do
  d=$(python3 -c "import json; print(json.dumps({'cut': [], 'delay_ms': {f'3-{j}': ${LAT_MS:-1500} for j in range($N) if j != 3}}))")
  control "$d"
  sleep "${LAT_SECS:-60}"
  control '{"cut": [], "delay_ms": {}}'
  top=$(h 0)
  ok=0
  for _ in $(seq 1 "${WAIT_S:-90}"); do
    x=$(h 3)
    [ "${x:-0}" -ge "$top" ] 2>/dev/null && { ok=1; break; }
    sleep 1
  done
  x=$(h 3)
  if [ "$ok" = 1 ]; then
    echo "attempt $a: node 3 ($(cat "$W/n3/kind")) caught up (height $x)"
  else
    echo "attempt $a: node 3 ($(cat "$W/n3/kind")) STUCK at height ${x:-?} (chain at $(h 0)) ${WAIT_S:-90} s after the latency ended"
    curl -s -m 3 "$(rpc "$W" 3)/consensus_state" | head -c 300; echo
    stuck=$((stuck + 1))
    break
  fi
done
cleanup
echo "stuck: $stuck"
