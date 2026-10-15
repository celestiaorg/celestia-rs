#!/usr/bin/env bash
# Genesis setup for v9-exec: account "multi" holds utia and uatom, so it can
# pay a fee in two denoms (accepted at app version 9 only).
set -euo pipefail
K=(--keyring-backend test --home "$HOME_DIR")
A="celestia-appd --home $HOME_DIR"
$A keys add multi "${K[@]}" >/dev/null 2>&1
$A genesis add-genesis-account "$($A keys show multi -a "${K[@]}")" 1000000000utia,1000000uatom >/dev/null
