#!/usr/bin/env bash
# Vesting workload for devnet.sh (accounts from vesting.setup.sh and
# vesting.genesis.py: vc continuous, vd delayed, vp periodic, vl permanently
# locked). Exercises locked/spendable coins, delegation tracking, celestia's
# reward UpdateSchedule, MsgCreateVestingAccount and the rejected vesting
# messages.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

NOW=$(python3 -c "import json; print(json.load(open('$HOME_DIR/vesting.json'))['now'])")
USER0=$(addr user0)
VAL0_OPER=$(valoper val0)
VC=$(addr vc)
VD=$(addr vd)
VP=$(addr vp)
VL=$(addr vl)
celestia-appd keys add vn "${K[@]}" >/dev/null 2>&1
celestia-appd keys add vn2 "${K[@]}" >/dev/null 2>&1
VN=$(addr vn)
VN2=$(addr vn2)

until_unix() { while [ "$(date +%s)" -lt "$1" ]; do sleep 1; done; }

# Continuous: locked coins limit sends; delegations track vesting/free.
tx vc bank send "$VC" "$USER0" 400000000utia # sdk 5: locked
tx vc bank send "$VC" "$USER0" 100000000utia
tx vc staking delegate "$VAL0_OPER" 500000000utia
tx vc staking delegate "$VAL0_OPER" 10000000utia # withdraws rewards: UpdateSchedule
tx vc staking unbond "$VAL0_OPER" 200000000utia
tx vc distribution withdraw-rewards "$VAL0_OPER"
tx vc distribution set-withdraw-addr "$USER0" # vesting account: sdk 18

# Permanently locked: only delegated coins free the balance.
tx vl bank send "$VL" "$USER0" 400000000utia # sdk 5
tx vl staking delegate "$VAL0_OPER" 800000000utia
tx vl bank send "$VL" "$USER0" 150000000utia
tx vl distribution withdraw-rewards "$VAL0_OPER"

# Periodic: the first period unlocks after 40 s.
tx vp bank send "$VP" "$USER0" 500000000utia # sdk 5
# Delayed: nothing unlocks before the end time.
tx vd bank send "$VD" "$USER0" 200000000utia # sdk 5

# MsgCreateVestingAccount (continuous and delayed), an existing target and
# the vesting messages celestia rejects.
tx user0 vesting create-vesting-account "$VN" 50000000utia $((NOW + 900)) --start-time $((NOW + 30))
tx user0 vesting create-vesting-account "$VN2" 50000000utia $((NOW + 900)) --start-time $((NOW + 30)) --delayed
tx user0 vesting create-vesting-account "$VN" 1000utia $((NOW + 900)) --start-time $((NOW + 30)) # exists: sdk 18
rawtx user0 "[{\"@type\":\"/cosmos.vesting.v1beta1.MsgCreatePermanentLockedAccount\",\"from_address\":\"$USER0\",\"to_address\":\"$VN2\",\"amount\":[{\"denom\":\"utia\",\"amount\":\"1\"}]}]" || true
rawtx user0 "[{\"@type\":\"/cosmos.vesting.v1beta1.MsgCreatePeriodicVestingAccount\",\"from_address\":\"$USER0\",\"to_address\":\"$VN\",\"start_time\":\"$NOW\",\"vesting_periods\":[{\"length\":\"10\",\"amount\":[{\"denom\":\"utia\",\"amount\":\"1\"}]}]}]"
tx vn bank send "$VN" "$USER0" 1000utia # all locked before the start: sdk 5

until_unix $((NOW + 45))
tx vp bank send "$VP" "$USER0" 500000000utia
until_unix $((NOW + 80))
tx vd bank send "$VD" "$USER0" 200000000utia
tx vc bank send "$VC" "$USER0" 100000000utia
tx vn bank send "$VN" "$USER0" 1000utia
wait_blocks 3
echo "workload done"
