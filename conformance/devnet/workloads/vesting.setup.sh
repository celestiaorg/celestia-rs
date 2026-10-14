#!/usr/bin/env bash
# Genesis setup for the vesting workload: a continuous (vc) and a delayed (vd)
# vesting account via add-genesis-account, plus vp and vl which
# vesting.genesis.py turns into periodic and permanently locked accounts.
# Times are written to $HOME_DIR/vesting.json for the patch and workload.
set -euo pipefail
K=(--keyring-backend test --home "$HOME_DIR")
A="celestia-appd --home $HOME_DIR"
NOW=$(date +%s)
for k in vc vd vp vl; do
  $A keys add $k "${K[@]}" >/dev/null 2>&1
done
addr() { $A keys show "$1" -a "${K[@]}"; }
$A genesis add-genesis-account "$(addr vc)" 1000000000utia --vesting-amount 800000000utia \
  --vesting-start-time "$NOW" --vesting-end-time $((NOW + 600)) >/dev/null
$A genesis add-genesis-account "$(addr vd)" 1000000000utia --vesting-amount 900000000utia \
  --vesting-end-time $((NOW + 75)) >/dev/null
$A genesis add-genesis-account "$(addr vp)" 1000000000utia >/dev/null
$A genesis add-genesis-account "$(addr vl)" 1000000000utia >/dev/null
cat >"$HOME_DIR/vesting.json" <<JSON
{"now": $NOW, "vp": "$(addr vp)", "vl": "$(addr vl)"}
JSON
