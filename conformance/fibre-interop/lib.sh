# shellcheck shell=bash
# Helpers for the Fibre interoperability devnets (sourced by
# rust-validators.sh and mixed-validators.sh after
# conformance/devnet/lib-mixed.sh). Expects ROOT, OFF, RS, W, FAIL, GOBIN.
#
# Ports of node i: RPC 26657+OFF+10i, app gRPC 9090+OFF+10i, Fibre
# 7980+OFF+10i, privval gRPC (Go validators) 7985+OFF+10i.

grpc_addr() { echo "127.0.0.1:$((9090 + OFF + 10 * $1))"; }
fibre_addr() { echo "127.0.0.1:$((7980 + OFF + 10 * $1))"; }
privval_addr() { echo "127.0.0.1:$((7985 + OFF + 10 * $1))"; }
node_rpc() { echo "tcp://127.0.0.1:$((26657 + OFF + 10 * $1))"; }

# build_interop: the Go v11 client program, with the toolchain its go.mod
# pins (GOTOOLCHAIN=auto downloads it).
build_interop() {
  local mod=$ROOT/conformance/fibre-interop
  (cd "$mod" && GOTOOLCHAIN=${GOTOOLCHAIN:-go$(awk '$1 == "go" { print $2; exit }' go.mod)} GOFLAGS=-mod=mod \
    go build -o "$GOBIN/fibre-interop" .)
}

interop() { "$GOBIN/fibre-interop" "$@"; }

# set_genesis_budget <dir> <bytes>: x/fibre full_stake_storage_budget in
# node 0's genesis, then copy it to every node.
set_genesis_budget() {
  python3 - "$1/n0/config/genesis.json" "$2" <<'PY'
import json, sys
p = sys.argv[1]
g = json.load(open(p))
g["app_state"]["fibre"]["params"]["full_stake_storage_budget"] = sys.argv[2]
json.dump(g, open(p, "w"), indent=2)
PY
  local n
  for n in "$1"/n*; do [ "$n" = "$1/n0" ] || cp "$1/n0/config/genesis.json" "$n/config/genesis.json"; done
}

# enable_app_grpc <dir> <i>: the app gRPC server that clients and the Go
# fibre server query (off in the app.toml that `init` writes, Go and Rust).
enable_app_grpc() {
  sed -i '/^\[grpc\]/,/^\[/ s/^enable = false/enable = true/' "$1/n$2/config/app.toml"
  grep -q '^enable = true' <(sed -n '/^\[grpc\]/,/^\[grpc-web\]/p' "$1/n$2/config/app.toml")
}

# enable_rust_fibre <dir> <i>: the embedded server on 127.0.0.1, readiness
# checked every second, and the app gRPC server.
enable_rust_fibre() {
  enable_app_grpc "$1" "$2"
  cat >> "$1/n$2/config/config.toml" <<EOF

[fibre]
enable = true
server_listen_address = "$(fibre_addr "$2")"

[fibre.health]
check_interval = "1s"
EOF
}

# register_host <dir> <i> <cli>: tx valaddr set-host with the node's
# operator key (val<i>, in its own keyring), through its own RPC.
register_host() {
  local out
  out=$("$3" tx valaddr set-host "$(fibre_addr "$2")" --from "val$2" --home "$1/n$2" --keyring-backend test \
    --chain-id "$CHAIN_ID" --node "$(node_rpc "$2")" --fees 40000utia --gas 400000 -y -o json 2>&1) || true
  echo "$out" | python3 -c 'import json,sys; d=json.loads(sys.stdin.read().strip().splitlines()[-1]); sys.exit(0 if d.get("code", 1) == 0 else 1)' ||
    { echo "set-host for node $2 failed: ${out:0:400}"; return 1; }
}

# wait_healthy <timeout-s> <addr>...: readiness and liveness SERVING.
wait_healthy() {
  local t=$1 _
  shift
  local addrs
  addrs=$(IFS=,; echo "$*")
  for _ in $(seq 1 "$t"); do
    interop health -addrs "$addrs" > "$W/health.log" 2>&1 && { cat "$W/health.log"; return 0; }
    sleep 1
  done
  cat "$W/health.log"
  return 1
}

# chain_agree <upto> <nodes...>: every listed node serves the same
# block hash and post-commit app hash at every height up to <upto>, over
# RPC (works for Go and Rust nodes).
chain_agree() {
  python3 - "$OFF" "$1" "${@:2}" <<'PY'
import json, sys, urllib.request
off, upto, nodes = int(sys.argv[1]), int(sys.argv[2]), [int(x) for x in sys.argv[3:]]
def rpc(i, path):
    return json.load(urllib.request.urlopen(f"http://127.0.0.1:{26657 + off + 10 * i}{path}", timeout=10))["result"]
ref = {}
bad = []
for i in nodes:
    for h in range(1, upto + 2):
        b = rpc(i, f"/block?height={h}")
        got = (b["block_id"]["hash"], b["block"]["header"]["app_hash"])
        if h not in ref:
            ref[h] = (i, got)
        elif ref[h][1] != got:
            bad.append(f"height {h}: node {i} block {got[0][:12]} app {got[1][:12]} != node {ref[h][0]} block {ref[h][1][0][:12]} app {ref[h][1][1][:12]}")
print(f"chain: nodes {nodes} agree on block and app hashes at heights 1..{upto + 1}" if not bad else "\n".join(bad[:10]))
sys.exit(1 if bad else 0)
PY
}

# latest <i>: latest height from a node's RPC.
latest() {
  curl -s "http://127.0.0.1:$((26657 + OFF + 10 * $1))/status" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["sync_info"]["latest_block_height"])' 2>/dev/null || echo 0
}

# peak_rss <pid>: peak resident set (VmHWM) in MiB.
peak_rss() { awk '/VmHWM/ { printf "%d", $2 / 1024 }' "/proc/$1/status" 2>/dev/null || echo "?"; }

# signer_kinds <records> <dir>: per blob, how many Go and Rust validators
# signed (consensus addresses mapped through priv_validator_key.json).
signer_kinds() {
  python3 - "$1" "$2" <<'PY'
import glob, json, sys
kinds = {}
for n in glob.glob(sys.argv[2] + "/n*"):
    k = json.load(open(n + "/config/priv_validator_key.json"))
    kinds[k["address"].upper()] = open(n + "/kind").read().strip()
for line in open(sys.argv[1]):
    r = json.loads(line)
    ks = [kinds.get(s.upper(), "?") for s in r["signers"]]
    print(f"{r['size']} bytes ({r['via']}): {len(ks)}/{r['validators']} signatures, go={ks.count('go')} rs={ks.count('rs')}, "
          f"upload {r['upload_ms']} ms, pay {r['pay_ms']} ms, download {r['download_ms']} ms, height {r['height']}")
    per = r.get("per_validator_ms") or {}
    if per:
        print("    accepted after: " + ", ".join(f"{kinds.get(a.upper(), '?')} {a[:8]} {ms} ms" for a, ms in sorted(per.items(), key=lambda x: x[1])))
PY
}
