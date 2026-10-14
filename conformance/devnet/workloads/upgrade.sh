#!/usr/bin/env bash
# Upgrade workload for devnet.sh. Run with CHAIN_ID=test (signal upgrade
# delay of 3 blocks) and ALLOW_HALT=1: the chain halts at the end, exactly
# where Go does.
#
#  1-7  MsgSoftwareUpgrade "v10" at descending candidate heights around the
#       predicted execution height E, all in one tx: candidates below E fail
#       ("in the past"), the rest schedule in order so the plan ends at
#       height E; the next PreBlocker runs the v10 handler (evidence params,
#       fibre module account, migrations) and marks v10 done.
#  8    MsgSoftwareUpgrade "v11" far in the future (scheduled)
#  9    MsgCancelUpgrade (clears it)
#  10   MsgSoftwareUpgrade "v10" again: already completed, proposal fails
#  then MsgSignalVersion 11 + MsgTryUpgrade: at the upgrade height the app
#  EndBlocker schedules "v11", bumps the app version to 11 and resets the
#  tally; the chain then halts (no v11 handler / unsupported version).
set -euo pipefail
. "$(dirname "$0")/lib.sh"

VAL0=$(addr val0)
VAL0_OPER=$(valoper val0)
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7
D=$HOME_DIR/proposals
mkdir -p "$D"

upgrade_msg() { # <name> <height>
  echo "{\"@type\":\"/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade\",\"authority\":\"$GOV\",\"plan\":{\"name\":\"$1\",\"height\":\"$2\",\"info\":\"\"}}"
}

proposal_msg() { # <title> <inner-msg-json>
  echo "{\"@type\":\"/cosmos.gov.v1.MsgSubmitProposal\",\"messages\":[$2],\"initial_deposit\":[{\"denom\":\"utia\",\"amount\":\"10000000\"}],\"proposer\":\"$VAL0\",\"metadata\":\"\",\"title\":\"$1\",\"summary\":\"$1\",\"expedited\":false}"
}

vote_msg() { # <id>
  echo "{\"@type\":\"/cosmos.gov.v1.MsgVote\",\"proposal_id\":\"$1\",\"voter\":\"$VAL0\",\"option\":\"VOTE_OPTION_YES\",\"metadata\":\"\"}"
}

wait_final() { # <id>...
  local id s
  for id in "$@"; do
    for _ in $(seq 1 120); do
      s=$(q gov proposal "$id" 2>/dev/null | json 'd["proposal"]["status"]' 2>/dev/null || echo gone)
      case "$s" in
      *VOTING_PERIOD | *DEPOSIT_PERIOD) sleep 2 ;;
      *) break ;;
      esac
    done
    echo "proposal $id: $s"
  done
}

# Blocks are ~2.61 s apart: a 60 s voting period ends ~23 blocks after the
# submission block (h0 + 1 to h0 + 3).
H0=$(height)
MSGS=""
for k in 6 5 4 3 2 1 0; do
  h=$((H0 + 24 + k))
  MSGS="$MSGS${MSGS:+,}$(proposal_msg "v10 at $h" "$(upgrade_msg v10 $h)")"
done
GAS=3000000 FEES=60000utia rawtx val0 "[$MSGS]"
VOTES=""
for id in 1 2 3 4 5 6 7; do VOTES="$VOTES${VOTES:+,}$(vote_msg $id)"; done
GAS=1000000 FEES=20000utia rawtx val0 "[$VOTES]"
wait_final 1 2 3 4 5 6 7
wait_blocks 2
q upgrade applied v10 || true

GAS=1000000 rawtx val0 "[$(proposal_msg "v11 later" "$(upgrade_msg v11 $(($(height) + 100000)))")]"
tx val0 gov vote 8 yes
wait_final 8
q upgrade plan || true
GAS=1000000 rawtx val0 "[$(proposal_msg "cancel" "{\"@type\":\"/cosmos.upgrade.v1beta1.MsgCancelUpgrade\",\"authority\":\"$GOV\"}")]"
tx val0 gov vote 9 yes
GAS=1000000 rawtx val0 "[$(proposal_msg "v10 again" "$(upgrade_msg v10 $(($(height) + 100000)))")]"
tx val0 gov vote 10 yes
wait_final 9 10

# Signal v11: the upgrade height is 3 blocks after try-upgrade on chain "test".
tx val0 signal signal 11 # celestia-appd tx signal signal <version> --from <validator>
tx val0 signal try-upgrade
# Wait for the halt: the height stops advancing.
last=$(height)
stalled=0
while [ "$stalled" -lt 10 ]; do
  sleep 2
  h=$(height 2>/dev/null || echo "$last")
  if [ "$h" = "$last" ]; then stalled=$((stalled + 1)); else stalled=0; last=$h; fi
done
echo "chain halted at height $last"
echo "workload done"
