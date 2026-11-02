#!/usr/bin/env bash
# Fresh current-release authz/feegrant inputs and ABCI/REST query responses.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
export DEVNET_BIN=${DEVNET_BIN:-$ROOT/target/populated-authz-bin}
: "${MAINNET_OBSERVATION:?set MAINNET_OBSERVATION to a fresh observe-mainnet.py output}"
CLI_HOME=$(mktemp -d)
trap 'rm -rf "$CLI_HOME"' EXIT
# NewRootCmd instantiates an app even for client-only subcommands. Keep that
# incidental application state inside this isolated campaign.
export CELESTIA_APP_HOME=$CLI_HOME
"$ROOT/conformance/devnet/build-bins.sh"
export GOTOOLCHAIN=${GOTOOLCHAIN:-go1.26.9}
(cd "$ROOT/conformance/devnet-oracle" && go build -buildvcs=false -o "$DEVNET_BIN/devnet-oracle" .)
(cd "$ROOT/conformance/rpccorpus/recorder" && \
  go build -buildvcs=false -o "$DEVNET_BIN/abci-recorder" ./cmd/abci-frame-recorder && \
  go build -buildvcs=false -o "$DEVNET_BIN/rpc-recorder" .)
DEVNET_DETERMINISTIC=1 DEVNET_KEYS_DIR="$ROOT/conformance/mainnet-rpc/populated-governance/devnet-keys" \
SAVE_BLOCKS=1 DEVNET_TX_INDEX=kv DEVNET_PORT_OFFSET=${DEVNET_PORT_OFFSET:-6000} \
  bash "$ROOT/conformance/devnet/devnet.sh" v105-authz-feegrant 50 \
  "$ROOT/conformance/devnet/workloads/authz-feegrant.sh"
bash "$ROOT/conformance/querycorpus/record.sh" v105-authz-feegrant
python3 "$ROOT/conformance/mainnet-rpc/populated-authz/receipt.py" \
  --observation "$MAINNET_OBSERVATION" --bin "$DEVNET_BIN"
python3 "$ROOT/conformance/mainnet-rpc/populated-authz/verify.py"
