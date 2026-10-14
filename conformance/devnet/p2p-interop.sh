#!/usr/bin/env bash
# P2P interop: Rust `p2p` crate nodes peering with a Go celestia-core v0.42.3
# validator (celestia-appd v10.4.0).
#
#   - Rust node A dials the Go node (outbound SecretConnection + NodeInfo
#     handshake, PEX request/response, consensus gossip received);
#   - the Go node dials Rust node B, configured as its persistent peer
#     (inbound path);
#   - both stay connected for DURATION seconds with 5s pings from Rust and
#     Go's own 60s pings, decoding every consensus / propagation message;
#   - the Go node's /net_info must list both Rust nodes in the right
#     direction, and its log must not show it dropping them.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/p2p-interop.sh [duration]
# Exit 0 iff all checks pass.
set -euo pipefail
DURATION=${1:-75}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-3000}
CHAIN_ID=rs-p2p-1
W=$(mktemp -d)
V=$W/val
RUST="$ROOT/target/debug/examples/p2p_interop"

(cd "$ROOT" && cargo build -q -p p2p --example p2p_interop)

A="celestia-appd --home $V"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A genesis add-genesis-account "$($A keys show val0 -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1

shift_ports() { # file delta
  python3 - "$1" "$2" <<'PY'
import re, sys
p, d = sys.argv[1], int(sys.argv[2])
s = open(p).read()
s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + d), s)
open(p, "w").write(s)
PY
}
shift_ports "$V/config/config.toml" "$OFF"
shift_ports "$V/config/app.toml" "$OFF"
GO_P2P=$((26656 + OFF)); GO_RPC=$((26657 + OFF))
RA_PORT=$((26756 + OFF)); RB_PORT=$((26757 + OFF))

# Rust node keys (B must be known to the Go node before it starts).
RA_ID=$("$RUST" --home "$W/ra" --print-id)
RB_ID=$("$RUST" --home "$W/rb" --print-id)
GO_ID=$(celestia-appd comet show-node-id --home "$V" 2>/dev/null || cometbft show_node_id --home "$V")
echo "go=$GO_ID rust-a=$RA_ID (dials go) rust-b=$RB_ID (dialed by go)"

sed -i -e 's#^addr_book_strict = true#addr_book_strict = false#' \
       -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' \
       -e "s#^persistent_peers = .*#persistent_peers = \"$RB_ID@127.0.0.1:$RB_PORT\"#" "$V/config/config.toml"

PIDS=(); cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }; trap cleanup EXIT
celestia-appd start --home "$V" --log_level "${GO_LOG_LEVEL:-info}" > "$W/go.log" 2>&1 & PIDS+=($!)

h() { curl -s "localhost:$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
for _ in $(seq 1 120); do [ "$(h $GO_RPC || echo 0)" -ge 2 ] 2>/dev/null && break; sleep 1; done
echo "go validator at height $(h $GO_RPC)"

"$RUST" --home "$W/rb" --chain-id $CHAIN_ID --listen "127.0.0.1:$RB_PORT" --expect-peer "$GO_ID" \
  --duration "$DURATION" > "$W/rb.json" 2> "$W/rb.log" & RB=$!; PIDS+=($RB)
"$RUST" --home "$W/ra" --chain-id $CHAIN_ID --listen "127.0.0.1:$RA_PORT" --expect-peer "$GO_ID" \
  --dial "$GO_ID@127.0.0.1:$GO_P2P" --duration "$DURATION" > "$W/ra.json" 2> "$W/ra.log" & RA=$!; PIDS+=($RA)

# Midway, the Go node must see A as inbound and B as outbound.
sleep $((DURATION / 2))
curl -s "localhost:$GO_RPC/net_info" > "$W/net_info.json"
NETINFO=$(python3 - "$W/net_info.json" "$RA_ID" "$RB_ID" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))["result"]
peers = {p["node_info"]["id"]: p for p in d["peers"]}
ok = sys.argv[2] in peers and sys.argv[3] in peers \
    and not peers[sys.argv[2]]["is_outbound"] and peers[sys.argv[3]]["is_outbound"]
print("ok" if ok else "bad", d["n_peers"], {k: (v["is_outbound"], v["node_info"]["moniker"]) for k, v in peers.items()})
PY
)
echo "go /net_info: $NETINFO"

RC=0
wait $RA || RC=1
wait $RB || RC=1
echo "rust-a: $(cat "$W/ra.json")"
echo "rust-b: $(cat "$W/rb.json")"
case "$NETINFO" in ok*) ;; *) echo "FAILED: Go node does not list both Rust peers correctly"; RC=1 ;; esac
# Go may only log the EOF when the Rust processes exit; any other reason
# for dropping a Rust peer (bad message, PEX abuse, filter) fails the run.
# Mid-run EOFs are caught on the Rust side (exactly one connection).
sed 's/\x1b\[[0-9;]*m//g' "$W/go.log" > "$W/go.clean.log"
BAD=$(grep -E "Stopping peer for error|rejected|filtered" "$W/go.clean.log" | grep -E "$RA_ID|$RB_ID" | grep -v "err=EOF" || true)
if [ -n "$BAD" ]; then
  echo "FAILED: Go node dropped a Rust peer:"; echo "$BAD" | head -5
  RC=1
fi
if [ $RC -eq 0 ]; then
  echo "p2p interop OK (go height $(h $GO_RPC))"
else
  echo "logs: $W"; tail -n 3 "$W/ra.log" "$W/rb.log"; trap - EXIT; cleanup
fi
exit $RC
