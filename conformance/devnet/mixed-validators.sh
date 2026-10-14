#!/usr/bin/env bash
# M9 acceptance: a private network of Go celestia-appd (celestia-core
# v0.42.3) and Rust `celestia-rs start` validators reaching consensus.
#
# Network A (scenarios a, b, d): validators V0, V1 (Go) and V2, V3 (Rust)
#   with equal power, plus a Go full node F4 and a Rust full node F5, all
#   peered with each other. A tx workload (bank sends, blob PFBs incl. 64x64
#   squares, staking: delegate / redelegate / unbond / edit-validator,
#   touching Rust validators) runs against the Go validator while the chain
#   grows to <heights>. Midway the Rust validator V3 is killed (SIGKILL),
#   kept down for a while and restarted: it must recover from its WAL,
#   catch up through consensus + block propagation and sign again.
# Network B (scenario c): Rust validators hold 80% of the power (V2, V3 at
#   4x the Go validators' stake). The Go validators and a Go full node
#   follow; then both Go validators are stopped and the Rust validators
#   alone keep committing while the Go full node follows.
#
# Checks: every height reaches every node with the same block hash and app
# hash (Go via RPC, Rust from its commit log); Rust validators propose
# their share of blocks and sign commits; no evidence in any block and no
# conflicting vote in any log (no double signs); the restarted validator
# replayed its WAL and signed blocks after the restart.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/mixed-validators.sh [heights] [heights-B]
# Exit 0 iff all checks pass.
set -euo pipefail
HEIGHTS=${1:-100}
HEIGHTS_B=${2:-30}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-15000}
RS=$ROOT/target/debug/celestia-rs
W=$(mktemp -d)
FAIL=()

(cd "$ROOT" && cargo build -q -p celestia-rs)

# shellcheck source=lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"

# ================================================================ network A
echo "== network A: 2 Go + 2 Rust validators, Go and Rust full nodes"
CHAIN_ID=rs-mixed-val-1
A=$W/a
mkdir -p "$A"
make_net "$A" "$CHAIN_ID" "go go rs rs go rs" "5000000000 5000000000 5000000000 5000000000 0 0"
for i in 0 1 2 3 4 5; do start_node "$A" $i; done
ensure_up "$A" 0 1 2 3 4 5
wait_height "$A" 0 3 180 || FAIL+=("network A did not start")
echo "go height $(height "$A" 0), rust heights $(height "$A" 2) $(height "$A" 3)"

# Bank / staking txs go to the Go validator, the blob workload (Go CLI
# broadcasts, RPC polling) to the Rust validator's RPC.
workload "$A" 0 2 > "$A/workload.log" 2>&1 &
WL=$!

# (b) crash / restart a Rust validator mid-run.
KILL_AT=$((HEIGHTS * 2 / 5)); RESTART_AT=$((KILL_AT + 12))
wait_height "$A" 0 "$KILL_AT" $((HEIGHTS * 10)) || FAIL+=("network A stalled before $KILL_AT")
echo "== killing rust validator 3 at height $(height "$A" 3)"
kill_node "$A" 3 KILL
wait_height "$A" 0 "$RESTART_AT" 300 || FAIL+=("network A stalled with validator 3 down")
echo "== restarting rust validator 3 at chain height $(height "$A" 0)"
echo "--- restart ---" >> "$A/n3/node.log"
# Without block sync the node must recover from its WAL and catch up
# through consensus (commit gossip) and block propagation (AddCommitment,
# parts with proofs served from the peers' block stores).
RS_EXTRA=--no-block-sync start_node "$A" 3
ensure_up "$A" 3
wait "$WL" || true
cat "$A/workload.log" | tail -n 25
wait_height "$A" 0 $((HEIGHTS + 2)) $((HEIGHTS * 10)) || FAIL+=("network A did not reach $((HEIGHTS + 2))")
for i in 1 2 3 4 5; do
  wait_height "$A" $i $((HEIGHTS + 1)) 120 || FAIL+=("network A node $i stuck at $(height "$A" $i)")
done
sed 's/\x1b\[[0-9;]*m//g' "$A/n3/node.log" | sed -n '/--- restart ---/,$p' > "$A/n3/restart.log"
grep -q "Catchup by replaying consensus messages" "$A/n3/restart.log" || FAIL+=("validator 3 did not replay its WAL")
check_chain "$A" "$HEIGHTS" A || FAIL+=("network A chain checks")
# Go client tooling against a Rust validator's RPC: status, blocks and
# queries, and the P2P / sync view (net_info, catching_up).
RRPC="tcp://127.0.0.1:$((26657 + OFF + 20))"
celestia-appd status --node "$RRPC" > "$A/rs-status.json" 2>&1 || FAIL+=("celestia-appd status against the rust node")
python3 - "$A/rs-status.json" <<'PY2' || FAIL+=("rust node status")
import json, sys
d = json.load(open(sys.argv[1]))
si = d.get("sync_info") or d.get("SyncInfo")
print("rust node status: height", si["latest_block_height"], "catching_up", si["catching_up"])
sys.exit(0 if not si["catching_up"] and int(si["latest_block_height"]) > 0 else 1)
PY2
celestia-appd query block --type=height 10 --node "$RRPC" -o json > /dev/null 2>&1 || FAIL+=("query block via the rust node")
celestia-appd query bank balances "$(celestia-appd keys show user0 -a --keyring-backend test --home "$A/n0")" --node "$RRPC" -o json > /dev/null 2>&1 || FAIL+=("bank query via the rust node")
NP=$(curl -s "$(rpc "$A" 2)/net_info" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["n_peers"])' 2>/dev/null || echo 0)
echo "rust node 2 net_info: $NP peers"
[ "$NP" -ge 4 ] || FAIL+=("rust node net_info lists $NP peers")
python3 - "$A" "$RESTART_AT" "$HEIGHTS" "$OFF" <<'PY' || FAIL+=("network A proposer / restart checks")
import json, sys, urllib.request
d, restart, upto, off = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
st = json.load(open(d + "/stats-A.json"))
vals = {a: v for a, v in st["vals"].items()}
fail = []
for a, (i, k) in vals.items():
    if k == "rs" and i in (2, 3):
        p = st["proposed"].get(a, 0)
        if p < upto // 10:
            fail.append(f"rust validator {i} proposed only {p} of {upto} blocks")
# validator 3 signs again after the restart
a3 = [a for a, (i, _) in vals.items() if i == 3][0]
signed_after = 0
for h in range(restart + 15, upto + 1):
    b = json.load(urllib.request.urlopen(f"http://127.0.0.1:{26657 + off}/block?height={h + 1}"))["result"]
    signed_after += any(s["validator_address"].upper() == a3 and s["block_id_flag"] == 2 for s in b["block"]["last_commit"]["signatures"])
print(f"validator 3 signed {signed_after} of {upto - restart - 14} blocks after its restart")
if signed_after < (upto - restart - 14) // 2:
    fail.append("validator 3 did not resume signing")
for f in fail:
    print("FAIL:", f)
sys.exit(1 if fail else 0)
PY
for i in 0 1 2 3 4 5; do kill_node "$A" $i; done

# ================================================================ network B
echo "== network B: Rust validators hold 80% of the power"
CHAIN_ID=rs-mixed-val-2
B=$W/b
mkdir -p "$B"
OFF=$((OFF + 100))
make_net "$B" "$CHAIN_ID" "go go rs rs go" "2500000000 2500000000 10000000000 10000000000 0"
for i in 0 1 2 3 4; do start_node "$B" $i; done
ensure_up "$B" 0 1 2 3 4
HALF=$((HEIGHTS_B / 2))
wait_height "$B" 4 "$HALF" $((HEIGHTS_B * 10 + 120)) || FAIL+=("network B did not reach $HALF")
echo "== stopping both Go validators at height $(height "$B" 4)"
kill_node "$B" 0
kill_node "$B" 1
wait_height "$B" 4 $((HEIGHTS_B + 2)) $((HEIGHTS_B * 10)) || FAIL+=("network B: Go full node stuck at $(height "$B" 4) without Go validators")
for i in 2 3; do wait_height "$B" $i $((HEIGHTS_B + 1)) 60 || FAIL+=("network B rust node $i stuck"); done
check_chain "$B" "$HEIGHTS_B" B || FAIL+=("network B chain checks")
for i in 2 3 4; do kill_node "$B" $i; done

trap - EXIT
cleanup
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "mixed validators OK"
  rm -rf "$W"
  exit 0
fi
printf 'FAILED: %s\n' "${FAIL[@]}"
echo "logs: $W"
exit 1
