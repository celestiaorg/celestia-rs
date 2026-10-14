#!/usr/bin/env bash
# Hyperlane + warp workload for devnet.sh: noop ISM, noop/merkle/IGP hooks,
# a mailbox, a collateral (utia) warp token with a remote router, an
# outbound remote transfer (merkle tree insert + IGP payment), an inbound
# message processed through the noop ISM (collateral release), an IGP claim
# and a few failing txs (wrong owner, unknown ISM).
#
# Ids are deterministic (router sequences start at 0):
#   noop ISM      router_ism           type 0 seq 0
#   noop hook     router_post_dispatch type 0 seq 0
#   merkle hook   router_post_dispatch type 3 seq 1
#   IGP           router_post_dispatch type 4 seq 2
#   mailbox       hyperlane            type 0 seq 0
#   token         router_app           type 1 seq 0
set -euo pipefail

A="celestia-appd --home $HOME_DIR"
TX="--from user0 --keyring-backend test --chain-id $CHAIN_ID --node $NODE --gas 400000 --fees 4000utia --yes --output json"
TX_VAL="--from val0 --keyring-backend test --chain-id $CHAIN_ID --node $NODE --gas 400000 --fees 4000utia --yes --output json"

id() { # <name> <type> <seq>
  python3 -c 'import sys; n=sys.argv[1].encode().ljust(20, b"\0"); print("0x" + (n + int(sys.argv[2]).to_bytes(4, "big") + int(sys.argv[3]).to_bytes(8, "big")).hex())' "$@"
}
ISM=$(id router_ism 0 0)
NOOP_HOOK=$(id router_post_dispatch 0 0)
MERKLE=$(id router_post_dispatch 3 1)
IGP=$(id router_post_dispatch 4 2)
MAILBOX=$(id hyperlane 0 0)
TOKEN=$(id router_app 1 0)
LOCAL_DOMAIN=69420
REMOTE_DOMAIN=1
REMOTE_ROUTER=0x000000000000000000000000ee00000000000000000000000000000000000001

height() { curl -s "${NODE/tcp:/http:}/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }

run() { # tx, then wait until it is committed so the next tx gets a new sequence
  local out code hash
  out=$($A tx "$@")
  code=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1]).get("code", 0))' "$out")
  hash=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["txhash"])' "$out")
  echo "tx $1 $2: checktx code=$code" >&2
  [ "$code" = 0 ] || return 0 # rejected by CheckTx: never included
  # Accepted into the mempool: it is included in the next block. Wait for
  # two more blocks (the tx indexer may be disabled).
  local h0
  h0=$(height)
  until [ "$(height)" -ge $((h0 + 2)) ]; do sleep 1; done
  echo "tx $hash included by height $((h0 + 2))" >&2
}

run hyperlane ism create-noop $TX
run hyperlane hooks noop create $TX
run hyperlane mailbox create "$ISM" $LOCAL_DOMAIN $TX
run hyperlane hooks merkle create "$MAILBOX" $TX
run hyperlane hooks igp create utia $TX
run hyperlane hooks igp set-destination-gas-config "$IGP" $REMOTE_DOMAIN 10000000000 1 100 $TX
run hyperlane mailbox set "$MAILBOX" --required-hook "$MERKLE" --default-hook "$IGP" $TX
# Fails: val0 does not own the mailbox.
run hyperlane mailbox set "$MAILBOX" --default-hook "$NOOP_HOOK" $TX_VAL
# Fails: unknown ISM.
run hyperlane mailbox create "$(id router_ism 5 77)" $LOCAL_DOMAIN $TX

run warp create-collateral-token "$MAILBOX" utia $TX
run warp enroll-remote-router "$TOKEN" $REMOTE_DOMAIN "$REMOTE_ROUTER" 50000 $TX
run warp transfer "$TOKEN" $REMOTE_DOMAIN 0x0000000000000000000000000000000000000000000000000000000000000abc 1000000 --max-hyperlane-fee 100000utia $TX

# Inbound: REMOTE_ROUTER on domain 1 sends 250000utia to user0.
USER_HEX=$($A keys show user0 -a --keyring-backend test | xargs $A debug addr | sed -n 's/^Address (hex): //p' | tr 'A-F' 'a-f')
BODY=$(printf '%024x%s%064x' 0 "$USER_HEX" 250000)
MSG=0x03$(printf '%08x%08x' 0 $REMOTE_DOMAIN)${REMOTE_ROUTER#0x}$(printf '%08x' $LOCAL_DOMAIN)${TOKEN#0x}$BODY
run hyperlane mailbox process "$MAILBOX" 0x "$MSG" $TX
# Replay of the same message fails.
run hyperlane mailbox process "$MAILBOX" 0x "$MSG" $TX

run hyperlane hooks igp claim "$IGP" $TX
