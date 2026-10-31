#!/usr/bin/env bash
# Capture hook run by conformance/devnet/devnet.sh while its isolated node lives.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
: "${CAPTURE_DIR:?missing capture output directory}"
: "${CAPTURE_HEIGHT:?missing capture height}"
: "${ABCI_ADDR:?missing direct ABCI address}"
: "${COMET_RPC:?missing Comet RPC URL}"
: "${SDK_GRPC:?missing Cosmos gRPC address}"
: "${REST_API:?missing REST URL}"
: "${HOME_DIR:?missing isolated app home}"
: "${CHAIN_ID:?missing chain ID}"
K=(--keyring-backend test --home "$HOME_DIR")
DELEGATOR=$(celestia-appd keys show user0 -a "${K[@]}")
VALIDATOR=$(celestia-appd keys show val0 --bech val -a "${K[@]}")
(cd "$ROOT/conformance/rpccorpus/recorder" && go run ./cmd/populated-queries \
  -abci "$ABCI_ADDR" -rpc "$COMET_RPC" -grpc "$SDK_GRPC" -rest "$REST_API" \
  -chain-id "$CHAIN_ID" -tip-height "$CAPTURE_HEIGHT" \
  -delegator "$DELEGATOR" -validator "$VALIDATOR" \
  -out "$CAPTURE_DIR/transports.json")
