#!/usr/bin/env bash
# Invalid UTF-8 workload for devnet.sh (genesis patched by utf8.genesis.py:
# the gov workload's short periods and small deposits).
#
# Go strings are bytes: gogoproto decodes a protobuf `string` without a
# UTF-8 check and the state machine stores, emits and logs the bytes. The
# txs come from `fuzz-diff craft` (crates/fuzz-diff/src/craft.rs; signed
# with this devnet's test keyring, SIGN_MODE_DIRECT or
# SIGN_MODE_LEGACY_AMINO_JSON) because the CLI round-trips strings through
# JSON, which replaces invalid UTF-8:
#   validator description fields (incl. a 70-byte moniker with 30 invalid
#   bytes and a too long details field), memos, gov v1 proposals (title,
#   summary, metadata; a JSON metadata whose decoded title differs), votes
#   and weighted votes with metadata, a v1beta1 text proposal, authz grants
#   and MsgExec of an edit-validator and a vote, a revoke of an invalid
#   type URL, a feegrant with invalid allowed messages, two amino-JSON
#   signed txs, and proposals that execute (or fail to execute) messages
#   with invalid strings.
# The run lasts until every proposal has finished its voting period.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ROOT=$(cd "$HERE/../../.." && pwd)
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
[ -x "$FUZZ_DIFF" ] || (cd "$ROOT" && cargo build -q --release -p fuzz-diff)

account() { # <address> -> "<account number> <sequence>"
  q auth account "$1" | python3 -c '
import json, sys
d = json.load(sys.stdin)["account"]
while "base_account" in d or "base_vesting_account" in d:
    d = d.get("base_account") or d["base_vesting_account"]["base_account"]
d = d.get("value", d)
print(int(d.get("account_number", 0)), int(d.get("sequence", 0)))'
}

VAL0=$(addr val0)
USER0=$(addr user0)
tx user0 staking delegate "$(valoper val0)" 1000000000utia

"$FUZZ_DIFF" craft "$HOME_DIR" list > "$HOME_DIR/utf8-cases.txt"
while read -r i signer what <&3; do
  a=$VAL0
  [ "$signer" = user0 ] && a=$USER0
  read -r num seq < <(account "$a")
  raw=$("$FUZZ_DIFF" craft "$HOME_DIR" "$CHAIN_ID" "$i" "$num" "$seq")
  out=$(curl -s "$RPC/broadcast_tx_sync?tx=0x$raw")
  code=$(echo "$out" | json 'd["result"]["code"]')
  if [ "$code" != "0" ]; then
    echo "checktx failed: case $i ($what): $out" >&2
    exit 1
  fi
  hash=$(echo "$out" | json 'd["result"]["hash"]')
  res=$(python3 "$HERE/wait_tx.py" "$RPC" "$hash") || {
    echo "tx $hash not included" >&2
    exit 1
  }
  echo "case $i ($what) -> $res"
done 3< "$HOME_DIR/utf8-cases.txt"

# Let the proposals finish (90 s voting period) and execute.
until [ "$(q gov proposals | python3 -c 'import json,sys; print(sum(p["status"] in ("PROPOSAL_STATUS_VOTING_PERIOD","PROPOSAL_STATUS_DEPOSIT_PERIOD") for p in json.load(sys.stdin).get("proposals") or []))')" = 0 ]; do
  sleep 2
done
q gov proposals | python3 -c '
import json, sys
for p in json.load(sys.stdin).get("proposals") or []:
    print("proposal", p["id"], p["status"], repr(p.get("title")), (p.get("failed_reason") or "")[:120])'
wait_blocks 2
