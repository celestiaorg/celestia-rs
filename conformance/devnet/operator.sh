#!/usr/bin/env bash
# M12 operator flow, driven only through the Rust CLI and checked by Go
# celestia-appd v10.4.0:
#
#  1. `celestia-rs init` + `keys add` (test keyring) + `keys export` /
#     `keys import` + `genesis add-genesis-account` / `gentx` /
#     `collect-gentxs` / `validate`; Go reads the keyring and validates
#     the genesis.
#  2. `celestia-rs start` (the only validator, Prometheus on); a Go full
#     node (`celestia-appd init` + the Rust genesis) follows it over P2P.
#  3. bank send, delegate and pay-for-blob signed and broadcast by
#     `celestia-rs tx`; the Go node executes them (query via Go).
#  4. SIGTERM, `celestia-rs rollback`, restart: the Rust validator
#     re-executes and re-signs the same block, the Go node keeps following
#     (same block hashes, no evidence).
#  5. Both nodes stopped; `celestia-rs export` vs `celestia-appd export`
#     at the same height: identical app_state, validators and params.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/operator.sh
# Exit 0 iff all checks pass.
set -euo pipefail
[ -n "${DEBUG:-}" ] && set -x
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=$((${DEVNET_PORT_OFFSET:-16000} + 600))
CHAIN_ID=rs-operator-1
W=$(mktemp -d)
R=$W/rs
G=$W/go
RS="$ROOT/target/debug/celestia-rs"
FAIL=()
PIDS=()
cleanup() {
  for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

(cd "$ROOT" && CARGO_INCREMENTAL=0 cargo build -q -p celestia-rs)

shift_ports() { # <file> <delta>
  python3 - "$1" "$2" <<'PY'
import re, sys
p, d = sys.argv[1], int(sys.argv[2])
s = open(p).read()
s = re.sub(r'((?:127\.0\.0\.1|0\.0\.0\.0|localhost):)(\d{4,5})', lambda m: m.group(1) + str(int(m.group(2)) + d), s)
open(p, "w").write(s)
PY
}
rpc_r="http://127.0.0.1:$((26657 + OFF))"
rpc_g="http://127.0.0.1:$((26657 + OFF + 10))"
height() { curl -s "$1/status" | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p' | grep . || echo 0; }
wait_height() { # <rpc> <height> <timeout>
  local i
  for i in $(seq 1 "$3"); do
    [ "$(height "$1")" -ge "$2" ] && return 0
    sleep 1
  done
  return 1
}
block_hash() { curl -s "$1/block?height=$2" | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["block_id"]["hash"])'; }

# ---------------------------------------------------------------- 1. setup
echo "== init / keys / genesis (Rust CLI)"
K=(--home "$R" --keyring-backend test)
$RS init val --chain-id $CHAIN_ID --home "$R" 2>/dev/null
$RS keys add validator "${K[@]}" --output json > "$W/validator.json"
$RS keys add user "${K[@]}" --output json > "$W/user.json"
VAL=$($RS keys show validator -a "${K[@]}")
USER=$($RS keys show user -a "${K[@]}")
VALOPER=$($RS keys show validator -a --bech val "${K[@]}")
printf 'exportpass\n' | $RS keys export user "${K[@]}" > "$W/user.armor" 2>/dev/null
printf 'exportpass\n' | $RS keys import user2 "$W/user.armor" "${K[@]}" 2>/dev/null
[ "$($RS keys show user2 -a "${K[@]}")" = "$USER" ] || FAIL+=("keys export/import changed the address")
$RS keys delete user2 -y "${K[@]}" 2>/dev/null
celestia-appd keys list --keyring-backend test --home "$R" --output json > "$W/go-keys.json"
python3 - "$W/go-keys.json" "$VAL" "$USER" <<'PY' || FAIL+=("Go cannot read the Rust keyring")
import json, sys
keys = {k["name"]: k["address"] for k in json.load(open(sys.argv[1]))}
assert keys == {"validator": sys.argv[2], "user": sys.argv[3]}, keys
print("Go reads the Rust keyring:", sorted(keys))
PY
$RS genesis add-genesis-account validator 1000000000000utia "${K[@]}"
$RS genesis add-genesis-account "$USER" 500000000utia --home "$R"
$RS genesis gentx validator 5000000000utia "${K[@]}" --ip 127.0.0.1 --p2p-port $((26656 + OFF)) 2>/dev/null
$RS genesis collect-gentxs --home "$R" 2>/dev/null
$RS genesis validate --home "$R"
celestia-appd genesis validate --home "$R" || FAIL+=("Go rejects the Rust genesis")

for f in config.toml app.toml client.toml; do shift_ports "$R/config/$f" "$OFF"; done
$RS config set config tx_index.indexer kv --home "$R"
$RS config set config instrumentation.prometheus true --home "$R"
$RS config set config instrumentation.prometheus_listen_addr "127.0.0.1:$((26660 + OFF))" --home "$R"
$RS config set config p2p.allow_duplicate_ip true --home "$R"
$RS config set config p2p.addr_book_strict false --home "$R"
$RS config set config p2p.pex false --home "$R"

# Go full node with the Rust genesis.
celestia-appd init gofull --chain-id $CHAIN_ID --home "$G" >/dev/null 2>&1
cp "$R/config/genesis.json" "$G/config/genesis.json"
for f in config.toml app.toml; do shift_ports "$G/config/$f" $((OFF + 10)); done
RID=$($RS comet show-node-id --home "$R")
sed -i.bak -e "s#^persistent_peers = .*#persistent_peers = \"$RID@127.0.0.1:$((26656 + OFF))\"#" \
  -e 's#^allow_duplicate_ip = false#allow_duplicate_ip = true#' -e 's#^addr_book_strict = true#addr_book_strict = false#' \
  -e 's#^pex = true#pex = false#' -e 's#^indexer = "null"#indexer = "kv"#' "$G/config/config.toml"
rm -f "$G/config/config.toml.bak"

# ---------------------------------------------------------------- 2. start
echo "== start (Rust validator, Go full node)"
start_rs() { RUST_LOG=${RUST_LOG:-info} "$RS" start --home "$R" >> "$W/rs.log" 2>&1 & RSPID=$!; PIDS+=($RSPID); }
start_rs
celestia-appd start --force-no-bbr --home "$G" >> "$W/go.log" 2>&1 & GOPID=$!; PIDS+=($GOPID)
wait_height "$rpc_r" 4 120 || FAIL+=("rust validator did not reach height 4")
wait_height "$rpc_g" 4 120 || FAIL+=("go full node did not follow to height 4")
[ "$(block_hash "$rpc_r" 3)" = "$(block_hash "$rpc_g" 3)" ] || FAIL+=("block 3 differs between Rust and Go")
curl -s "http://127.0.0.1:$((26660 + OFF))/metrics" > "$W/metrics.txt"
for m in cometbft_consensus_height cometbft_consensus_latest_block_height cometbft_p2p_peers cometbft_mempool_size \
  cometbft_consensus_block_interval_seconds_count cometbft_consensus_validator_last_signed_height; do
  grep -q "^$m{" "$W/metrics.txt" || FAIL+=("metric $m missing")
done
grep -q '^cometbft_p2p_peers{chain_id="'$CHAIN_ID'"} 1' "$W/metrics.txt" || FAIL+=("p2p_peers metric is not 1")

# ------------------------------------------------------------------ 3. txs
echo "== txs via the Rust CLI"
TX=(--home "$R" --keyring-backend test --wait)
SEND=$($RS tx bank send validator "$USER" 777utia "${TX[@]}" --fees 2000utia)
$RS tx staking delegate "$VALOPER" 1000000utia --from validator "${TX[@]}" --fees 2000utia > /dev/null
PFB=$($RS tx blob pay-for-blob 0a0b0c0d0e deadbeefcafebabe --from user "${TX[@]}")
SEND_HASH=$(echo "$SEND" | python3 -c 'import json,sys; print(json.load(sys.stdin)["txhash"])')
PFB_HASH=$(echo "$PFB" | python3 -c 'import json,sys; print(json.load(sys.stdin)["txhash"])')
H_TX=$(height "$rpc_r")
wait_height "$rpc_g" "$H_TX" 60 || FAIL+=("go node behind after the txs")
celestia-appd query bank balances "$USER" --node "tcp://127.0.0.1:$((26657 + OFF + 10))" -o json > "$W/go-bal.json"
python3 - "$W/go-bal.json" <<'PY' || FAIL+=("go node balance after the Rust-signed send")
import json, sys
b = {c["denom"]: int(c["amount"]) for c in json.load(open(sys.argv[1]))["balances"]}
# 500000000 genesis + 777 sent - PFB fee
assert 499990000 < b["utia"] < 500000777, b
print("go node sees the user balance", b)
PY
for h in "$SEND_HASH" "$PFB_HASH"; do
  celestia-appd query tx "$h" --node "tcp://127.0.0.1:$((26657 + OFF + 10))" -o json > "$W/go-tx.json" 2>&1 \
    && python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert d["code"]==0, d' "$W/go-tx.json" \
    || FAIL+=("go node: tx $h not committed with code 0")
done
celestia-appd query staking delegation "$VAL" "$VALOPER" --node "tcp://127.0.0.1:$((26657 + OFF + 10))" -o json | grep -q 5001000000 \
  || FAIL+=("go node does not see the Rust-signed delegation")
$RS query bank balances "$USER" --home "$R" > /dev/null || FAIL+=("rust query bank balances")
$RS query tx "$SEND_HASH" --home "$R" > /dev/null || FAIL+=("rust query tx")
$RS status --home "$R" > /dev/null || FAIL+=("rust status")

# ------------------------------------------------- 4. rollback and restart
echo "== SIGTERM, rollback, restart"
kill -TERM $RSPID
wait $RSPID || FAIL+=("rust node exit code after SIGTERM")
grep -q "node stopped" "$W/rs.log" || FAIL+=("no clean shutdown in the rust log")
TOP=$(sed 's/\x1b\[[0-9;]*m//g' "$W/rs.log" | sed -n 's/.*committed block height=\([0-9]*\).*/\1/p' | tail -n1)
ROLL=$($RS rollback --home "$R")
echo "$ROLL"
echo "$ROLL" | grep -q "Rolled back state to height $((TOP - 1)) " || FAIL+=("rollback did not go to $((TOP - 1)): $ROLL")
start_rs
wait_height "$rpc_r" $((TOP + 3)) 120 || FAIL+=("rust validator did not resume after the rollback")
wait_height "$rpc_g" $((TOP + 3)) 120 || FAIL+=("go node did not follow after the rollback")
for h in $((TOP - 1)) "$TOP" $((TOP + 1)) $((TOP + 2)); do
  [ "$(block_hash "$rpc_r" "$h")" = "$(block_hash "$rpc_g" "$h")" ] || FAIL+=("block $h differs after the rollback")
done
EV=$(curl -s "$rpc_g/block?height=$((TOP + 1))" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["block"]["evidence"]["evidence"] or []))')
[ "$EV" = 0 ] || FAIL+=("evidence after the rollback")

# ------------------------------------------------------------ 5. export
echo "== export (Rust vs Go)"
kill -TERM $RSPID; wait $RSPID || true
sleep 2
kill -TERM $GOPID; wait $GOPID || true
# Go's `export --height` panics in v10.4.0 ("baseapp already sealed"):
# export Go's latest height and the same height from the (later) Rust node.
celestia-appd export --home "$G" > "$W/go-export.json" 2>"$W/go-export.err" || FAIL+=("go export")
EH=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["initial_height"] - 1)' "$W/go-export.json")
$RS export --home "$R" --height "$EH" --output-document "$W/rs-export.json" 2>/dev/null || FAIL+=("rust export")
python3 - "$W/rs-export.json" "$W/go-export.json" <<'PY' || FAIL+=("export differs from Go")
import json, sys
r, g = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
bad = [k for k in ("chain_id", "initial_height", "app_state", "consensus") if r[k] != g[k]]
for m in sorted(set(r["app_state"]) | set(g["app_state"])):
    if r["app_state"].get(m) != g["app_state"].get(m):
        print("module differs:", m)
print("export at height", g["initial_height"] - 1, "identical" if not bad else f"differs in {bad}")
sys.exit(1 if bad else 0)
PY

trap - EXIT
cleanup
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "operator flow OK"
  rm -rf "$W"
  exit 0
fi
printf 'FAILED: %s\n' "${FAIL[@]}"
echo "logs: $W"
exit 1
