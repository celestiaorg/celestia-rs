#!/usr/bin/env bash
# Staking / distribution / slashing workload for devnet.sh.
#
# Exercises: create-validator, delegate, edit-validator, redelegate,
# unbond, cancel-unbond, withdraw rewards (+commission), set withdraw
# address, fund community pool, and a failing unjail.
#
# Env (from devnet.sh): HOME_DIR, CHAIN_ID, NODE; celestia-appd on PATH.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
RPC=${NODE/tcp:/http:}
K=(--keyring-backend test --home "$HOME_DIR")
VAL0=$(celestia-appd keys show val0 -a "${K[@]}")
VAL0_OPER=$(celestia-appd keys show val0 --bech val -a "${K[@]}")
USER0=$(celestia-appd keys show user0 -a "${K[@]}")
USER0_OPER=$(celestia-appd keys show user0 --bech val -a "${K[@]}")

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

LAST_HEIGHT=0
# tx <from> <args...>: broadcast and wait for inclusion; records the height.
tx() {
  local from=$1
  shift
  local out hash res
  out=$(celestia-appd tx "$@" --from "$from" "${K[@]}" --chain-id "$CHAIN_ID" \
    --node "$NODE" --fees 20000utia --gas 400000 -y -o json)
  if [ "$(echo "$out" | json 'd.get("code", 0)')" != "0" ]; then
    echo "checktx failed: $out" >&2
    return 1
  fi
  hash=$(echo "$out" | json 'd["txhash"]')
  res=$(python3 "$HERE/wait_tx.py" "$RPC" "$hash") || {
    echo "tx $hash not included" >&2
    return 1
  }
  LAST_HEIGHT=${res%% *}
  echo "$* -> $res"
}

# A second validator operated by user0 with a fresh consensus key; small
# power so val0 alone keeps > 2/3 of the voting power.
PK=$(python3 -c "import base64,os; print(base64.b64encode(os.urandom(32)).decode())")
cat > "$HOME_DIR/val1.json" <<JSON
{
  "pubkey": {"@type": "/cosmos.crypto.ed25519.PubKey", "key": "$PK"},
  "amount": "2000000utia",
  "moniker": "val1",
  "commission-rate": "0.10",
  "commission-max-rate": "0.20",
  "commission-max-change-rate": "0.01",
  "min-self-delegation": "1"
}
JSON

tx user0 staking delegate "$VAL0_OPER" 100000000utia
tx user0 staking create-validator "$HOME_DIR/val1.json"
tx val0 staking edit-validator --new-moniker val0-edited --details "edited" --min-self-delegation 2
tx user0 staking redelegate "$VAL0_OPER" "$USER0_OPER" 50000000utia
tx user0 staking unbond "$VAL0_OPER" 10000000utia
UNBOND_HEIGHT=$LAST_HEIGHT
tx user0 staking cancel-unbond "$VAL0_OPER" 4000000utia "$UNBOND_HEIGHT"
tx user0 distribution withdraw-rewards "$VAL0_OPER"
tx val0 distribution withdraw-rewards "$VAL0_OPER" --commission
tx user0 distribution set-withdraw-addr "$VAL0"
tx user0 distribution fund-community-pool 1000utia
tx user0 staking delegate "$USER0_OPER" 3000000utia
tx user0 distribution withdraw-rewards "$USER0_OPER"
tx val0 slashing unjail
echo "workload done (user0=$USER0)"
