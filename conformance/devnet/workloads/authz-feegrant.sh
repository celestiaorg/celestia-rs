#!/usr/bin/env bash
# authz + feegrant workload for devnet.sh.
#
# authz: generic, send (spend limit, allow list, expiry) and stake
# (delegate, allow list, max tokens) grants; MsgExec updating, exhausting
# and missing grants; a self-signed MsgExec; a failing nested message;
# revoke; a short-lived grant pruned by the BeginBlocker; MsgPruneExpiredGrants.
#
# feegrant: basic (spend limit used up -> removed), periodic (period reset),
# allowed-msg allowances; revoke (Go keeps the queue entry); a short-lived
# allowance removed by the EndBlocker; MsgPruneAllowances.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

celestia-appd keys add user1 "${K[@]}" >/dev/null 2>&1
celestia-appd keys add user2 "${K[@]}" >/dev/null 2>&1
VAL0=$(addr val0)
USER0=$(addr user0)
USER1=$(addr user1)
USER2=$(addr user2)
VAL0_OPER=$(valoper val0)
D=$HOME_DIR/authz
mkdir -p "$D"

now() { date +%s; }
rfc3339_in() { date -u -d "@$(($(now) + $1))" +%Y-%m-%dT%H:%M:%SZ; }

tx val0 bank send "$VAL0" "$USER1" 50000000utia
tx val0 bank send "$VAL0" "$USER2" 50000000utia

# --- authz ---
tx val0 authz grant "$USER1" generic --msg-type /cosmos.bank.v1beta1.MsgSend
gen "$D/send100.json" bank send "$VAL0" "$USER2" 100utia --from "$VAL0"
tx user1 authz exec "$D/send100.json"
# A send authorization replaces the generic grant (same msg type).
tx val0 authz grant "$USER1" send --spend-limit 1000utia --allow-list "$USER2" \
  --expiration $(($(now) + 3600))
gen "$D/send400.json" bank send "$VAL0" "$USER2" 400utia --from "$VAL0"
tx user1 authz exec "$D/send400.json" # updated to 600utia
gen "$D/send700.json" bank send "$VAL0" "$USER2" 700utia --from "$VAL0"
tx user1 authz exec "$D/send700.json" # over the limit: sdk 5
gen "$D/send600.json" bank send "$VAL0" "$USER2" 600utia --from "$VAL0"
tx user1 authz exec "$D/send600.json" # exhausted: grant deleted
tx user1 authz exec "$D/send100.json" # no grant: authz 2
gen "$D/sendu0.json" bank send "$VAL0" "$USER0" 1utia --from "$VAL0"
tx val0 authz grant "$USER1" send --spend-limit 1000utia --allow-list "$USER2"
tx user1 authz exec "$D/sendu0.json" # not in allow list: sdk 4

tx val0 authz grant "$USER1" delegate --spend-limit 5000utia --allowed-validators "$VAL0_OPER"
gen "$D/del2000.json" staking delegate "$VAL0_OPER" 2000utia --from "$VAL0"
tx user1 authz exec "$D/del2000.json"
tx user0 authz grant "$USER1" unbond --deny-validators "$VAL0_OPER"
tx user1 authz exec "$D/del2000.json"

# Self-signed MsgExec needs no grant; a failing nested message fails the tx.
gen "$D/self.json" bank send "$USER1" "$USER2" 5utia --from "$USER1"
tx user1 authz exec "$D/self.json"
gen "$D/toomuch.json" bank send "$USER1" "$USER2" 999999999999utia --from "$USER1"
tx user1 authz exec "$D/toomuch.json"

tx val0 authz revoke "$USER1" /cosmos.staking.v1beta1.MsgDelegate
tx val0 authz revoke "$USER1" /cosmos.staking.v1beta1.MsgDelegate # authz 2

# Short-lived grants: one pruned by the BeginBlocker, one by MsgPruneExpiredGrants.
tx val0 authz grant "$USER2" generic --msg-type /cosmos.gov.v1.MsgVote --expiration $(($(now) + 12))
tx user0 authz grant "$USER2" generic --msg-type /cosmos.gov.v1.MsgDeposit --expiration $(($(now) + 12))
tx user0 authz grant "$USER2" generic --msg-type /cosmos.gov.v1.MsgVote --expiration $(($(now) + 3600))

# --- feegrant ---
tx val0 feegrant grant "$VAL0" "$USER2" --spend-limit 60000utia --expiration "$(rfc3339_in 3600)"
FEES=20000utia tx user2 bank send "$USER2" "$USER0" 1utia --fee-granter "$VAL0"
FEES=20000utia tx user2 bank send "$USER2" "$USER0" 1utia --fee-granter "$VAL0"
FEES=20000utia tx user2 bank send "$USER2" "$USER0" 1utia --fee-granter "$VAL0" # used up: removed
tx val0 feegrant grant "$VAL0" "$USER1" --spend-limit 200000utia --period 8 --period-limit 40000utia
FEES=30000utia tx user1 bank send "$USER1" "$USER0" 1utia --fee-granter "$VAL0"
sleep 10
FEES=30000utia tx user1 bank send "$USER1" "$USER0" 1utia --fee-granter "$VAL0" # after reset
tx val0 feegrant grant "$VAL0" "$USER0" --allowed-messages /cosmos.bank.v1beta1.MsgSend \
  --spend-limit 100000utia
tx user0 bank send "$USER0" "$USER1" 1utia --fee-granter "$VAL0"
tx val0 feegrant grant "$VAL0" "$USER0" --spend-limit 1utia || true # already exists
# Revoked allowance with an expiry: Go leaves its queue entry behind.
tx user1 feegrant grant "$USER1" "$USER2" --spend-limit 100000utia --expiration "$(rfc3339_in 15)"
tx user1 feegrant revoke "$USER1" "$USER2"
tx user0 feegrant grant "$USER0" "$USER2" --expiration "$(rfc3339_in 15)"
tx user0 feegrant grant "$USER0" "$USER1" --expiration "$(rfc3339_in 3600)"
tx val0 feegrant revoke "$VAL0" "$USER1"

sleep 18
rawtx user2 "[{\"@type\":\"/cosmos.authz.v1beta1.MsgPruneExpiredGrants\",\"pruner\":\"$USER2\"}]"
tx user2 feegrant prune
wait_blocks 3
echo "workload done"
