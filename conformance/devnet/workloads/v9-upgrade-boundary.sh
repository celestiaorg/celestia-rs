#!/usr/bin/env bash
# v9 -> v10 upgrade with state that crosses the boundary, on a live devnet
# with the real binary switch. Run with APP_V9=1 UPGRADE_TO_V10=1
# CHAIN_ID=test (signal upgrade delay 3):
#
#   APP_V9=1 UPGRADE_TO_V10=1 CHAIN_ID=test conformance/devnet/devnet.sh \
#     v9-upgrade-boundary 90 conformance/devnet/workloads/v9-upgrade-boundary.sh
#
# Under v9 (v9.0.8 CLI): an authz grant and a fee allowance from val0 to
# user0, an authz grant for a v10-only message (fails at v9), a
# continuous vesting account, a gov proposal of a staking MsgUpdateParams
# (90 s voting: it executes at v10), a
# try-upgrade before any signal (no quorum), MsgSignalVersion 10,
# MsgTryUpgrade twice (the second fails: upgrade pending). After the switch
# (v10.4.0 CLI): the grant and the allowance used, a fibre escrow deposit
# and withdrawal request right after the store is mounted, the valaddr host,
# the vesting account spending, the signal tally queried.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

V10=$(dirname "$(dirname "$(command -v celestia-appd)")")/celestia-appd
VAL0=$(addr val0)
USER0=$(addr user0)
GOV=celestia10d07y265gmmuvt4z0w9aw880jnsr700jtgz4v7

app_version() { curl -s "$RPC/abci_info" | json 'd["result"]["response"].get("app_version","0")'; }

# --- app version 9 ---
tx val0 authz grant "$USER0" send --spend-limit 5000000utia
tx val0 feegrant grant "$VAL0" "$USER0" --spend-limit 3000000utia
GAS=400000 rawtx val0 "[{\"@type\":\"/cosmos.authz.v1beta1.MsgGrant\",\"granter\":\"$VAL0\",\"grantee\":\"$USER0\",\"grant\":{\"authorization\":{\"@type\":\"/cosmos.authz.v1beta1.GenericAuthorization\",\"msg\":\"/celestia.fibre.v1.MsgDepositToEscrow\"},\"expiration\":null}}]" || true
celestia-appd keys add vest0 "${K[@]}" >/dev/null 2>&1
VEST0=$(addr vest0)
END=$(($(date +%s) + 90))
tx val0 vesting create-vesting-account "$VEST0" 200000000utia "$END" --start-time "$(date +%s)"
GAS=1000000 rawtx val0 "[{\"@type\":\"/cosmos.gov.v1.MsgSubmitProposal\",\"messages\":[{\"@type\":\"/cosmos.staking.v1beta1.MsgUpdateParams\",\"authority\":\"$GOV\",\"params\":{\"unbonding_time\":\"1213200s\",\"max_validators\":100,\"max_entries\":8,\"historical_entries\":10000,\"bond_denom\":\"utia\",\"min_commission_rate\":\"0.050000000000000000\"}}],\"initial_deposit\":[{\"denom\":\"utia\",\"amount\":\"10000000\"}],\"proposer\":\"$VAL0\",\"metadata\":\"\",\"title\":\"v9\",\"summary\":\"v9\",\"expedited\":false}]"
tx val0 gov vote 1 yes
tx user0 signal try-upgrade
echo "app version before: $(app_version)"
tx val0 signal signal 10
tx val0 signal try-upgrade
tx user0 signal try-upgrade
until [ "$(app_version)" = "10" ]; do sleep 0.5; done
echo "app version 10 at height $(height)"
# Until the v10 app's first commit its check state has an empty header
# (height 0): CheckTx verifies signatures with account number 0
# (security.md 70).
wait_blocks 2

# --- app version 10 (v10.4.0 CLI) ---
v10tx() { # <from> <args...>
  local from=$1
  shift
  PATH=$(dirname "$V10"):$PATH tx "$from" "$@"
}
v10tx user0 fibre deposit-to-escrow 1000000utia
v10tx user0 fibre request-withdrawal 1000utia
v10tx val0 valaddr set-host fibre.example:7980 || true
v10tx user0 bank send "$USER0" "$VAL0" 10utia --fee-granter "$VAL0"
"$V10" tx bank send "$VAL0" "$USER0" 1000utia --from val0 "${K[@]}" --chain-id "$CHAIN_ID" \
  --generate-only -o json >"$HOME_DIR/exec.json"
v10tx user0 authz exec "$HOME_DIR/exec.json"
v10tx val0 signal signal 10 || true
"$V10" query signal tally 10 --node "$NODE" -o json || true
until [ "$(date +%s)" -gt "$END" ]; do sleep 1; done
v10tx vest0 bank send "$VEST0" "$USER0" 1000utia --fees 20000utia || true
for _ in $(seq 1 60); do
  s=$("$V10" query gov proposal 1 --node "$NODE" -o json 2>/dev/null | json 'd["proposal"]["status"]' || echo gone)
  case "$s" in *VOTING_PERIOD) sleep 2 ;; *) break ;; esac
done
echo "proposal 1: $s"
echo "workload done"
