#!/usr/bin/env bash
# Deterministic current-release governance state for issue #188.
#
# Proposal 1 is in voting with two differently shaped votes and structured
# metadata. Proposal 2 remains in deposit period with two depositors. Proposal
# 3 is expedited and in voting. Every proposal contains a protobuf Any so the
# four-transport capture covers both binary Any bytes and grpc-gateway JSON.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

VAL0=$(addr val0)
USER0=$(addr user0)
VAL0_OPER=$(valoper val0)
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7
DIR=$HOME_DIR/populated-governance
mkdir -p "$DIR"

# This workload enables retained ABCI transaction results. Refuse to continue
# if a transaction was included but failed DeliverTx; merely observing its
# hash in a block is not evidence that it populated the intended state.
tx_success() {
  local output result height code
  output=$(tx "$@")
  echo "$output"
  result=${output##* -> }
  read -r height code _ <<<"$result"
  if [[ ! $height =~ ^[0-9]+$ || $code != 0 ]]; then
    echo "deliver tx failed or result unavailable: $output" >&2
    return 1
  fi
}

proposal() { # <file> <title> <deposit> <expedited> <gas-per-byte> <metadata>
  python3 - "$@" "$GOV" <<'PY'
import json, sys
path,title,deposit,expedited,gas,metadata,authority=sys.argv[1:]
message={"@type":"/celestia.blob.v1.MsgUpdateBlobParams",
         "authority":authority,
         "params":{"gas_per_blob_byte":int(gas),"gov_max_square_size":"64"}}
json.dump({"messages":[message],"metadata":metadata,"deposit":deposit,
           "title":title,"summary":title+" summary",
           "expedited":expedited=="true"},open(path,"w"))
PY
}

tx_success user0 staking delegate "$VAL0_OPER" 1000000000utia

proposal "$DIR/1.json" "populated voting" 10000000utia false 9 \
  '{"title":"populated voting","summary":"populated voting summary","authors":["alice","bob"]}'
tx_success val0 gov submit-proposal "$DIR/1.json"
tx_success val0 gov vote 1 yes
tx_success user0 gov weighted-vote 1 yes=0.6,no=0.3,abstain=0.1

proposal "$DIR/2.json" "populated deposits" 5000000utia false 10 ipfs://deposit
tx_success val0 gov submit-proposal "$DIR/2.json"
tx_success user0 gov deposit 2 2000000utia

proposal "$DIR/3.json" "populated expedited" 50000000utia true 11 ipfs://expedited
tx_success user0 gov submit-proposal "$DIR/3.json"
tx_success val0 gov vote 3 no

wait_blocks 3
echo "populated governance workload done: proposer=$VAL0 secondary=$USER0"
