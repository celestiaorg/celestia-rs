#!/usr/bin/env bash
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
: "${CAPTURE_DIR:?}" "${CAPTURE_HEIGHT:?}" "${ABCI_ADDR:?}" "${COMET_RPC:?}" "${SDK_GRPC:?}" "${REST_API:?}" "${HOME_DIR:?}" "${CHAIN_ID:?}"
"${POPULATED_REQUESTS_BIN:-$ROOT/target/debug/examples/populated_requests}" "$ABCI_ADDR" "$CAPTURE_HEIGHT" \
  "$(dirname "$HOME_DIR")/abci.rec" "$CAPTURE_DIR/requests.json"
populated-matrix -abci "$ABCI_ADDR" -rpc "$COMET_RPC" -grpc "$SDK_GRPC" -rest "$REST_API" \
  -chain-id "$CHAIN_ID" -inputs "$CAPTURE_DIR/requests.json" -out "$CAPTURE_DIR/transports.json"
gzip -9 -n "$CAPTURE_DIR/transports.json"
"${PROOFCHECK_BIN:-proofcheck}" "$CAPTURE_DIR/transports.json.gz" "$CAPTURE_DIR/proofs.json"
