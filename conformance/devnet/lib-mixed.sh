# Helpers for mixed Go / Rust networks (sourced by mixed-validators.sh and
# double-sign-evidence.sh). Expects ROOT, OFF, RS, W and FAIL.

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
evidence = []
for h in range(1, upto + 1):
    hdr = blocks[h]["block"]["header"]
    p = hdr["proposer_address"].upper()
    proposed[p] = proposed.get(p, 0) + 1
    ev = blocks[h]["block"]["evidence"]["evidence"] or []
    if ev:
        evidence.append((h, ev))
        if os.environ.get("ALLOW_EVIDENCE") != "1":
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
    pats = ["CONSENSUS FAILURE", "panicked"]
    if os.environ.get("ALLOW_EVIDENCE") != "1":
        pats += ["conflicting votes", "found conflicting vote", "double sign"]
    for pat in pats:
        if pat.lower() in text.lower():
            fail.append(f"node {i} log has '{pat}'")
print(f"[{label}] heights 1..{upto} checked against go node {ref}")
for a, (i, k) in sorted(vals.items(), key=lambda x: x[1]):
    if a in proposed or a in signed:
        print(f"[{label}]   node {i} ({k}) {a[:12]}: proposed {proposed.get(a, 0)}, signed {signed.get(a, 0)}")
json.dump({"proposed": proposed, "signed": signed, "vals": vals, "evidence": evidence}, open(d + f"/stats-{label}.json", "w"))
for f in fail:
    print(f"[{label}] FAIL: {f}")
sys.exit(1 if fail else 0)
PY
}

# ---------------------------------------------------------------- workload
workload() { # <dir> <rpc-node-index> <blob-rpc-node-index>
  local dir=$1 node="tcp://127.0.0.1:$((26657 + OFF + 10 * $2))" h0 a k out v2 v3 v0
  local bnode="tcp://127.0.0.1:$((26657 + OFF + 10 * $3))"
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
  HOME_DIR=$h0 CHAIN_ID=$CHAIN_ID NODE=$bnode bash "$ROOT/conformance/devnet/workloads/blob.sh" > "$dir/blob.log" 2>&1 || echo "blob workload exited with $?"
  grep -E "^pfb" "$dir/blob.log" | head -30 || true
  for i in $(seq 1 6); do send user0 bank send "$u0" "$u1" "$((3000 + i))utia"; sleep 1; done
}

