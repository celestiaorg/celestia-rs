#!/usr/bin/env bash
# Fibre on a mixed network: two Go validators (celestia-appd v11.0.0, each
# with the standalone Go `fibre` v11.0.0 server signing through the node's
# privval gRPC) and two celestia-rs validators (embedded server), equal
# stake, genesis made by celestia-appd v11.0.0.
#
#  1. Every validator registers its host; the Go health client sees all four
#     servers SERVING.
#  2. The celestia-app v11.0.0 Go client uploads $SIZES awaiting all four
#     signatures (Go and Rust servers accept the same uploads), pays with
#     MsgPayForFibre carrying both kinds of signatures, requires code 0 and
#     downloads every blob byte-identical.
#  3. The Rust side's client uploads too: the celestia-rs bridge of node 2
#     (fibre.Submit over its DA JSON-RPC, lumina's celestia-fibre client)
#     pays from its own escrow; the Go client downloads those blobs.
#  4. Unknown blob: fibre.ErrNotFound.
#  5. Chain checks (lib-mixed check_chain, Go node as reference, plus RPC
#     block and app hashes of all four nodes): no divergence.
#  6. One Go and one Rust validator stop (appd + fibre, and the Rust node):
#     each survivor holds fewer rows than reconstruction needs, so every
#     blob must reconstruct from a mix of Go and Rust shards.
#
# The Go binaries come from conformance/devnet/build-bins.sh
# (APP_VERSION=v11.0.0 CORE_VERSION=v0.42.8 APPD_ONLY=1 FIBRE=1).
#
# usage: [SIZES=1K,300K,8M,32M] [DEVNET_BIN=dir] [DEVNET_PORT_OFFSET=n]
#        conformance/fibre-interop/mixed-validators.sh <celestia-rs built with --features da-fibre>
# Exit 0 iff every check passes.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
RS=$(realpath "${1:?pass a celestia-rs binary built with --features da-fibre}")
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin-v11}
export PATH=$BIN:$PATH
SIZES=${SIZES:-1K,300K,8M,32M}
OFF=${DEVNET_PORT_OFFSET:-23000}
W=$(mktemp -d)
GOBIN=${GOBIN:-$W/bin}
mkdir -p "$GOBIN"
FAIL=()
CHAIN_ID=fibre-mixed-1
BRIDGE=127.0.0.1:$((39000 + OFF % 1000))

for b in celestia-appd fibre; do
  [ -x "$BIN/$b" ] || { echo "missing $BIN/$b: run conformance/devnet/build-bins.sh with APP_VERSION=v11.0.0 FIBRE=1" >&2; exit 1; }
done
echo "Go: $(cat "$BIN/celestia-appd.module") and $(cat "$BIN/fibre.module")"

# shellcheck source=../devnet/lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"
# shellcheck source=lib.sh
source "$ROOT/conformance/fibre-interop/lib.sh"
build_interop

FIBRE_PIDS=()
stop_go_fibre() { # <i>
  local p
  p=$(cat "$D/n$1/fibre.pid" 2>/dev/null) || return 0
  kill "$p" 2>/dev/null || true
  # The Go server drains until a second signal.
  sleep 1
  kill -KILL "$p" 2>/dev/null || true
  wait "$p" 2>/dev/null || true
}
cleanup_all() {
  for p in "${FIBRE_PIDS[@]}"; do kill -KILL "$p" 2>/dev/null || true; done
  cleanup
}
trap cleanup_all EXIT

D=$W/net
mkdir -p "$D"
make_net "$D" "$CHAIN_ID" "go go rs rs" "5000000000 5000000000 5000000000 5000000000"
python3 -c 'import json,sys; g=json.load(open(sys.argv[1])); print("genesis app version", g["consensus"]["params"]["version"]["app"])' "$D/n0/config/genesis.json"
for i in 0 1; do
  enable_app_grpc "$D" $i
  sed -i "s#^priv_validator_grpc_laddr = .*#priv_validator_grpc_laddr = \"$(privval_addr $i)\"#" "$D/n$i/config/config.toml"
  grep -q "^priv_validator_grpc_laddr = \"$(privval_addr $i)\"" "$D/n$i/config/config.toml" || { echo "no priv_validator_grpc_laddr in the Go config" >&2; exit 1; }
done
for i in 2 3; do enable_rust_fibre "$D" $i; done
echo "== 2 Go (celestia-appd + fibre v11.0.0) and 2 celestia-rs validators"
start_node "$D" 0
start_node "$D" 1
RS_EXTRA="--bridge-signer val2 --bridge-rpc-listen $BRIDGE" start_node "$D" 2
start_node "$D" 3
ensure_up "$D" 0 1 2 3
wait_height "$D" 0 3 180 || FAIL+=("devnet did not start")
for i in 0 1; do
  fibre start --home "$D/n$i/fibre" --app-grpc-address "$(grpc_addr $i)" --server-listen-address "$(fibre_addr $i)" \
    --signer-grpc-address "$(privval_addr $i)" >> "$D/n$i/fibre.log" 2>&1 &
  echo $! > "$D/n$i/fibre.pid"
  FIBRE_PIDS+=($!)
done

for i in 0 1 2 3; do register_host "$D" $i celestia-appd || FAIL+=("set-host $i"); done
wait_healthy 180 "$(fibre_addr 0)" "$(fibre_addr 1)" "$(fibre_addr 2)" "$(fibre_addr 3)" || FAIL+=("readiness")

G=(-grpc "$(grpc_addr 0)" -home "$D/n0" -key user0)
REC=$W/records.jsonl
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "== Go client: uploads of $SIZES to Go and Rust servers"
  interop deposit "${G[@]}" -amount 50000000000 || FAIL+=("deposit")
  interop put "${G[@]}" -sizes "$SIZES" -seed 300 -out "$REC" || FAIL+=("put")
  [ -f "$REC" ] && signer_kinds "$REC" "$D"

  echo "== Rust client (celestia-rs bridge of node 2): uploads, Go client downloads"
  interop deposit -grpc "$(grpc_addr 2)" -home "$D/n2" -key val2 -amount 50000000000 || FAIL+=("bridge deposit")
  TOKEN=$("$RS" da-auth write --home "$D/n2")
  interop bridge -grpc "$(grpc_addr 1)" -home "$D/n2" -key val2 -bridge "http://$BRIDGE" -token "$TOKEN" -sizes 1K,4M -seed 400 -out "$REC" || FAIL+=("bridge upload")
  signer_kinds "$REC" "$D" | tail -n 2
  [ -f "$REC" ] && python3 - "$REC" "$D" <<'PY' || FAIL+=("a payment without both kinds of signatures")
import glob, json, sys
kinds = {}
for n in glob.glob(sys.argv[2] + "/n*"):
    kinds[json.load(open(n + "/config/priv_validator_key.json"))["address"].upper()] = open(n + "/kind").read().strip()
for line in open(sys.argv[1]):
    r = json.loads(line)
    if r["via"] == "Client.Upload+MsgPayForFibre" and len(r["signers"]) != r["validators"]:
        sys.exit(f"{r['size']} bytes: {len(r['signers'])} of {r['validators']} signatures")
    if {kinds[s.upper()] for s in r["signers"]} != {"go", "rs"}:
        sys.exit(f"{r['size']} bytes: signers {r['signers']}")
PY


  echo "== unknown blob"
  interop notfound "${G[@]}" || FAIL+=("notfound")
fi

for i in 0 1 2 3; do echo "node $i ($(cat "$D/n$i/kind")) peak RSS $(peak_rss "$(cat "$D/n$i/pid")") MiB"; done
TOP=$(($(latest 0) - 2))
check_chain "$D" "$TOP" mixed || FAIL+=("chain checks")
chain_agree "$TOP" 0 1 2 3 || FAIL+=("chain agreement")

if [ -f "$REC" ]; then
  echo "== Go validator 1 and Rust validator 3 stopped: downloads from Go node 0 and Rust node 2 shards"
  stop_go_fibre 1
  kill_node "$D" 1
  kill_node "$D" 3
  interop download -grpc "$(grpc_addr 0)" -in "$REC" || FAIL+=("download from mixed shards")
fi
for i in 0 1; do grep -iE "panic|fatal" "$D/n$i/fibre.log" && FAIL+=("Go fibre $i log"); done
if grep -l "panicked" "$D"/n*/node.log; then FAIL+=("a node panicked"); fi

for i in 0 1; do stop_go_fibre $i; done
for i in 0 1 2 3; do kill_node "$D" $i; done
trap - EXIT
cleanup_all
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "Fibre on a mixed Go v11.0.0 / celestia-rs network OK"
  rm -rf "$W"
  exit 0
fi
printf 'FAILED: %s\n' "${FAIL[@]}"
echo "logs: $W"
exit 1
