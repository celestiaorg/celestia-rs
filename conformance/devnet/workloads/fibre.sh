#!/usr/bin/env bash
# x/fibre + x/valaddr workload for devnet.sh / mixed.sh / rust-proposer.sh
# (genesis patched by fibre.genesis.py: 60 s withdrawal delay, 30 s promise
# timeout, 30 s gov voting).
#
# Payment promises are built by conformance/fibre-client: signed by the
# escrow owner's secp256k1 key (exported from the test keyring) and by the
# validator's consensus key (priv_validator_key.json).
#
#  - valaddr set-host by the validator; by a non-validator (fails)
#  - escrow deposits; withdrawal requests (paid out by the BeginBlocker
#    after the delay), one exceeding the available balance (fails)
#  - MsgPayForFibre: settled; replayed, corrupted validator signature, no
#    escrow, insufficient escrow (CheckTx rejects); one drawing on a
#    pending withdrawal; two promises racing for one escrow (PrepareProposal
#    drops the unsettleable one)
#  - MsgPaymentPromiseTimeout: settled; too early and repeated (fail)
#  - gov MsgUpdateFibreParams: valid (executed), invalid (rejected)
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ROOT=$(cd "$HERE/../../.." && pwd)
FC=$HOME_DIR/fibre-client
(cd "$ROOT/conformance/fibre-client" && GOFLAGS=-mod=mod go build -o "$FC" .)
PV=$HOME_DIR/config/priv_validator_key.json
NS=00000000000000000000000000000000000000f1b2e000000000000000
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7

USER0=$(addr user0)
for k in fibre1 fibre2 fibre3 fibre4 fibre5 proc1 proc2; do
  celestia-appd keys add "$k" "${K[@]}" >/dev/null 2>&1 || true
done
tx user0 bank multi-send "$USER0" $(for k in fibre1 fibre2 fibre3 fibre4 fibre5 proc1 proc2; do addr $k; done) 100000000utia

key() { echo y | celestia-appd keys export "$1" --unarmored-hex --unsafe "${K[@]}" 2>/dev/null; }
commitment() { python3 -c "import hashlib,sys; print(hashlib.sha256(sys.argv[1].encode()).hexdigest())" "$1"; }
now() { python3 -c "import datetime,sys; print((datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=int(sys.argv[1]))).strftime('%Y-%m-%dT%H:%M:%S.%fZ'))" "${1:-0}"; }

# promise <escrow key name> <tag> <blob size> [created offset s] [extra fibre-client flags...]
# prints the fibre-client JSON
promise() {
  local who=$1 tag=$2 size=$3 off=${4:-0}
  shift 3
  [ $# -gt 0 ] && shift
  "$FC" -chain-id "$CHAIN_ID" -height "$(height)" -namespace "$NS" -blob-size "$size" \
    -commitment "$(commitment "$tag")" -created "$(now "$off")" -escrow-key "$(key "$who")" \
    -val-key "$PV" "$@"
}
pjson() { echo "$1" | json 'json.dumps(d["promise"])'; }
psigs() { echo "$1" | json 'd["signatures"]'; }

# send <from> <args...>: broadcast without waiting; prints "<code> <hash>".
send() {
  local from=$1
  shift
  celestia-appd tx "$@" --from "$from" "${K[@]}" --chain-id "$CHAIN_ID" --node "$NODE" \
    --fees 20000utia --gas 400000 -y -o json 2>/dev/null | json 'str(d.get("code", 0)) + " " + d["txhash"]' || echo "err -"
}
# rejected <from> <args...>: expects a CheckTx rejection.
rejected() {
  local r
  r=$(send "$@")
  echo "rejected? $2 $3 -> ${r%% *}"
}
wait_hash() { python3 "$HERE/wait_tx.py" "$RPC" "$1" || echo "tx $1 not included"; }

# valaddr: the validator registers its fibre endpoint; a plain account fails.
tx val0 valaddr set-host 127.0.0.1:7980
tx user0 valaddr set-host 127.0.0.1:7981

# Escrows. fibre4 keeps 200000 available after a pending withdrawal.
tx fibre1 fibre deposit-to-escrow 5000000utia
tx fibre2 fibre deposit-to-escrow 1000000utia
tx fibre4 fibre deposit-to-escrow 1000000utia
tx fibre5 fibre deposit-to-escrow 1000000utia
tx fibre1 fibre request-withdrawal 1000000utia
tx fibre4 fibre request-withdrawal 800000utia
tx fibre1 fibre request-withdrawal 99000000utia

# PayForFibre: settles (695000utia for one 256 KiB chunk).
P=$(promise fibre1 blob-1 1000)
tx proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"
# CheckTx rejections: replay, bad validator signature, no escrow, too large
# for the escrow.
rejected proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"
P=$(promise fibre1 blob-2 1000 0 -corrupt-val-sig)
rejected proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"
P=$(promise fibre3 blob-3 1000)
rejected proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"
P=$(promise fibre2 blob-4 200000000)
rejected proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"
# Larger blob (3 chunks) from fibre1.
P=$(promise fibre1 blob-5 600000)
tx proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"

# Draws 200000 from available and 495000 from fibre4's pending withdrawal.
P=$(promise fibre4 blob-6 1000)
tx proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")"

# Two promises against fibre2 (1000000utia covers one) in the same block:
# both pass CheckTx, PrepareProposal settles the first and drops the second
# (later evicted by recheck).
A=$(promise fibre2 race-a 1000)
B=$(promise fibre2 race-b 1000)
ra=$(send proc1 fibre pay-for-fibre "$(pjson "$A")" "$(psigs "$A")")
rb=$(send proc2 fibre pay-for-fibre "$(pjson "$B")" "$(psigs "$B")")
echo "race: $ra / $rb"
wait_blocks 3

# Timeouts: a promise created 40 s ago (timeout 30 s) is claimable once;
# a fresh one is not yet.
T=$(promise fibre1 timeout-1 1000 -40)
tx proc2 fibre payment-promise-timeout "$(pjson "$T")"
tx proc2 fibre payment-promise-timeout "$(pjson "$T")"
T=$(promise fibre1 timeout-2 1000)
tx proc2 fibre payment-promise-timeout "$(pjson "$T")"

# A timeout debit and a pay-for-fibre against fibre5 (1000000utia) in one
# block: the timeout (a normal tx) settles first in the proposal, so the
# promise can no longer settle and is dropped.
T=$(promise fibre5 timeout-3 1000 -40)
P=$(promise fibre5 blob-7 1000)
rt=$(send proc2 fibre payment-promise-timeout "$(pjson "$T")")
rp=$(send proc1 fibre pay-for-fibre "$(pjson "$P")" "$(psigs "$P")")
echo "timeout+pff: $rt / $rp"
wait_hash "${rt#* }"
wait_blocks 3

# Withdrawals requested above mature 60 s after their request.
sleep 40
wait_blocks 2

# Gov: MsgUpdateFibreParams (valid -> executed; invalid -> submit fails).
D=$HOME_DIR/proposals
mkdir -p "$D"
params() { # <withdrawal_delay> <timeout>
  echo "[{\"@type\":\"/celestia.fibre.v1.MsgUpdateFibreParams\",\"authority\":\"$GOV\",\"params\":{\"withdrawal_delay\":\"$1\",\"payment_promise_timeout\":\"$2\",\"payment_promise_height_window\":\"500\",\"shard_retention\":\"7200s\",\"full_stake_storage_budget\":\"1099511627776\"}}]"
}
python3 - "$D/ok.json" "$(params 50000s 1800s)" <<'PY'
import json, sys
json.dump({"messages": json.loads(sys.argv[2]), "metadata": "ipfs://CID", "deposit": "10000000utia",
           "title": "fibre params", "summary": "fibre params", "expedited": False}, open(sys.argv[1], "w"))
PY
python3 - "$D/bad.json" "$(params 60s 30s)" <<'PY'
import json, sys
json.dump({"messages": json.loads(sys.argv[2]), "metadata": "ipfs://CID", "deposit": "10000000utia",
           "title": "bad fibre params", "summary": "bad", "expedited": False}, open(sys.argv[1], "w"))
PY
tx val0 gov submit-proposal "$D/ok.json"
tx val0 gov vote 1 yes
tx val0 gov submit-proposal "$D/bad.json"
sleep 32
wait_blocks 2
q fibre params || true
