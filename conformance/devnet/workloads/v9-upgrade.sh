#!/usr/bin/env bash
# v9 → v10 upgrade workload, as mainnet / Mocha upgrade. Run with
# APP_V9=1 UPGRADE_TO_V10=1 CHAIN_ID=test (signal upgrade delay 3):
#
#   APP_V9=1 UPGRADE_TO_V10=1 CHAIN_ID=test conformance/devnet/devnet.sh \
#     v9-upgrade 40 conformance/devnet/workloads/v9-upgrade.sh
#
# Under v9: bank sends, a delegation,
# MsgSignalVersion 10, MsgTryUpgrade. At the upgrade
# height the v9 EndBlocker schedules plan "v10" at H+1 and sets app version
# 10; the recorder switches to the v10 binary after that Commit (like the
# multiplexer), whose store loader adds the fibre / valaddr stores and
# whose PreBlocker runs the v10 handler at H+1. Then txs under v10.
set -euo pipefail
. "$(dirname "$0")/lib.sh"

VAL0=$(addr val0)
USER0=$(addr user0)

app_version() { curl -s "$RPC/abci_info" | json 'd["result"]["response"].get("app_version","0")'; }

tx val0 bank send "$VAL0" "$USER0" 1000utia
tx user0 bank send "$USER0" "$VAL0" 10utia
tx val0 staking delegate "$(valoper val0)" 1000000utia
echo "app version before: $(app_version)"

tx val0 signal signal 10
tx val0 signal try-upgrade
until [ "$(app_version)" = "10" ]; do sleep 1; done
echo "app version 10 at height $(height)"
wait_blocks 2

# Under v10 (the upgrade height + 1 ran the v10 handler).
tx val0 bank send "$VAL0" "$USER0" 2000utia
tx user0 bank send "$USER0" "$VAL0" 20utia
tx val0 distribution withdraw-rewards "$(valoper val0)" --commission
q upgrade applied v10 || true
echo "workload done"
