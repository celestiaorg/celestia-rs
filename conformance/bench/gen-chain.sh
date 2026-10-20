#!/usr/bin/env bash
# B1 benchmarks, step 1: record a heavy chain.
#
# One Go celestia-appd v10.4.0 validator (standalone ABCI) behind Go
# celestia-core v0.42.3 with the abci-recorder proxy in between (as
# conformance/devnet/devnet.sh), loaded by `bench_load` (Rust example,
# locally signed txs): every height ~600 bank sends to fresh addresses +
# ~180 small PFBs (celestia-app caps blocks at 600 SDK messages + 200
# PFBs), and every $BENCH_BIG_EVERY heights 4 big PFBs alternating 128x128
# (~7.6 MB) and 256x256 (~29 MB) squares.
#
# Output ($BENCH_DIR/chain):
#   abci.rec      every ABCI frame (replayed by run.sh through both apps)
#   genesis.json  SDK genesis
#   home/         the validator's app + core homes (block store and
#                 snapshots: the source for the block / state sync runs)
#   meta.json
#
# usage: [BENCH_DIR=target/bench] [BENCH_HEIGHTS=500] [BENCH_BIG_EVERY=20]
#        [DEVNET_BIN=dir] [DEVNET_PORT_OFFSET=21000] conformance/bench/gen-chain.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-21000}
HEIGHTS=${BENCH_HEIGHTS:-500}
OUT=${BENCH_DIR:-$ROOT/target/bench}/chain
LOAD=${BENCH_LOAD:-$ROOT/target/release/examples/bench_load}
CHAIN_ID=rs-bench-1
P_RPC=$((26657 + OFF)); P_P2P=$((26656 + OFF)); P_PROXY=$((26658 + OFF)); P_APP=$((36658 + OFF))
P_GRPC=$((9090 + OFF)); P_API=$((1317 + OFF)); P_PPROF=$((6060 + OFF)); P_PRIVVAL_GRPC=$((26669 + OFF))

[ -x "$LOAD" ] || (cd "$ROOT" && cargo build -q --release -p celestia-rs --example bench_load)
rm -rf "$OUT" && mkdir -p "$OUT"
H=$OUT/home
A="celestia-appd --home $H"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A genesis add-genesis-account "$($A keys show val0 -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
# Load accounts (~1,000): written into the genesis directly (one CLI call
# per account would take minutes).
"$LOAD" --print-addresses > "$OUT/load-addresses.txt"
python3 - "$H/config/genesis.json" "$OUT/load-addresses.txt" <<'PY'
import json, sys
p, f = sys.argv[1], sys.argv[2]
g = json.load(open(p))
auth, bank = g["app_state"]["auth"], g["app_state"]["bank"]
n = len(auth["accounts"])
amount = 100000000000000
for i, addr in enumerate(open(f).read().split()):
    auth["accounts"].append({"@type": "/cosmos.auth.v1beta1.BaseAccount", "address": addr, "pub_key": None,
                             "account_number": str(n + i), "sequence": "0"})
    bank["balances"].append({"address": addr, "coins": [{"denom": "utia", "amount": str(amount)}]})
if bank.get("supply"):  # empty: computed from the balances at InitGenesis
    supply = {c["denom"]: int(c["amount"]) for c in bank["supply"]}
    supply["utia"] = supply.get("utia", 0) + amount * len(open(f).read().split())
    bank["supply"] = [{"denom": d, "amount": str(v)} for d, v in sorted(supply.items())]
json.dump(g, open(p, "w"), indent=1)
PY
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
# Snapshots for the state sync run (every 100 heights, keep 2).
sed -i.bak -e 's#^snapshot-interval = .*#snapshot-interval = 100#' -e 's#^snapshot-keep-recent = .*#snapshot-keep-recent = 2#' \
  -e "s#localhost:9090#localhost:$P_GRPC#" -e "s#tcp://localhost:1317#tcp://localhost:$P_API#" \
  -e "s#0.0.0.0:9090#0.0.0.0:$P_GRPC#" "$H/config/app.toml"
rm -f "$H/config/app.toml.bak"
sed -i.bak -e "s#tcp://127.0.0.1:26657#tcp://127.0.0.1:$P_RPC#" -e "s#tcp://0.0.0.0:26656#tcp://127.0.0.1:$P_P2P#" \
  -e "s#localhost:6060#localhost:$P_PPROF#" -e "s#127.0.0.1:26669#127.0.0.1:$P_PRIVVAL_GRPC#" \
  -e "s#tcp://127.0.0.1:9098#tcp://127.0.0.1:$((9098 + OFF))#" \
  -e 's#^addr_book_strict = true#addr_book_strict = false#' -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' \
  -e 's#^max_request_batch_size = .*#max_request_batch_size = 1000#' "$H/config/config.toml"
rm -f "$H/config/config.toml.bak"

# celestia-core wants a CometBFT GenesisDoc, not the SDK app genesis.
mkdir -p "$H/core/config" "$H/core/data"
python3 - "$H" <<'PY'
import json, sys
h = sys.argv[1]
g = json.load(open(h + "/config/genesis.json"))
c = {"genesis_time": g["genesis_time"], "chain_id": g["chain_id"],
     "initial_height": str(g["initial_height"]),
     "consensus_params": g["consensus"]["params"],
     "validators": g["consensus"].get("validators") or [],
     "app_hash": g.get("app_hash") or "", "app_state": g["app_state"]}
json.dump(c, open(h + "/core/config/genesis.json", "w"), indent=2)
PY
cp "$H"/config/{config.toml,node_key.json,priv_validator_key.json} "$H/core/config/"
echo '{"height":"0","round":0,"step":0}' > "$H/core/data/priv_validator_state.json"

PIDS=()
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; wait 2>/dev/null || true; }
trap cleanup EXIT
celestia-appd start --force-no-bbr --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP > "$OUT/app.log" 2>&1 &
PIDS+=($!); sleep 4
abci-recorder 127.0.0.1:$P_PROXY 127.0.0.1:$P_APP "$OUT/abci.rec" > "$OUT/rec.log" 2>&1 &
PIDS+=($!); sleep 1
cometbft start --home "$H/core" --proxy_app tcp://127.0.0.1:$P_PROXY > "$OUT/core.log" 2>&1 &
CORE=$!; PIDS+=($CORE)
height() { curl -s localhost:$P_RPC/status | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
until [ "$(height || echo 0)" -ge 2 ] 2>/dev/null; do kill -0 $CORE || { tail "$OUT/core.log"; exit 1; }; sleep 1; done
T0=$(date +%s)
"$LOAD" --node "http://127.0.0.1:$P_RPC" --chain-id $CHAIN_ID --heights "$HEIGHTS" \
  --big-every "${BENCH_BIG_EVERY:-20}" | tee "$OUT/load.log"
# Let the mempool drain into a few more blocks, then stop consensus first
# so the app commits nothing past what core saw.
sleep 15
LAST=$(height)
kill $CORE; wait $CORE 2>/dev/null || true
sleep 2
cleanup
cp "$H/config/genesis.json" "$OUT/genesis.json"
cat > "$OUT/meta.json" <<JSON
{
  "chain_id": "$CHAIN_ID",
  "celestia_app": "v10.4.0",
  "celestia_core": "v0.42.3",
  "last_height": $LAST,
  "generation_secs": $(( $(date +%s) - T0 )),
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
echo "wrote $OUT (heights 1..$LAST, $(du -sh "$OUT/abci.rec" | cut -f1) recording)"
