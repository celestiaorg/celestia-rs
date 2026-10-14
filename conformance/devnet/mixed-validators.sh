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

PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

shift_ports() { # file delta: shift every host:port listener
  python3 - "$1" "$2" <<'PY'
import re, sys
p, d = sys.argv[1], int(sys.argv[2])
s = open(p).read()
s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + d), s)
open(p, "w").write(s)
PY
}

# ---------------------------------------------------------------- network
# make_net <dir> <chain-id> <kinds> <stakes>: node i of kind go|rs, a
# validator when its stake (utia) is non-zero. Keys live in node0's keyring.
make_net() {
  local dir=$1 chain=$2
  local -a kinds=($3) stakes=($4)
  local n=${#kinds[@]} i h a
  for i in $(seq 0 $((n - 1))); do
    celestia-appd init "node$i" --chain-id "$chain" --home "$dir/n$i" >/dev/null 2>&1
  done
  a="celestia-appd --home $dir/n0"
  for k in user0 user1; do
    $a keys add $k --keyring-backend test >/dev/null 2>&1
    $a genesis add-genesis-account "$($a keys show $k -a --keyring-backend test)" 100000000000000utia >/dev/null 2>&1
  done
  for i in $(seq 0 $((n - 1))); do
    h=$dir/n$i
    celestia-appd keys add "val$i" --keyring-backend test --home "$h" >/dev/null 2>&1
    $a genesis add-genesis-account "$(celestia-appd keys show "val$i" -a --keyring-backend test --home "$h")" 100000000000000utia >/dev/null 2>&1
  done
  for i in $(seq 1 $((n - 1))); do cp "$dir/n0/config/genesis.json" "$dir/n$i/config/genesis.json"; done
  mkdir -p "$dir/n0/config/gentx"
  for i in $(seq 0 $((n - 1))); do
    [ "${stakes[$i]}" = 0 ] && continue
    h=$dir/n$i
    celestia-appd genesis gentx "val$i" "${stakes[$i]}utia" --chain-id "$chain" --keyring-backend test \
      --home "$h" --fees 500000utia --output-document "$dir/n0/config/gentx/val$i.json" >/dev/null 2>&1
  done
  $a genesis collect-gentxs >/dev/null 2>&1
  for i in $(seq 1 $((n - 1))); do cp "$dir/n0/config/genesis.json" "$dir/n$i/config/genesis.json"; done
  # Go configs: shifted ports, relaxed local p2p, all-to-all peers.
  local peers=() id
  for i in $(seq 0 $((n - 1))); do
    id=$(celestia-appd comet show-node-id --home "$dir/n$i" 2>/dev/null)
    peers+=("$id@127.0.0.1:$((26656 + OFF + 10 * i))")
    echo "$id" > "$dir/n$i/node_id"
  done
  for i in $(seq 0 $((n - 1))); do
    h=$dir/n$i
    shift_ports "$h/config/config.toml" $((OFF + 10 * i))
    shift_ports "$h/config/app.toml" $((OFF + 10 * i))
    local others
    others=$(printf '%s\n' "${peers[@]}" | grep -v "^$(cat "$h/node_id")@" | paste -sd, -)
    sed -i -e 's#^addr_book_strict = true#addr_book_strict = false#' \
      -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' \
      -e "s#^persistent_peers = .*#persistent_peers = \"$others\"#" \
      -e 's#^pex = true#pex = false#' "$h/config/config.toml"
    echo "$others" > "$h/peers"
    echo "${kinds[$i]}" > "$h/kind"
  done
}

start_node() { # <dir> <i>
  local h=$1/n$2 log
  log=$h/node.log
  if [ "$(cat "$h/kind")" = go ]; then
    celestia-appd start --home "$h" >> "$log" 2>&1 &
  else
    RUST_LOG=${RUST_LOG:-info} "$RS" start --home "$h" --p2p-laddr "127.0.0.1:$((26656 + OFF + 10 * $2))" \
      --persistent-peers "$(cat "$h/peers")" --moniker "node$2" --allow-duplicate-ip ${RS_EXTRA:-} >> "$log" 2>&1 &
  fi
  echo $! > "$h/pid"
  PIDS+=($!)
}

# Ports above 32767 can be taken as ephemeral source ports by the other
# nodes' outbound dials before a node binds them; restart nodes that died
# on "address already in use".
ensure_up() { # <dir> <i>...
  local d=$1 i t
  shift
  for t in 1 2 3 4 5; do
    sleep 8
    local again=0
    for i in "$@"; do
      if ! kill -0 "$(cat "$d/n$i/pid")" 2>/dev/null && grep -qi "address already in use" "$d/n$i/node.log"; then
        echo "node $i lost a port race; restarting"
        start_node "$d" "$i"
        again=1
      fi
    done
    [ $again = 0 ] && return 0
  done
}

kill_node() { # <dir> <i> [signal]
  local pid
  pid=$(cat "$1/n$2/pid")
  kill "-${3:-TERM}" "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
}

rpc() { echo "http://127.0.0.1:$((26657 + OFF + 10 * $2))"; } # <dir> <i>

# height <dir> <i>: latest committed height of a node.
height() {
  local h=$1/n$2
  if [ "$(cat "$h/kind")" = go ]; then
    curl -s "$(rpc "$1" "$2")/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p' | grep . || echo 0
  else
    sed 's/\x1b\[[0-9;]*m//g' "$h/node.log" | sed -n 's/.*\(committed\|synced\) block height=\([0-9]*\).*/\2/p' | tail -n1 | grep . || echo 0
  fi
}

wait_height() { # <dir> <i> <height> <timeout-s>
  local i
  for i in $(seq 1 "$4"); do
    [ "$(height "$1" "$2")" -ge "$3" ] 2>/dev/null && return 0
    sleep 1
  done
  return 1
}

# check_chain <dir> <upto> <label>: block / app hash agreement at every
# height on every node, proposer and signature counts, no evidence, no
# conflicting votes. Uses node 0 (Go) or the first Go node still running
# as the reference.
check_chain() {
  python3 - "$1" "$2" "$3" "$OFF" <<'PY'
import base64, glob, json, os, re, sys, urllib.request
d, upto, label, off = sys.argv[1], int(sys.argv[2]), sys.argv[3], int(sys.argv[4])
nodes = sorted(glob.glob(d + "/n*"), key=lambda p: int(p.rsplit("n", 1)[1]))
fail = []
def rpc(i, path):
    return json.load(urllib.request.urlopen(f"http://127.0.0.1:{26657 + off + 10 * i}{path}", timeout=10))["result"]
kinds = [open(n + "/kind").read().strip() for n in nodes]
ref = None
for i, k in enumerate(kinds):
    if k == "go":
        try:
            if int(rpc(i, "/status")["sync_info"]["latest_block_height"]) >= upto + 1:
                ref = i
                break
        except Exception:
            pass
if ref is None:
    print(f"[{label}] no Go node at height {upto + 1}")
    sys.exit(1)
# Reference chain from Go.
blocks = {}
for h in range(1, upto + 2):
    b = rpc(ref, f"/block?height={h}")
    blocks[h] = b
vals = {}
for i, n in enumerate(nodes):
    k = json.load(open(n + "/config/priv_validator_key.json"))
    vals[k["address"].upper()] = (i, kinds[i])
proposed = {}
signed = {}
for h in range(1, upto + 1):
    hdr = blocks[h]["block"]["header"]
    p = hdr["proposer_address"].upper()
    proposed[p] = proposed.get(p, 0) + 1
    ev = blocks[h]["block"]["evidence"]["evidence"] or []
    if ev:
        fail.append(f"evidence in block {h}: {ev}")
    for s in blocks[h + 1]["block"]["last_commit"]["signatures"]:
        if s["block_id_flag"] == 2:
            a = s["validator_address"].upper()
            signed[a] = signed.get(a, 0) + 1
# Every Go node agrees with the reference.
for i, k in enumerate(kinds):
    if k != "go" or i == ref:
        continue
    try:
        top = int(rpc(i, "/status")["sync_info"]["latest_block_height"])
    except Exception:
        continue
    for h in range(1, min(upto, top) + 1):
        if rpc(i, f"/block?height={h}")["block_id"]["hash"] != blocks[h]["block_id"]["hash"]:
            fail.append(f"go node {i} block {h} differs")
            break
    if top < upto:
        fail.append(f"go node {i} only at height {top}")
# Every Rust node: commit log block hash and app hash.
for i, k in enumerate(kinds):
    if k != "rs":
        continue
    seen = {}
    for line in open(nodes[i] + "/node.log", errors="replace"):
        line = re.sub(r"\x1b\[[0-9;]*m", "", line)
        m = re.search(r"(?:committed|synced) block height=(\d+) hash=([0-9A-F]+) app_hash=([0-9A-F]+)", line)
        if m:
            seen[int(m.group(1))] = (m.group(2), m.group(3))
    missing = [h for h in range(1, upto + 1) if h not in seen]
    # Heights committed while a node was down come from catch-up and are
    # logged too, so every height must be present.
    if missing:
        fail.append(f"rust node {i} missing heights {missing[:10]}{'...' if len(missing) > 10 else ''}")
    bad = 0
    for h, (bh, ah) in seen.items():
        if h > upto:
            continue
        if bh != blocks[h]["block_id"]["hash"].upper() or ah != blocks[h + 1]["block"]["header"]["app_hash"].upper():
            bad += 1
            if bad <= 3:
                fail.append(f"rust node {i} height {h}: block {bh[:12]} app {ah[:12]} != go block {blocks[h]['block_id']['hash'][:12]} app {blocks[h + 1]['block']['header']['app_hash'][:12]}")
    print(f"[{label}] rust node {i}: {len(seen)} commits logged, {bad} mismatches")
# Logs: no double signs / conflicting votes / consensus failures.
for i, n in enumerate(nodes):
    text = re.sub(r"\x1b\[[0-9;]*m", "", open(n + "/node.log", errors="replace").read())
    for pat in ["conflicting votes", "found conflicting vote", "CONSENSUS FAILURE", "double sign", "panicked"]:
        if pat.lower() in text.lower():
            fail.append(f"node {i} log has '{pat}'")
print(f"[{label}] heights 1..{upto} checked against go node {ref}")
for a, (i, k) in sorted(vals.items(), key=lambda x: x[1]):
    if a in proposed or a in signed:
        print(f"[{label}]   node {i} ({k}) {a[:12]}: proposed {proposed.get(a, 0)}, signed {signed.get(a, 0)}")
json.dump({"proposed": proposed, "signed": signed, "vals": vals}, open(d + f"/stats-{label}.json", "w"))
for f in fail:
    print(f"[{label}] FAIL: {f}")
sys.exit(1 if fail else 0)
PY
}

# ---------------------------------------------------------------- workload
workload() { # <dir> <rpc-node-index>
  local dir=$1 node="tcp://127.0.0.1:$((26657 + OFF + 10 * $2))" h0 a k out v2 v3 v0
  h0=$dir/n0
  k=(--keyring-backend test --home "$h0")
  a() { celestia-appd keys show "$1" -a "${k[@]}"; }
  # Validator operator keys live in each node's own keyring.
  v0=$(celestia-appd keys show val0 --bech val -a --keyring-backend test --home "$dir/n0")
  v2=$(celestia-appd keys show val2 --bech val -a --keyring-backend test --home "$dir/n2")
  v3=$(celestia-appd keys show val3 --bech val -a --keyring-backend test --home "$dir/n3")
  send() { # <from> <args...>
    local from=$1; shift
    out=$(celestia-appd tx "$@" --from "$from" "${k[@]}" --chain-id "$CHAIN_ID" --node "$node" \
      --fees 40000utia --gas 400000 -y -o json 2>&1) || true
    echo "tx $* -> $(echo "$out" | python3 -c 'import json,sys; d=json.load(sys.stdin); print(d.get("code", 0), d.get("txhash", ""))' 2>/dev/null || echo "${out:0:160}")"
  }
  local u0 u1
  u0=$(a user0); u1=$(a user1)
  # bank sends (both directions, several per block)
  for i in $(seq 1 6); do
    send user0 bank send "$u0" "$u1" "$((1000 + i))utia" &
    send user1 bank send "$u1" "$u0" "$((2000 + i))utia" &
    wait
    sleep 1
  done
  # staking touching the Rust validators
  send user0 staking delegate "$v2" 3000000000utia; sleep 4
  send user1 staking delegate "$v3" 2000000000utia; sleep 4
  send user0 staking redelegate "$v2" "$v0" 1000000000utia --gas 600000; sleep 4
  send user1 staking unbond "$v3" 500000000utia; sleep 4
  send user0 distribution withdraw-all-rewards; sleep 4
  # blobs (PFBs up to 64x64 squares: multi-part blocks)
  HOME_DIR=$h0 CHAIN_ID=$CHAIN_ID NODE=$node bash "$ROOT/conformance/devnet/workloads/blob.sh" > "$dir/blob.log" 2>&1 || echo "blob workload exited with $?"
  grep -E "^pfb" "$dir/blob.log" | head -30 || true
  for i in $(seq 1 6); do send user0 bank send "$u0" "$u1" "$((3000 + i))utia"; sleep 1; done
}

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

workload "$A" 0 > "$A/workload.log" 2>&1 &
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
