#!/usr/bin/env bash
# Record the query corpus of the celestia-core-backed services against a
# live Go node: celestia-appd (standalone ABCI) behind celestia-core, as in
# conformance/devnet/devnet.sh, with the app's RPC client pointed at a
# recording proxy in front of celestia-core.
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/querycorpus/record-comet.sh [scenario] [workload]
#
# Writes
#   conformance/fixtures/devnet/<scenario>/   genesis, abci.rec, state dumps,
#                                             rpc.json.gz (every JSON-RPC
#                                             exchange the Go app made)
#   conformance/querycorpus/<scenario>.json.gz
# The Rust test replays abci.rec, serves rpc.json.gz from a mock
# celestia-core RPC and requires Go's answers (queries name a committed
# height: the node keeps producing blocks while they run).
set -euo pipefail
trap 'echo "record-comet.sh: failed at line $LINENO" >&2' ERR

SCENARIO=${1:-comet-queries}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
WORKLOAD=${2:-$ROOT/conformance/querycorpus/comet-workload.sh}
CHAIN_ID=${CHAIN_ID:-rs-devnet-1}
OFF=${DEVNET_PORT_OFFSET:-0}
P_RPC=$((26657 + OFF)); P_P2P=$((26656 + OFF)); P_PROXY=$((26658 + OFF)); P_APP=$((36658 + OFF))
P_GRPC=$((9090 + OFF)); P_API=$((1317 + OFF)); P_PPROF=$((6060 + OFF)); P_PRIVVAL_GRPC=$((26669 + OFF))
P_RPCREC=$((26757 + OFF))
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
WORK=$(mktemp -d)
OUT=$ROOT/conformance/fixtures/devnet/$SCENARIO

(cd "$ROOT" && CARGO_INCREMENTAL=0 cargo build -q -p devnet-conformance --example record_comet)
REC=$ROOT/target/debug/examples/record_comet

H=$WORK/home
A="celestia-appd --home $H"
$A init val0 --chain-id "$CHAIN_ID" >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A keys add user0 --keyring-backend test >/dev/null 2>&1
for k in val0 user0; do
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
$A genesis gentx val0 5000000000utia --chain-id "$CHAIN_ID" --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
sed -i 's/^pruning = "default"/pruning = "nothing"/' "$H/config/app.toml"
sed -i -e "s#tcp://127.0.0.1:26657#tcp://127.0.0.1:$P_RPC#" -e "s#tcp://0.0.0.0:26656#tcp://0.0.0.0:$P_P2P#" \
  -e "s#localhost:6060#localhost:$P_PPROF#" -e "s#127.0.0.1:26669#127.0.0.1:$P_PRIVVAL_GRPC#" \
  -e "s#tcp://127.0.0.1:9098#tcp://127.0.0.1:$((9098 + OFF))#" \
  -e "s#^indexer = \"null\"#indexer = \"kv\"#" \
  -e "s#^discard_abci_responses = true#discard_abci_responses = false#" "$H/config/config.toml"
python3 - "$H/config/app.toml" "$P_API" "$P_GRPC" <<'PY'
import sys
p, api, grpc = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
def section(s, name, f):
    i = s.index("[%s]" % name)
    j = s.find("\n[", i + 1)
    j = len(s) if j < 0 else j
    return s[:i] + f(s[i:j]) + s[j:]
s = section(s, "api", lambda b: b.replace("enable = false", "enable = true", 1)
            .replace("tcp://localhost:1317", "tcp://127.0.0.1:" + api))
s = section(s, "grpc", lambda b: b.replace("enable = false", "enable = true", 1)
            .replace("localhost:9090", "127.0.0.1:" + grpc)
            .replace("0.0.0.0:9090", "127.0.0.1:" + grpc))
open(p, "w").write(s)
PY
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
# The app (not celestia-core) talks to the recording proxy.
sed -i -e "s#tcp://127.0.0.1:$P_RPC#tcp://127.0.0.1:$P_RPCREC#" "$H/config/config.toml"

PIDS=()
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }
trap cleanup EXIT
celestia-appd start --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP > "$WORK/app.log" 2>&1 &
PIDS+=($!); sleep 4
abci-recorder 127.0.0.1:$P_PROXY 127.0.0.1:$P_APP "$WORK/abci.rec" > "$WORK/rec.log" 2>&1 &
PIDS+=($!); sleep 1
cometbft start --home "$H/core" --proxy_app tcp://127.0.0.1:$P_PROXY > "$WORK/core.log" 2>&1 &
CORE=$!; PIDS+=($CORE)

height() { curl -s localhost:$P_RPC/status | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
until [ "$(height || echo 0)" -ge 2 ] 2>/dev/null; do kill -0 $CORE || { tail "$WORK/core.log"; exit 1; }; sleep 1; done
HOME_DIR=$H CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$P_RPC bash "$WORKLOAD"

mkdir -p "$OUT"
"$REC" "$SCENARIO" "127.0.0.1:$P_APP" "127.0.0.1:$P_API" "127.0.0.1:$P_RPCREC" "127.0.0.1:$P_RPC" \
  "$ROOT/conformance/querycorpus/$SCENARIO.json" "$OUT/rpc.json"
gzip -9 -n -f "$ROOT/conformance/querycorpus/$SCENARIO.json" "$OUT/rpc.json"

kill $CORE 2>/dev/null || true; sleep 2; cleanup; sleep 1
LAST=$(python3 - "$WORK/abci.rec" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
i = n = 0
while i < len(d):
    conn, dr, ln = struct.unpack("<IBI", d[i:i+9]); body = d[i+9:i+9+ln]; i += 9 + ln
    if dr == 0 and body[:1] == b"\x5a": n += 1
print(n)
PY
)
rm -rf "$OUT/state" && mkdir -p "$OUT/state"
cp "$H/config/genesis.json" "$OUT/genesis.json"
cp "$WORK/abci.rec" "$OUT/abci.rec"
for h in 1 "$LAST"; do
  devnet-oracle statedump --db "$H/data/application.db" --height "$h" > "$OUT/state/$h.json"
done
cat > "$OUT/meta.json" <<JSON
{
  "scenario": "$SCENARIO",
  "chain_id": "$CHAIN_ID",
  "celestia_app": "v10.4.0",
  "celestia_core": "v0.42.3",
  "committed_heights": $LAST,
  "workload": "$(basename "$WORKLOAD")",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
rm -rf "$WORK"
echo "wrote $OUT and conformance/querycorpus/$SCENARIO.json.gz (last height $LAST)"
