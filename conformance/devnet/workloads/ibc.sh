#!/usr/bin/env bash
# IBC workload for devnet.sh: a second Go chain ("peer") connected to the
# recorded devnet by the Go relayer (cosmos/relayer v2, `rly`).
#
# Exercises on the recorded chain: 07-tendermint CreateClient and
# UpdateClient (adjacent and non-adjacent headers), the connection and
# channel handshakes (both directions of each step), ICS-20 MsgTransfer
# (escrow of the native denom, burn of a voucher), RecvPacket (unescrow,
# mint with a new denom trace, failing receive -> error ack), Acknowledgement
# (success and error -> refund), Timeout (refund), packet-forward-middleware
# forwarding back to the peer, and redundant relays.
#
# Env (from devnet.sh, run with DEVNET_TX_INDEX=kv): HOME_DIR, CHAIN_ID, NODE;
# celestia-appd on PATH.
# DEVNET_PORT_OFFSET (inherited) shifts the peer's ports by +500; `rly` must
# be on PATH (conformance/devnet/build-rly.sh).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
RPC=${NODE/tcp:/http:}
K=(--keyring-backend test --home "$HOME_DIR")
USER0=$(celestia-appd keys show user0 -a "${K[@]}")

OFF=$(( ${DEVNET_PORT_OFFSET:-0} + 500 ))
PEER_ID=rs-peer-2
PEER=$(mktemp -d)/peer
P_RPC=$((26657 + OFF)); P_P2P=$((26656 + OFF)); P_GRPC=$((9090 + OFF)); P_API=$((1317 + OFF))
P_PPROF=$((6060 + OFF)); P_PRIVVAL=$((26669 + OFF)); P_APP=$((36658 + OFF)); P_RPCGRPC=$((9098 + OFF))
PEER_NODE=tcp://127.0.0.1:$P_RPC
PEER_RPC=http://127.0.0.1:$P_RPC
PK=(--keyring-backend test --home "$PEER")

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

# --- peer chain --------------------------------------------------------------
P="celestia-appd --home $PEER"
$P init peer0 --chain-id $PEER_ID >/dev/null 2>&1
$P keys add val0 "${PK[@]}" >/dev/null 2>&1
$P keys add user0 "${PK[@]}" >/dev/null 2>&1
RLY_PEER_MN=$($P keys add relayer "${PK[@]}" --output json 2>&1 | json 'd["mnemonic"]')
for k in val0 user0 relayer; do
  $P genesis add-genesis-account "$($P keys show $k -a "${PK[@]}")" 1000000000000utia >/dev/null 2>&1
done
$P genesis gentx val0 5000000000utia --chain-id $PEER_ID "${PK[@]}" --fees 500000utia >/dev/null 2>&1
$P genesis collect-gentxs >/dev/null 2>&1
sed -i -e "s#tcp://127.0.0.1:26657#tcp://127.0.0.1:$P_RPC#" -e "s#tcp://0.0.0.0:26656#tcp://0.0.0.0:$P_P2P#" \
  -e "s#tcp://127.0.0.1:36658#tcp://127.0.0.1:$P_APP#" -e "s#tcp://127.0.0.1:9098#tcp://127.0.0.1:$P_RPCGRPC#" \
  -e "s#localhost:6060#localhost:$P_PPROF#" -e "s#127.0.0.1:26669#127.0.0.1:$P_PRIVVAL#" \
  -e "s#127.0.0.1:26659#127.0.0.1:$P_PRIVVAL#" "$PEER/config/config.toml"
sed -i -e "s#localhost:9090#localhost:$P_GRPC#" -e "s#tcp://localhost:1317#tcp://localhost:$P_API#" \
  -e "s#0.0.0.0:9090#0.0.0.0:$P_GRPC#" "$PEER/config/app.toml"
sed -i -e 's#^indexer = "null"#indexer = "kv"#' -e 's#^discard_abci_responses = true#discard_abci_responses = false#' \
  "$PEER/config/config.toml"
celestia-appd start --home "$PEER" > "$PEER.log" 2>&1 &
PEER_PID=$!
trap 'kill $PEER_PID 2>/dev/null || true' EXIT
peer_height() { curl -s "$PEER_RPC/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
until [ "$(peer_height || echo 0)" -ge 2 ] 2>/dev/null; do
  kill -0 $PEER_PID || { tail -30 "$PEER.log"; exit 1; }
  sleep 1
done

# --- relayer -----------------------------------------------------------------
RLY_MN=$(celestia-appd keys add relayer "${K[@]}" --output json 2>&1 | json 'd["mnemonic"]')
RLY_ADDR=$(celestia-appd keys show relayer -a "${K[@]}")

LAST_HEIGHT=0
# tx <home> <node> <from> <args...>: broadcast and wait for inclusion.
tx() {
  local home=$1 node=$2 from=$3
  shift 3
  local out hash res chain
  chain=$CHAIN_ID; [ "$home" = "$PEER" ] && chain=$PEER_ID
  out=$(celestia-appd tx "$@" --from "$from" --keyring-backend test --home "$home" --chain-id "$chain" \
    --node "$node" --fees 40000utia --gas 400000 -y -o json)
  if [ "$(echo "$out" | json 'd.get("code", 0)')" != "0" ]; then
    echo "checktx failed: $out" >&2
    return 1
  fi
  hash=$(echo "$out" | json 'd["txhash"]')
  res=$(python3 "$HERE/wait_tx.py" "${node/tcp:/http:}" "$hash") || {
    echo "tx $hash not included" >&2
    return 1
  }
  LAST_HEIGHT=${res%% *}
  echo "$* -> $res"
}
main_tx() { tx "$HOME_DIR" "$NODE" "$@"; }
peer_tx() { tx "$PEER" "$PEER_NODE" "$@"; }

main_tx user0 bank send "$USER0" "$RLY_ADDR" 100000000utia

RH=$(mktemp -d)/rly
export RLY_FIXED_GAS=${RLY_FIXED_GAS:-3000000}
rly() { command rly --home "$RH" "$@"; }
rly config init >/dev/null
chain_json() { # <chain-id> <rpc> <grpc>
  cat <<JSON
{"type": "cosmos", "value": {"key": "relayer", "chain-id": "$1", "rpc-addr": "$2",
 "grpc-addr": "$3", "account-prefix": "celestia", "keyring-backend": "test",
 "gas-adjustment": 1.5, "gas-prices": "0.1utia", "min-gas-amount": 0, "max-gas-amount": 0,
 "debug": false, "timeout": "20s", "output-format": "json",
 "sign-mode": "direct", "extra-codecs": [], "coin-type": 118, "signing-algorithm": "",
 "broadcast-mode": "batch"}}
JSON
}
chain_json "$CHAIN_ID" "$RPC" "http://127.0.0.1:$((9090 + ${DEVNET_PORT_OFFSET:-0}))" > "$RH.main.json"
chain_json "$PEER_ID" "$PEER_RPC" "http://127.0.0.1:$P_GRPC" > "$RH.peer.json"
rly chains add --file "$RH.main.json" main >/dev/null
rly chains add --file "$RH.peer.json" peer >/dev/null
rly keys restore main relayer "$RLY_MN" >/dev/null
rly keys restore peer relayer "$RLY_PEER_MN" >/dev/null
rly paths new "$CHAIN_ID" "$PEER_ID" demo >/dev/null


# Clients, connection and an ICS-20 channel (both chains run Go).
rly tx link demo --src-port transfer --dst-port transfer --order unordered --version ics20-1 \
  --client-tp 10m --max-retries 10 2>&1 | tail -1
CH=$(rly q channels main -o json 2>/dev/null | json 'd["channel_id"]' | head -1)
PEER_CH=$(rly q channels peer -o json 2>/dev/null | json 'd["channel_id"]' | head -1)
echo "channels: main $CH, peer $PEER_CH"
PEER_USER=$($P keys show user0 -a "${PK[@]}")

# Packets are relayed by a running `rly start` (its startup flush needs
# tx_search, which rly cannot decode from celestia-core; live relaying works).
# (the binary itself, not the `rly` function: `$!` must be the relayer, not
# a subshell that `kill` would leave the relayer running behind)
command rly --home "$RH" start demo --flush-interval 1h > "$RH.start.log" 2>&1 &
RLY_PID=$!
trap 'kill $RLY_PID $PEER_PID 2>/dev/null || true' EXIT
sleep 5
# settle: wait until no packets or acks are left unrelayed on the channel.
settle() {
  local i out
  for i in $(seq 1 60); do
    out=$(rly q unrelayed-packets demo "$CH" 2>/dev/null)$(rly q unrelayed-acknowledgements demo "$CH" 2>/dev/null)
    if [ "$(echo "$out" | sed 's/}{/}\n{/' | python3 -c 'import json,sys; print(all(not v for l in sys.stdin for v in json.loads(l).values()))')" = True ]; then
      sleep 3
      return 0
    fi
    sleep 2
  done
  echo "not settled: $out" >&2
  return 1
}

# 1. main -> peer (escrow utia), relayed both ways.
main_tx user0 ibc-transfer transfer transfer "$CH" "$PEER_USER" 1000utia --memo "hello"
settle

# 2. peer -> main: mints a voucher with a new denom trace on main.
peer_tx user0 ibc-transfer transfer transfer "$PEER_CH" "$USER0" 5000utia
settle
VOUCHER=$(celestia-appd q bank balances "$USER0" --node "$NODE" -o json | json '[c["denom"] for c in d["balances"] if c["denom"].startswith("ibc/")][0]')
echo "voucher on main: $VOUCHER"

# 3. send the voucher back (burn on main, unescrow on peer).
main_tx user0 ibc-transfer transfer transfer "$CH" "$PEER_USER" "2000$VOUCHER"
settle

# 4. receive failure on main (invalid receiver) -> error ack, refund on peer.
peer_tx user0 ibc-transfer transfer transfer "$PEER_CH" "not-an-address" 7utia
settle

# 5. PFM: peer -> main -> peer (main forwards back to the peer).
FWD="{\"forward\":{\"receiver\":\"$PEER_USER\",\"port\":\"transfer\",\"channel\":\"$CH\"}}"
peer_tx user0 ibc-transfer transfer transfer "$PEER_CH" "$USER0" 300utia --memo "$FWD"
settle

# 6. PFM with a failing hop: forwarding to a channel that does not exist
#    on main -> error ack back to the peer.
BAD="{\"forward\":{\"receiver\":\"$PEER_USER\",\"port\":\"transfer\",\"channel\":\"channel-99\"}}"
peer_tx user0 ibc-transfer transfer transfer "$PEER_CH" "$USER0" 301utia --memo "$BAD"
settle

# 7. timeout: a packet from main whose timestamp elapses before relaying;
#    rly then sends MsgTimeout to main (refund).
NOW_NS=$(python3 -c 'import time; print(int((time.time()+1)*1e9))')
main_tx user0 ibc-transfer transfer transfer "$CH" "$PEER_USER" 77utia \
  --packet-timeout-height 0-0 --packet-timeout-timestamp "$NOW_NS" --absolute-timeouts
settle

# 8. more traffic both ways in the same blocks.
main_tx user0 ibc-transfer transfer transfer "$CH" "$PEER_USER" 11utia
peer_tx user0 ibc-transfer transfer transfer "$PEER_CH" "$USER0" 13utia
settle

kill $RLY_PID 2>/dev/null || true
grep -c 'Successful transaction' "$RH.start.log" || true
echo "workload done (user0=$USER0, relayer=$RLY_ADDR)"
