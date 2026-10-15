#!/usr/bin/env bash
# App-version-9 specific behavior (ADR 0004). Run with APP_V9=1:
#
#   APP_V9=1 conformance/devnet/devnet.sh v9-exec 30 conformance/devnet/workloads/v9-exec.sh
#
#  - Execution order: two txs of one account in one block, the first sending
#    almost everything, the second paying its fee. v9 (cosmos-sdk v0.52.8)
#    delivers them one after the other: the second fails in the ante
#    handler (insufficient funds). v10 would run both fees first.
#  - Fees in several denoms: accepted by v9's fee checker (v10 rejects any
#    non-utia denom), from a payer without the second denom (CheckTx
#    fails: insufficient funds) and from one holding it (included).
#    (A tx repeating a TxRaw field is not sent: v9 CheckTx answers it with an
#    error, which stops celestia-core's socket client and the node.)
#  - authz MsgExec wrapping MsgSubmitProposal: v9's MsgExecDecorator allows
#    it (v10's NestedMsgDecorator rejects it).
set -euo pipefail
. "$(dirname "$0")/lib.sh"

VAL0=$(addr val0)
USER0=$(addr user0)
celestia-appd keys add drain "${K[@]}" >/dev/null 2>&1
DRAIN=$(addr drain)
D=$HOME_DIR/v9exec
mkdir -p "$D"

tx val0 bank send "$VAL0" "$DRAIN" 1000000utia

# signed <file> <sequence> <args...>: an offline-signed tx from drain.
signed() {
  local f=$1 seq=$2 acc
  shift 2
  acc=$(q auth account "$DRAIN" | json 'd.get("account",d)["value"]["account_number"] if "value" in d.get("account",d) else d.get("account",d)["account_number"]')
  celestia-appd tx "$@" --from drain "${K[@]}" --chain-id "$CHAIN_ID" --fees 20000utia \
    --gas 100000 --generate-only -o json >"$D/$f.json"
  celestia-appd tx sign "$D/$f.json" --from drain "${K[@]}" --chain-id "$CHAIN_ID" \
    --offline --account-number "$acc" --sequence "$seq" -o json >"$D/$f.signed.json"
  celestia-appd tx encode "$D/$f.signed.json"
}
broadcast() { # <base64 tx>
  curl -s "$RPC" -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"broadcast_tx_sync\",\"params\":{\"tx\":\"$1\"}}"
  echo
}

# Same block, sequential semantics: the second tx cannot pay its fee.
T1=$(signed t1 0 bank send "$DRAIN" "$VAL0" 980000utia)
T2=$(signed t2 1 bank send "$DRAIN" "$VAL0" 1utia)
wait_blocks 1
broadcast "$T1"
broadcast "$T2"
wait_blocks 3

# Fees in two denoms: the payer holds no uatom (CheckTx fails), then one
# that does (priority = the lowest per-denom gas price).
FEES=20000utia,1uatom tx val0 bank send "$VAL0" "$USER0" 1utia || echo "multi-denom fee rejected"
FEES=20000utia,7uatom tx multi bank send "$(addr multi)" "$USER0" 1utia

# authz: user0 executes a MsgSubmitProposal on behalf of val0.
tx val0 authz grant "$USER0" generic --msg-type /cosmos.gov.v1.MsgSubmitProposal
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7
SEND="{\"@type\":\"/cosmos.bank.v1beta1.MsgSend\",\"from_address\":\"$GOV\",\"to_address\":\"$VAL0\",\"amount\":[{\"denom\":\"utia\",\"amount\":\"1\"}]}"
PROP="{\"@type\":\"/cosmos.gov.v1.MsgSubmitProposal\",\"messages\":[$SEND],\"initial_deposit\":[{\"denom\":\"utia\",\"amount\":\"1000\"}],\"proposer\":\"$VAL0\",\"metadata\":\"\",\"title\":\"exec\",\"summary\":\"exec\",\"expedited\":false}"
GAS=400000 rawtx user0 "[{\"@type\":\"/cosmos.authz.v1beta1.MsgExec\",\"grantee\":\"$USER0\",\"msgs\":[$PROP]}]" || echo "exec rejected"
wait_blocks 2
echo "workload done"
