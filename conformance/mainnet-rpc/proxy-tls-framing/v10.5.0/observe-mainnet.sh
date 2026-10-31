#!/bin/sh
set -eu
python3 observe-mainnet.py \
  --endpoint https://rpc.celestia.pops.one \
  --out mainnet-observation.json
