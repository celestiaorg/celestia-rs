#!/usr/bin/env bash
# Gov workload for devnet.sh (genesis patched by gov.genesis.py: 10 TIA min
# deposit, 50 TIA expedited, 60 s deposit / 90 s voting / 45 s expedited).
#
# Proposals (ids in submission order):
#  1 blob params (metadata JSON)  yes            -> passed
#  2 blob MsgUpdateBlobParams      yes            -> passed, executed
#  3 MsgSoftwareUpgrade            yes            -> failed (circuit breaker)
#  4 text                          no_with_veto   -> rejected, deposit burned
#  5 text                          no             -> rejected, refunded
#  6 text                          (no votes)     -> rejected (quorum)
#  7 text, half deposit + deposit  weighted/abstain -> passed (deductions)
#  8 text, half deposit            -              -> dropped, refunded
#  9 text by user0                 yes, cancelled -> deleted, 50% burned
# 10 expedited text                yes            -> passed early
# 11 expedited text                no             -> converted, then rejected
# 12 gov MsgUpdateParams           yes            -> passed, executed
# "text" proposals carry a blob MsgUpdateBlobParams (the param filter
# rejects proposals without messages); plus failing messages: inactive vote, bad deposit denom, non-gov signer,
# legacy content (v1beta1 and MsgExecLegacyContent), empty title, wrong
# canceller, weights not summing to one.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

VAL0=$(addr val0)
USER0=$(addr user0)
VAL0_OPER=$(valoper val0)
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7
D=$HOME_DIR/proposals
mkdir -p "$D"

# proposal <file> <title> <deposit> [expedited] [messages-json] [metadata]
proposal() {
  python3 - "$@" <<'PY'
import json, sys
f, title, deposit = sys.argv[1:4]
expedited = len(sys.argv) > 4 and sys.argv[4] == "true"
msgs = json.loads(sys.argv[5]) if len(sys.argv) > 5 and sys.argv[5] else []
metadata = sys.argv[6] if len(sys.argv) > 6 else "ipfs://CID"
json.dump({"messages": msgs, "metadata": metadata, "deposit": deposit,
           "title": title, "summary": title + " summary", "expedited": expedited},
          open(f, "w"))
PY
}

# A gov-signed message for "text" proposals (the ante handler's param
# filter rejects proposals without messages).
blob() { # <gas_per_blob_byte> <gov_max_square_size>
  echo "[{\"@type\":\"/celestia.blob.v1.MsgUpdateBlobParams\",\"authority\":\"$GOV\",\"params\":{\"gas_per_blob_byte\":$1,\"gov_max_square_size\":\"$2\"}}]"
}

submit() { # <from> <file>
  tx "$1" gov submit-proposal "$2"
}

tx user0 staking delegate "$VAL0_OPER" 1000000000utia

proposal "$D/1.json" "text one" 10000000utia false "$(blob 9 64)" '{"title":"text one","summary":"text one summary","authors":["a"]}'
submit val0 "$D/1.json"
tx val0 gov vote 1 yes

proposal "$D/2.json" "blob params" 10000000utia false \
  "[{\"@type\":\"/celestia.blob.v1.MsgUpdateBlobParams\",\"authority\":\"$GOV\",\"params\":{\"gas_per_blob_byte\":9,\"gov_max_square_size\":\"32\"}}]"
submit val0 "$D/2.json"
tx val0 gov vote 2 yes

proposal "$D/3.json" "upgrade" 10000000utia false \
  "[{\"@type\":\"/cosmos.upgrade.v1beta1.MsgSoftwareUpgrade\",\"authority\":\"$GOV\",\"plan\":{\"name\":\"v11\",\"height\":\"100000\",\"info\":\"\"}}]"
submit val0 "$D/3.json"
tx val0 gov vote 3 yes

proposal "$D/4.json" "veto" 10000000utia false "$(blob 12 64)"
submit val0 "$D/4.json"
tx val0 gov vote 4 no_with_veto

proposal "$D/5.json" "no" 10000000utia false "$(blob 12 64)"
submit user0 "$D/5.json"
tx val0 gov vote 5 no

proposal "$D/6.json" "quorum" 10000000utia false "$(blob 12 64)"
submit val0 "$D/6.json"

proposal "$D/7.json" "two deposits" 5000000utia false "$(blob 10 64)"
submit val0 "$D/7.json"
tx user0 gov deposit 7 5000000utia
tx user0 gov weighted-vote 7 yes=0.6,no=0.4
tx val0 gov vote 7 abstain

proposal "$D/8.json" "never funded" 5000000utia false "$(blob 12 64)"
submit val0 "$D/8.json"
tx val0 gov vote 8 yes # inactive proposal: gov 3

proposal "$D/9.json" "cancelled" 10000000utia false "$(blob 12 64)"
submit user0 "$D/9.json"
tx val0 gov vote 9 yes
tx val0 gov cancel-proposal 9 # not the proposer: gov 18
tx user0 gov cancel-proposal 9

proposal "$D/10.json" "expedited yes" 50000000utia true "$(blob 11 64)"
submit val0 "$D/10.json"
tx val0 gov vote 10 yes

proposal "$D/11.json" "expedited no" 50000000utia true "$(blob 13 64)"
submit val0 "$D/11.json"
tx val0 gov vote 11 no

PARAMS=$(q gov params | python3 -c "
import json,sys
p=json.load(sys.stdin)['params']
p['burn_vote_quorum']=True
print(json.dumps(p))")
proposal "$D/12.json" "gov params" 10000000utia false \
  "[{\"@type\":\"/cosmos.gov.v1.MsgUpdateParams\",\"authority\":\"$GOV\",\"params\":$PARAMS}]"
submit val0 "$D/12.json"
tx val0 gov vote 12 yes

# Failing messages (included with a non-zero code).
tx val0 gov deposit 1 1000ufoo || true # gov 23 (denom); may fail in CheckTx
proposal "$D/f1.json" "bad signer" 10000000utia false \
  "[{\"@type\":\"/cosmos.bank.v1beta1.MsgSend\",\"from_address\":\"$USER0\",\"to_address\":\"$VAL0\",\"amount\":[{\"denom\":\"utia\",\"amount\":\"1\"}]}]"
submit user0 "$D/f1.json" # gov 13
tx val0 gov submit-legacy-proposal --title legacy --description legacy --type Text \
  --deposit 10000000utia || true # gov 9
proposal "$D/f2.json" "exec legacy" 10000000utia false \
  "[{\"@type\":\"/cosmos.gov.v1.MsgExecLegacyContent\",\"authority\":\"$GOV\",\"content\":{\"@type\":\"/cosmos.gov.v1beta1.TextProposal\",\"title\":\"t\",\"description\":\"d\"}}]"
submit val0 "$D/f2.json" # gov 9
proposal "$D/f3.json" "" 10000000utia false "$(blob 14 64)"
submit val0 "$D/f3.json" || true # empty title: sdk 18
tx val0 gov weighted-vote 12 yes=0.5,no=0.4 || true # gov 7

# Wait until every proposal left the voting/deposit period.
status() { q gov proposal "$1" 2>/dev/null | json 'd["proposal"]["status"]' 2>/dev/null || echo gone; }
for id in 1 2 3 4 5 6 7 8 9 10 11 12; do
  for _ in $(seq 1 120); do
    s=$(status "$id")
    case "$s" in
    *VOTING_PERIOD | *DEPOSIT_PERIOD) sleep 2 ;;
    *) break ;;
    esac
  done
  echo "proposal $id: $(status "$id")"
done
wait_blocks 2
echo "workload done"
