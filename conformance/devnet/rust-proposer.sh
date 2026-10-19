#!/usr/bin/env bash
# Rust-proposer devnet: the only validator is Go celestia-core on top of the
# Rust state machine (`celestia-rs abci`), so every block is built by the
# Rust PrepareProposal. A Go `celestia-appd` full node follows it: it runs
# ProcessProposal on each proposal and checks every block's app hash, so
# reaching <blocks> without a rejected proposal proves Go accepts the Rust
# proposals (square layout, data root) and executes them identically.
# Txs are submitted to the Go full node and gossiped to the validator's
# mempool (CheckTx in Rust).
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/devnet/rust-proposer.sh <blocks> [workload.sh]
# Workload hooks (<workload>.setup.sh / .genesis.py) run as in devnet.sh.
# Exit 0 iff the Go full node reaches <blocks> without rejecting a proposal.
set -euo pipefail
BLOCKS=${1:?blocks}; WORKLOAD=${2:-}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-4000}
CHAIN_ID=rs-proposer-1
W=$(mktemp -d)
V=$W/val; C=$W/core; G=$W/full
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

port() { echo $(( $1 + OFF + $2 )); }   # base, node index (0 validator, 100 full node)
shift_ports() { # file delta: shift every host:port listener
  python3 - "$1" "$2" <<'PY'
import re, sys
p, d = sys.argv[1], int(sys.argv[2])
s = open(p).read()
s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + d), s)
open(p, "w").write(s)
PY
}
relax_p2p() {
  sed -i.bak -e 's#^addr_book_strict = true#addr_book_strict = false#' \
    -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' "$1"
  rm -f "$1.bak"
}

# Validator: celestia-core with val0's keys, Rust app behind it.
mkdir -p "$C/config" "$C/data"
cometbft init --home "$C" >/dev/null 2>&1 || true
python3 - "$V" "$C" <<'PY'
import json, sys
v, c = sys.argv[1:]
g = json.load(open(v + "/config/genesis.json"))
d = {"genesis_time": g["genesis_time"], "chain_id": g["chain_id"],
     "initial_height": str(g["initial_height"]),
     "consensus_params": g["consensus"]["params"], "validators": [],
     "app_hash": "", "app_state": g["app_state"]}
json.dump(d, open(c + "/config/genesis.json", "w"))
PY
cp "$V/config/config.toml" "$C/config/config.toml"
cp "$V/config/priv_validator_key.json" "$V/config/node_key.json" "$C/config/"
echo '{"height":"0","round":0,"step":0}' > "$C/data/priv_validator_state.json"
shift_ports "$C/config/config.toml" "$OFF"
relax_p2p "$C/config/config.toml"
VAL_ID=$(cometbft show_node_id --home "$C" 2>/dev/null | tail -n1)

# Go full node.
celestia-appd init full --chain-id $CHAIN_ID --home "$G" >/dev/null 2>&1
cp "$V/config/genesis.json" "$G/config/genesis.json"
shift_ports "$G/config/config.toml" $((OFF + 100))
shift_ports "$G/config/app.toml" $((OFF + 100))
relax_p2p "$G/config/config.toml"
sed -i.bak "s#^persistent_peers = .*#persistent_peers = \"$VAL_ID@127.0.0.1:$(port 26656 0)\"#" "$G/config/config.toml"
rm -f "$G/config/config.toml.bak"

PIDS=(); cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }; trap cleanup EXIT
"$ROOT/target/debug/celestia-rs" abci --addr "127.0.0.1:$(port 26658 0)" > "$W/rust.log" 2>&1 & PIDS+=($!)
sleep 1
cometbft start --home "$C" --proxy_app "tcp://127.0.0.1:$(port 26658 0)" > "$W/val.log" 2>&1 & VAL=$!; PIDS+=($VAL)
celestia-appd start --force-no-bbr --home "$G" > "$W/full.log" 2>&1 & FULL=$!; PIDS+=($FULL)

h() { curl -s "localhost:$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
fail() {
  echo "FAILED: $1 validator=$(h "$(port 26657 0)") full=$(h "$(port 26657 100)")"
  grep -E "Rejected proposal|AppHash|CONSENSUS FAILURE|panic" "$W/full.log" | head -5
  grep -E "panic|ERROR|error" "$W/val.log" | head -5
  tail -5 "$W/rust.log"
  echo "logs: $W"
  trap - EXIT; cleanup; exit 1
}
for _ in $(seq 1 120); do
  [ "$(h "$(port 26657 100)" || echo 0)" -ge 2 ] 2>/dev/null && break
  kill -0 $VAL 2>/dev/null && kill -0 $FULL 2>/dev/null || fail "node exited during startup"
  sleep 1
done
if [ -n "$WORKLOAD" ]; then
  HOME_DIR=$V CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$(port 26657 100) bash "$WORKLOAD" || true
fi
for _ in $(seq 1 $((BLOCKS * 4 + 120))); do
  fh=$(h "$(port 26657 100)" || echo 0)
  if grep -q "Rejected proposal block\|wrong Block.Header.AppHash\|CONSENSUS FAILURE" "$W/full.log"; then
    fail "Go full node rejected a Rust proposal or diverged"
  fi
  if [ "${fh:-0}" -ge "$BLOCKS" ] 2>/dev/null; then
    echo "Go full node reached height $fh following the Rust proposer"
    exit 0
  fi
  kill -0 $FULL 2>/dev/null && kill -0 $VAL 2>/dev/null || break
  sleep 1
done
fail "timeout or node exit"
