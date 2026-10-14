#!/usr/bin/env bash
# Validator lifecycle workload for devnet.sh: self-unbond below the minimum
# self-delegation (jail + BeginUnbondingValidator), redelegation from an
# unbonding validator, full undelegation (celestia UserOutstandingRewards
# parking and withdrawal without a delegation), a failing and a succeeding
# unjail (back to bonded), an unbonded validator receiving a redelegation
# from a bonded one, and validator removal once its shares reach zero.
#
# Env (from devnet.sh): HOME_DIR, CHAIN_ID, NODE; celestia-appd on PATH.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
RPC=${NODE/tcp:/http:}
K=(--keyring-backend test --home "$HOME_DIR")
celestia-appd keys add user1 "${K[@]}" >/dev/null 2>&1
VAL0=$(celestia-appd keys show val0 -a "${K[@]}")
VAL0_OPER=$(celestia-appd keys show val0 --bech val -a "${K[@]}")
USER0_OPER=$(celestia-appd keys show user0 --bech val -a "${K[@]}")
USER1=$(celestia-appd keys show user1 -a "${K[@]}")
USER1_OPER=$(celestia-appd keys show user1 --bech val -a "${K[@]}")

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

# tx <from> <args...>: broadcast and wait for inclusion.
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
  echo "$* -> $res"
}

validator_json() { # <file> <moniker> <amount> <min-self>
  local pk
  pk=$(python3 -c "import base64,os; print(base64.b64encode(os.urandom(32)).decode())")
  cat > "$1" <<JSON
{
  "pubkey": {"@type": "/cosmos.crypto.ed25519.PubKey", "key": "$pk"},
  "amount": "$3",
  "moniker": "$2",
  "commission-rate": "0.10",
  "commission-max-rate": "0.20",
  "commission-max-change-rate": "0.01",
  "min-self-delegation": "$4"
}
JSON
}

validator_json "$HOME_DIR/val1.json" val1 2000000utia 1500000
validator_json "$HOME_DIR/val2.json" val2 500000utia 1

tx val0 bank send "$VAL0" "$USER1" 10000000utia
tx user0 staking create-validator "$HOME_DIR/val1.json"
tx val0 staking delegate "$USER0_OPER" 1000000utia
# Self-delegation drops below the minimum: val1 is jailed and starts
# unbonding at the end of the block.
tx user0 staking unbond "$USER0_OPER" 1000000utia
tx val0 staking redelegate "$USER0_OPER" "$VAL0_OPER" 500000utia
# Removes val0's delegation to val1 (rewards parked, then withdrawn).
tx val0 staking unbond "$USER0_OPER" 500000utia
tx val0 distribution withdraw-rewards "$USER0_OPER"
tx user0 distribution withdraw-rewards "$USER0_OPER"
tx user0 slashing unjail
tx user0 staking delegate "$USER0_OPER" 600000utia
tx user0 slashing unjail
# val2 stays unbonded (power 0) and receives a redelegation from val0.
tx user1 staking create-validator "$HOME_DIR/val2.json"
tx val0 staking redelegate "$VAL0_OPER" "$USER1_OPER" 200000utia
tx user1 staking unbond "$USER1_OPER" 500000utia
# Last shares leave the unbonded val2: it is removed.
tx val0 staking unbond "$USER1_OPER" 200000utia
echo "workload done"
