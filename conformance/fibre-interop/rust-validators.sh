#!/usr/bin/env bash
# The celestia-app v11.0.0 Go Fibre client against a devnet of four
# celestia-rs validators with the embedded Fibre server (equal stake, app
# version 10, genesis made with the celestia-rs CLI).
#
#  1. Each validator registers its Fibre host (tx valaddr set-host); the Go
#     gRPC health client sees readiness and liveness SERVING over TLS 1.3.
#  2. The Go client funds an escrow (MsgDepositToEscrow), then uploads blobs
#     of $SIZES: the first with fibre.Put, the others with Client.Upload
#     awaiting every validator's signature plus a MsgPayForFibre. Each tx
#     must commit with code 0 (the Rust servers' signatures verify on chain)
#     and each blob must download byte-identical with the Go client.
#  3. Download of a blob nobody uploaded: fibre.ErrNotFound.
#  4. $CLIENTS separate Go clients (one connection per validator each, more
#     than the servers' 16-connection cap) run fibre.Put at once; then one
#     client runs $CLIENTS Puts of 2 MiB at once (more streams per
#     connection than the servers' 13-stream cap).
#  5. One validator is stopped: fibre.Put still collects 2/3 of the
#     signatures and commits, and every earlier blob still reconstructs.
#  6. A raw UploadShard loop fills validator 0's storage budget (set low in
#     genesis) until it answers ResourceExhausted; grpc-go must decode the
#     RetryInfo detail as the Go client's retry logic does.
#  7. All nodes agree on block and app hashes at every height.
#
# usage: [SIZES=1K,300K,8M,64M] [CLIENTS=20] [DEVNET_PORT_OFFSET=n]
#        conformance/fibre-interop/rust-validators.sh [path/to/celestia-rs]
# Exit 0 iff every check passes.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SIZES=${SIZES:-1K,300K,8M,64M}
CLIENTS=${CLIENTS:-20}
OFF=${DEVNET_PORT_OFFSET:-21000}
W=$(mktemp -d)
GOBIN=${GOBIN:-$W/bin}
mkdir -p "$GOBIN"
FAIL=()
CHAIN_ID=fibre-rs-1

if [ -n "${1:-}" ]; then
  RS=$(realpath "$1")
else
  (cd "$ROOT" && cargo build -q -p celestia-rs)
  RS=${CARGO_TARGET_DIR:-$ROOT/target}/debug/celestia-rs
fi

# shellcheck source=../devnet/lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"
# shellcheck source=lib.sh
source "$ROOT/conformance/fibre-interop/lib.sh"
build_interop

# lib-mixed's make_net drives a celestia-appd compatible CLI: the
# celestia-rs one (init, keys, genesis, comet show-node-id).
celestia-appd() { "$RS" "$@"; }

D=$W/net
mkdir -p "$D"
make_net "$D" "$CHAIN_ID" "rs rs rs rs" "5000000000 5000000000 5000000000 5000000000"
# Storage budget. A validator's budget is full_stake_budget x assigned rows
# / 4096 original rows and its shard of a blob is assigned rows x row size,
# so the full-stake budget counts in upload sizes (plus proofs): what this
# run uploads, plus room for a few 8 MiB probes, so that the probe loop of
# step 6 runs out within its 40 tries.
BUDGET=$(python3 -c '
import sys
tot = 0
for f in sys.argv[1].split(","):
    m = {"K": 1 << 10, "M": 1 << 20}.get(f[-1], 1)
    n = int(f.rstrip("KM")) * m
    tot += max(n, 1 << 18)
print(int((tot + int(sys.argv[2]) * ((1 << 18) + (2 << 20)) + 4 * (1 << 18)) * 1.15) + 96 * (1 << 20))' "$SIZES" "$CLIENTS")
set_genesis_budget "$D" "$BUDGET"
echo "== four celestia-rs validators, full-stake storage budget $((BUDGET >> 20)) MiB"
for i in 0 1 2 3; do enable_rust_fibre "$D" $i; start_node "$D" $i; done
ensure_up "$D" 0 1 2 3
wait_height "$D" 0 3 180 || FAIL+=("devnet did not start")

for i in 0 1 2 3; do register_host "$D" $i "$RS" || FAIL+=("set-host $i"); done
wait_healthy 120 "$(fibre_addr 0)" "$(fibre_addr 1)" "$(fibre_addr 2)" "$(fibre_addr 3)" || FAIL+=("readiness")

G=(-grpc "$(grpc_addr 0)" -home "$D/n0" -key user0)
REC=$W/records.jsonl
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "== Go client: escrow, uploads of $SIZES, payments, downloads"
  interop deposit "${G[@]}" -amount 50000000000 || FAIL+=("deposit")
  interop put "${G[@]}" -sizes "$SIZES" -seed 100 -out "$REC" || FAIL+=("put")
  [ -f "$REC" ] && signer_kinds "$REC" "$D"
  for i in 0 1 2 3; do echo "node $i peak RSS $(peak_rss "$(cat "$D/n$i/pid")") MiB"; done
  echo "== Go client: unknown blob"
  interop notfound "${G[@]}" || FAIL+=("notfound")
  echo "== $CLIENTS Go clients at once"
  interop concurrent "${G[@]}" -clients "$CLIENTS" -seed 1000 || FAIL+=("concurrent clients")
  interop concurrent "${G[@]}" -clients "$CLIENTS" -shared -size 2M -seed 2000 || FAIL+=("concurrent Puts on one client")

  echo "== validator 3 stopped"
  chain_agree "$(($(latest 3) - 1))" 0 1 2 3 || FAIL+=("chain agreement before the stop")
  kill_node "$D" 3
  interop put "${G[@]}" -sizes 1K,2M -seed 200 -quorum -out "$W/minority.jsonl" || FAIL+=("put with a validator stopped")
  [ -f "$W/minority.jsonl" ] && signer_kinds "$W/minority.jsonl" "$D"
  interop download "${G[@]}" -in "$REC" || FAIL+=("download with a validator stopped")

  echo "== storage budget of validator 0"
  interop exhaust "${G[@]}" -target "$(fibre_addr 0)" || FAIL+=("budget exhaustion")
fi

TOP=$(($(latest 0) - 2))
chain_agree "$TOP" 0 1 2 || FAIL+=("chain agreement")
if grep -l "panicked" "$D"/n*/node.log; then FAIL+=("a node panicked"); fi
for i in 0 1 2; do kill_node "$D" $i; done

trap - EXIT
cleanup
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "Go v11 Fibre client against celestia-rs validators OK"
  rm -rf "$W"
  exit 0
fi
printf 'FAILED: %s\n' "${FAIL[@]}"
echo "logs: $W"
exit 1
