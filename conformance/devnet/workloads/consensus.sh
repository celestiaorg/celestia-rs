#!/usr/bin/env bash
# x/consensus MsgUpdateParams through gov (genesis: consensus.genesis.py):
#  1 block.max_bytes change + appconsts evidence params -> passed, executed
#  2 block.max_gas change -> passed, fails on execution (MaxGas immutable)
#  3 abci vote_extensions_enable_height in the past -> fails on execution
#  4 evidence params differing from appconsts -> rejected by the ante param
#    filter (CheckTx; never included)
set -euo pipefail
. "$(dirname "$0")/lib.sh"

VAL0=$(addr val0)
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7

cmsg() { # <max_bytes> <max_gas> <evidence_blocks> [abci-json]
  local abci=${4:+,\"abci\":$4}
  echo "{\"@type\":\"/cosmos.consensus.v1.MsgUpdateParams\",\"authority\":\"$GOV\",\"block\":{\"max_bytes\":\"$1\",\"max_gas\":\"$2\"},\"evidence\":{\"max_age_num_blocks\":\"$3\",\"max_age_duration\":\"1213200s\",\"max_bytes\":\"1048576\"},\"validator\":{\"pub_key_types\":[\"ed25519\"]}$abci}"
}
proposal_msg() { # <title> <inner-msg-json>
  echo "{\"@type\":\"/cosmos.gov.v1.MsgSubmitProposal\",\"messages\":[$2],\"initial_deposit\":[{\"denom\":\"utia\",\"amount\":\"10000000\"}],\"proposer\":\"$VAL0\",\"metadata\":\"\",\"title\":\"$1\",\"summary\":\"$1\",\"expedited\":false}"
}
vote_msg() {
  echo "{\"@type\":\"/cosmos.gov.v1.MsgVote\",\"proposal_id\":\"$1\",\"voter\":\"$VAL0\",\"option\":\"VOTE_OPTION_YES\",\"metadata\":\"\"}"
}

P1=$(proposal_msg "block size" "$(cmsg 16777216 -1 404400)")
P2=$(proposal_msg "max gas" "$(cmsg 16777216 5 404400)")
P3=$(proposal_msg "vote extensions" "$(cmsg 16777216 -1 404400 '{"vote_extensions_enable_height":"2"}')")
GAS=2000000 FEES=40000utia rawtx val0 "[$P1,$P2,$P3]"
GAS=1000000 rawtx val0 "[$(vote_msg 1),$(vote_msg 2),$(vote_msg 3)]"
GAS=1000000 rawtx val0 "[$(proposal_msg "bad evidence" "$(cmsg 16777216 -1 100)")]" || true

for id in 1 2 3; do
  for _ in $(seq 1 60); do
    s=$(q gov proposal "$id" | json 'd["proposal"]["status"]')
    case "$s" in *VOTING_PERIOD) sleep 2 ;; *) break ;; esac
  done
  echo "proposal $id: $s"
done
q consensus params || true
wait_blocks 3
echo "workload done"
