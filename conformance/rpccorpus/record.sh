#!/usr/bin/env bash
# Record the CometBFT RPC differential corpus of a devnet scenario against
# Go celestia-core v0.42.3 (+ Go celestia-appd v10.4.0).
#
# usage: [DEVNET_PORT_OFFSET=n] conformance/rpccorpus/record.sh <scenario> <blocks> [workload.sh]
#
# 1. Live: a single-validator Go devnet (celestia-appd as standalone ABCI app
#    behind the `cometbft` binary of celestia-core, tx/block indexing on,
#    FinalizeBlock responses kept, celestia-core gRPC on) runs the workload
#    while `recorder ws` records WebSocket event subscriptions.
# 2. The node is stopped at <blocks> (or later) and its block store dumped
#    (`recorder dump`: blocks + seen commits).
# 3. Frozen: the node restarts with a fresh, non-validator key (so the chain
#    stays at its last height) and `recorder record` issues the request
#    corpus (URI and JSON-RPC routes, WebSocket calls, gRPC BlockAPI /
#    BlobstreamAPI / BroadcastAPI) incl. broadcasts of freshly signed txs.
#
# Output: conformance/rpccorpus/<scenario>/
#   genesis.json        SDK app genesis (InitChain)

#   blocks.bin.gz       blocks and seen commits (uvarint-length-prefixed protos)
#   ws.jsonl.gz         live WebSocket frames
#   corpus.json.gz      requests and Go's responses
#   meta.json           versions, heights
set -euo pipefail

SCENARIO=${1:?scenario}
BLOCKS=${2:?blocks}
WORKLOAD=${3:-}
CHAIN_ID=${CHAIN_ID:-rs-devnet-1}
OFF=${DEVNET_PORT_OFFSET:-0}
P_RPC=$((26657 + OFF)); P_P2P=$((26656 + OFF)); P_APP=$((36658 + OFF)); P_CGRPC=$((9098 + OFF))
P_GRPC=$((9090 + OFF)); P_API=$((1317 + OFF)); P_PPROF=$((6060 + OFF)); P_PRIVVAL_GRPC=$((26669 + OFF))

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
WORK=$(mktemp -d)
OUT=$ROOT/conformance/rpccorpus/$SCENARIO
export PATH=$BIN:$PATH
for b in celestia-appd cometbft; do
  command -v $b >/dev/null || { echo "missing $b in $BIN (build it with conformance/devnet/devnet.sh)"; exit 1; }
done
REC=$BIN/rpc-recorder
[ -x "$REC" ] || (cd "$ROOT/conformance/rpccorpus/recorder" && GOFLAGS=-mod=mod go build -o "$REC" .)

H=$WORK/home
A="celestia-appd --home $H"
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
  HOME_DIR=$H CHAIN_ID=$CHAIN_ID bash "$GENESIS_SETUP"
fi
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
if [ -n "${GENESIS_PATCH:-}" ]; then
  python3 "$GENESIS_PATCH" "$H/config/genesis.json"
fi
sed -i.bak 's/^pruning = "default"/pruning = "nothing"/' "$H/config/app.toml"
rm -f "$H/config/app.toml.bak"
sed -i.bak -e "s#tcp://127.0.0.1:26657#tcp://127.0.0.1:$P_RPC#" -e "s#tcp://0.0.0.0:26656#tcp://0.0.0.0:$P_P2P#" \
  -e "s#localhost:6060#localhost:$P_PPROF#" -e "s#127.0.0.1:26669#127.0.0.1:$P_PRIVVAL_GRPC#" \
  -e "s#tcp://127.0.0.1:9098#tcp://127.0.0.1:$P_CGRPC#" \
  -e 's#^indexer = "null"#indexer = "kv"#' -e 's#^discard_abci_responses = true#discard_abci_responses = false#' \
  -e 's#^grpc_laddr = ""#grpc_laddr = "tcp://127.0.0.1:'$P_CGRPC'"#' "$H/config/config.toml"
rm -f "$H/config/config.toml.bak"
sed -i.bak -e "s#localhost:9090#localhost:$P_GRPC#" -e "s#tcp://localhost:1317#tcp://localhost:$P_API#" -e "s#0.0.0.0:9090#0.0.0.0:$P_GRPC#" "$H/config/app.toml"
rm -f "$H/config/app.toml.bak"
grep -q "^grpc_laddr = \"tcp://127.0.0.1:$P_CGRPC\"" "$H/config/config.toml" || { echo "grpc_laddr not set"; exit 1; }

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
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }
LOGS=$ROOT/target/rpccorpus-logs/$SCENARIO
trap 'cleanup; mkdir -p "$LOGS"; cp "$WORK"/*.log "$LOGS/" 2>/dev/null; rm -rf "$WORK"' EXIT
start_node() {
  celestia-appd start --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP >> "$WORK/app.log" 2>&1 &
  APP=$!; PIDS+=($APP); sleep 4
  cometbft start --home "$H/core" --proxy_app tcp://127.0.0.1:$P_APP >> "$WORK/core.log" 2>&1 &
  CORE=$!; PIDS+=($CORE)
}
stop_node() {
  kill $CORE 2>/dev/null || true; wait $CORE 2>/dev/null || true
  kill $APP 2>/dev/null || true; wait $APP 2>/dev/null || true
}
height() { curl -s localhost:$P_RPC/status | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }

# REUSE=1 skips the live run and the dump and restarts from the home saved
# by the previous run of the scenario (to iterate on the request corpus).
SAVED=$ROOT/target/rpccorpus-home/$SCENARIO
if [ -n "${REUSE:-}" ] && [ -d "$SAVED" ]; then
  rm -rf "$H" && cp -r "$SAVED" "$H"
  cp "$SAVED.ws.jsonl.gz" "$WORK/ws.jsonl.gz"
else
# --- 1. live run ---------------------------------------------------------------
start_node
"$REC" ws -url ws://127.0.0.1:$P_RPC/websocket -out "$WORK/ws.jsonl" \
  -q "tm.event='NewBlock'" -q "tm.event='NewBlockHeader'" -q "tm.event='Tx'" \
  -q "tm.event='NewBlockEvents'" -q "tm.event='ValidatorSetUpdates'" -q "tm.event='NewSignedBlock'" \
  -q "tm.event='Tx' AND message.action='/cosmos.bank.v1beta1.MsgSend'" \
  -q "tm.event='NewEvidence'" -q "tx.height>3" -q "tm.event='NewBlock' AND block.height>2" \
  > "$WORK/ws.log" 2>&1 &
WS=$!; PIDS+=($WS)
until [ "$(height || echo 0)" -ge 2 ] 2>/dev/null; do kill -0 $CORE || { tail "$WORK/core.log"; exit 1; }; sleep 1; done
if [ -n "$WORKLOAD" ]; then
  HOME_DIR=$H CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$P_RPC bash "$WORKLOAD"
fi
until [ "$(height || echo 0)" -ge "$BLOCKS" ] 2>/dev/null; do
  kill -0 $CORE 2>/dev/null || { tail "$WORK/core.log"; exit 1; }
  sleep 1
done
kill $WS 2>/dev/null || true; wait $WS 2>/dev/null || true
stop_node

# --- 2. dump ------------------------------------------------------------------------
rm -rf "$OUT" && mkdir -p "$OUT"
"$REC" dump -data "$H/core/data" -out "$OUT/blocks.bin.gz"
gzip -9 -n -c "$WORK/ws.jsonl" > "$WORK/ws.jsonl.gz"
mkdir -p "$(dirname "$SAVED")" && cp "$WORK/ws.jsonl.gz" "$SAVED.ws.jsonl.gz"
rm -rf "$SAVED" && mkdir -p "$(dirname "$SAVED")" && cp -r "$H" "$SAVED"
fi

# --- 3. frozen node ------------------------------------------------------------------
# The frozen node is `celestia-appd start` with celestia-core embedded (the
# production layout, where CheckTx errors are RPC errors instead of ABCI
# socket failures and the app's gRPC server also serves BlockAPI): the
# consensus data moves next to the app data, the core config (indexer, gRPC
# address) replaces the app home's config.toml.
celestia-appd init frozen --chain-id $CHAIN_ID --home "$WORK/frozen" >/dev/null 2>&1
cp -r "$H/core/data/"* "$H/data/"
cp "$H/core/config/config.toml" "$H/config/config.toml"
cp "$WORK/frozen/config/priv_validator_key.json" "$H/config/priv_validator_key.json"
sed -i.bak -e 's#^block_sync = true#block_sync = false#' "$H/config/config.toml"
rm -f "$H/config/config.toml.bak"
celestia-appd start --home "$H" >> "$WORK/frozen.log" 2>&1 &
APP=$!; CORE=$APP; PIDS+=($APP)
for _ in $(seq 1 60); do curl -s localhost:$P_RPC/health >/dev/null && break; sleep 1; done
sleep 3

# Txs to broadcast: signed offline by user0 against the frozen state.
K=(--keyring-backend test --home "$H")
USER0=$($A keys show user0 -a "${K[@]}")
VAL0=$($A keys show val0 -a "${K[@]}")
ACC=$($A query auth account "$USER0" --node tcp://127.0.0.1:$P_RPC -o json)
NUM=$(echo "$ACC" | python3 -c 'import json,sys; d=json.load(sys.stdin); a=d.get("account",d); a=a.get("value",a); print(a.get("account_number","0"))')
SEQ=$(echo "$ACC" | python3 -c 'import json,sys; d=json.load(sys.stdin); a=d.get("account",d); a=a.get("value",a); print(a.get("sequence","0"))')
sign() { # <name> <sequence> <fee> <amount>
  $A tx bank send "$USER0" "$VAL0" "$4" "${K[@]}" --chain-id $CHAIN_ID --fees "$3" --gas 100000 \
    --generate-only -o json > "$WORK/$1.unsigned.json" 2>/dev/null
  $A tx sign "$WORK/$1.unsigned.json" --from user0 "${K[@]}" --chain-id $CHAIN_ID --offline \
    --account-number "$NUM" --sequence "$2" -o json > "$WORK/$1.signed.json" 2>/dev/null
  $A tx encode "$WORK/$1.signed.json" 2>/dev/null
}
python3 - "$WORK/txs.json" \
  "a-ok=$(sign a "$SEQ" 2000utia 1utia)" \
  "b-ok-async=$(sign b "$((SEQ + 1))" 2000utia 2utia)" \
  "c-json-ok=$(sign c "$((SEQ + 2))" 4000utia 3utia)" \
  "d-stale-seq=$(sign d "$((SEQ > 0 ? SEQ - 1 : 0))" 2000utia 4utia)" \
  "e-low-fee=$(sign e "$((SEQ + 3))" 1utia 5utia)" \
  "f-future-seq=$(sign f "$((SEQ + 9))" 2000utia 6utia)" <<'PY'
import json, sys
json.dump(dict(a.split("=", 1) for a in sys.argv[2:]), open(sys.argv[1], "w"), indent=1)
PY
"$REC" record -rpc http://127.0.0.1:$P_RPC -grpc 127.0.0.1:$P_CGRPC -txs "$WORK/txs.json" -out "$OUT/corpus.json"
stop_node

cp "$H/config/genesis.json" "$OUT/genesis.json"

gzip -9 -n -f "$OUT/corpus.json"
cp "$WORK/ws.jsonl.gz" "$OUT/ws.jsonl.gz"
cat > "$OUT/meta.json" <<JSON
{
  "scenario": "$SCENARIO",
  "chain_id": "$CHAIN_ID",
  "celestia_app": "v10.4.0",
  "celestia_core": "v0.42.3",
  "blocks": $BLOCKS,
  "workload": "$(basename "${WORKLOAD:-none}")",
  "frozen_validator": $(python3 -c 'import json,sys; k=json.load(open(sys.argv[1])); print(json.dumps({"address": k["address"], "pub_key": k["pub_key"]}))' "$H/config/priv_validator_key.json"),
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
echo "wrote $OUT"
