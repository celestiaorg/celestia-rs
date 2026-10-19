#!/usr/bin/env bash
# Mixed devnet: a Go celestia-appd validator plus a follower node running Go
# celestia-core on top of the Rust state machine (`celestia-rs abci`).
# celestia-core checks every block's app hash against the follower's own
# state, so the follower halts on the first divergence.
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/devnet/mixed.sh <blocks> [workload.sh]
# Workload hooks (<workload>.setup.sh / .genesis.py) run as in devnet.sh.
# Exit 0 iff the follower reaches <blocks> with the validator.
set -euo pipefail
BLOCKS=${1:?blocks}; WORKLOAD=${2:-}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-3000}
CHAIN_ID=rs-mixed-1
W=$(mktemp -d)
V=$W/val; F=$W/fol
A="celestia-appd --home $V"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A keys add user0 --keyring-backend test >/dev/null 2>&1
for k in val0 user0; do
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
if [ -n "$WORKLOAD" ]; then
  GENESIS_SETUP=${GENESIS_SETUP:-$(ls "${WORKLOAD%.sh}.setup.sh" 2>/dev/null || true)}
  GENESIS_PATCH=${GENESIS_PATCH:-$(ls "${WORKLOAD%.sh}.genesis.py" 2>/dev/null || true)}
fi
if [ -n "${GENESIS_SETUP:-}" ]; then
  HOME_DIR=$V CHAIN_ID=$CHAIN_ID bash "$GENESIS_SETUP"
fi
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
if [ -n "${GENESIS_PATCH:-}" ]; then
  python3 "$GENESIS_PATCH" "$V/config/genesis.json"
fi

port() { echo $(( $1 + OFF + $2 )); }   # base, node index (0 validator, 100 follower)
# Shift every host:port listener in a config file by <delta>.
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
sed -i.bak -e 's#^addr_book_strict = true#addr_book_strict = false#' -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' "$V/config/config.toml"
rm -f "$V/config/config.toml.bak"

# Follower: celestia-core home with the validator's genesis (CometBFT form).
mkdir -p "$F/config" "$F/data"
cometbft init --home "$F" >/dev/null 2>&1 || true
python3 - "$V" "$F" <<'PY'
import json, sys
v, f = sys.argv[1:]
g = json.load(open(v + "/config/genesis.json"))
c = {"genesis_time": g["genesis_time"], "chain_id": g["chain_id"],
     "initial_height": str(g["initial_height"]),
     "consensus_params": g["consensus"]["params"], "validators": [],
     "app_hash": "", "app_state": g["app_state"]}
json.dump(c, open(f + "/config/genesis.json", "w"))
PY
cp "$V/config/config.toml" "$F/config/config.toml"
shift_ports "$F/config/config.toml" 100
VAL_ID=$(celestia-appd comet show-node-id --home "$V" 2>/dev/null || cometbft show_node_id --home "$V")
sed -i.bak "s#^persistent_peers = .*#persistent_peers = \"$VAL_ID@127.0.0.1:$(port 26656 0)\"#" "$F/config/config.toml"
rm -f "$F/config/config.toml.bak"

PIDS=(); cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }; trap cleanup EXIT
celestia-appd start --force-no-bbr --home "$V" > "$W/val.log" 2>&1 & PIDS+=($!)
"$ROOT/target/debug/celestia-rs" abci --addr "127.0.0.1:$(port 26658 100)" > "$W/rust.log" 2>&1 & PIDS+=($!)
sleep 1
cometbft start --home "$F" --proxy_app "tcp://127.0.0.1:$(port 26658 100)" > "$W/fol.log" 2>&1 & FOL=$!; PIDS+=($FOL)

h() { curl -s "localhost:$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
until [ "$(h "$(port 26657 0)" || echo 0)" -ge 2 ] 2>/dev/null; do sleep 1; done
if [ -n "$WORKLOAD" ]; then
  HOME_DIR=$V CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$(port 26657 0) bash "$WORKLOAD" || true
fi
for _ in $(seq 1 $((BLOCKS * 4 + 120))); do
  fh=$(h "$(port 26657 100)" || echo 0)
  [ "${fh:-0}" -ge "$BLOCKS" ] 2>/dev/null && { echo "follower reached height $fh"; exit 0; }
  kill -0 $FOL 2>/dev/null || break
  if grep -q "wrong Block.Header.AppHash\|CONSENSUS FAILURE" "$W/fol.log"; then break; fi
  sleep 1
done
echo "FAILED: validator=$(h "$(port 26657 0)") follower=${fh:-?}"
grep -E "AppHash|CONSENSUS FAILURE|panic|ERROR" "$W/fol.log" | head -5
tail -5 "$W/rust.log"
echo "logs: $W"
trap - EXIT; cleanup; exit 1
