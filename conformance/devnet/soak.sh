#!/usr/bin/env bash
# Long-running soak + chaos test of a mixed Go / Rust network.
#
# Topology: validators V0, V1 (Go celestia-appd v10.4.0) and V2, V3, V4
# (Rust `celestia-rs start`) with equal power, a Go full node F5 and a Rust
# full node F6. Every P2P link goes through `soak/netproxy.py` (one
# listener per directed node pair), so links can be cut or slowed without
# iptables.
#
# `soak/soak.py` then runs for $SOAK_SECS (default 7200) and at least
# $SOAK_MIN_HEIGHTS heights (default 1500):
#
# - workload: bank sends at volume (Python-signed MsgSend txs broadcast
#   round-robin to every node's RPC), PFBs (CLI; medium rounds and
#   near-max rounds filling 128x128..256x256 squares), staking churn
#   (delegate / redelegate / unbond on all validators), gov proposals
#   (MsgUpdateBlobParams, yes / no votes from the validators);
# - chaos, one disruptive event at a time: SIGKILL + restart of Rust
#   validators (WAL recovery) and the Rust full node, isolating a node
#   (all its links cut), added link latency, a minority/majority split
#   that halts the chain until healed, a Rust validator wiped and
#   restarted from state sync (Go snapshots), and a Rust validator
#   unbonding its whole self-delegation (leaves the set) and re-bonding
#   + unjailing later;
# - continuous checks: identical block / app hashes on every node at every
#   height they hold, no evidence in any block and no double-sign log
#   lines, no stall > 60 s outside the planned halt, the Go full node never
#   rejects a block (Rust-proposed blocks are counted), RSS of every node
#   sampled every 15 s and disk usage every 60 s.
#
# Writes $W/report.json and $W/report.md (plus rss.csv, disk.csv,
# events.log); exits 0 iff every check passed.
#
# usage: [DEVNET_PORT_OFFSET=19000] [DEVNET_BIN=dir] [SOAK_SECS=7200]
#        [SOAK_MIN_HEIGHTS=1500] [SOAK_SEED=n] [CELESTIA_RS=path]
#        [SOAK_DIR=dir] [SOAK_PROFILE=chaos|memory] [SOAK_MEM_SAMPLE=30]
#        [SOAK_RS_N<i>=path] [SOAK_ENV_N<i>="K=V;K2=V2"] conformance/devnet/soak.sh
#
# SOAK_PROFILE=memory: 2 Go + 2 Rust validators + a Rust full node, the
# same workload, no chaos (single node lifetimes) and per-node memory
# samples (mem.jsonl); SOAK_RS_N<i> / SOAK_ENV_N<i> run node i on another
# binary / with extra environment (e.g. a `--features memstats` build with
# MALLOC_CONF heap profiling).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-19000}
if [ -z "${CELESTIA_RS:-}" ]; then
  (cd "$ROOT" && cargo build -q --release -p celestia-rs)
  CELESTIA_RS=$ROOT/target/release/celestia-rs
fi
RS=$CELESTIA_RS
W=${SOAK_DIR:-$(mktemp -d)}
mkdir -p "$W"
FAIL=()
CHAIN_ID=rs-soak-1
echo "soak dir: $W"

# shellcheck source=lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"
trap - EXIT

STAKE=5000000000
if [ "${SOAK_PROFILE:-chaos}" = memory ]; then
  # Memory soak (docs/benchmarks/memory.md): V0, V1 Go, V2, V3 Rust
  # validators, a Rust full node F4; no chaos, so every node keeps one
  # lifetime; memory sampled every $SOAK_MEM_SAMPLE s into mem.jsonl. V1
  # holds the tx index and is the hash reference.
  KINDS="go go rs rs rs"
  make_net "$W" "$CHAIN_ID" "$KINDS" "$STAKE $STAKE $STAKE $STAKE 0"
  N=5
  PROFILE_ARGS=(--no-chaos --validators 0,1,2,3 --ref-node 1 --mem-sample "${SOAK_MEM_SAMPLE:-30}")
else
  KINDS="go go rs rs rs go rs"
  make_net "$W" "$CHAIN_ID" "$KINDS" "$STAKE $STAKE $STAKE $STAKE $STAKE 0 0"
  N=7
  PROFILE_ARGS=()
fi

# Workload accounts in node 0's keyring (CLI txs), funded in genesis.
a="celestia-appd --home $W/n0"
for k in blob1 blob2 blob3 blob4 blob5 blob6 blob7 blob8 stake1 stake2 stake3 gov1; do
  $a keys add $k --keyring-backend test >/dev/null 2>&1
  $a genesis add-genesis-account "$($a keys show $k -a --keyring-backend test)" 100000000000000utia >/dev/null 2>&1
done
# Bank-load accounts (keys derived by soak.py from fixed seeds).
python3 "$ROOT/conformance/devnet/soak/soak.py" --print-load-addresses --load-accounts "${SOAK_LOAD_ACCOUNTS:-48}" |
  while read -r addr; do
    $a genesis add-genesis-account "$addr" 10000000000000utia >/dev/null 2>&1
  done
# Short gov periods so proposals finish during the soak.
python3 - "$W/n0/config/genesis.json" <<'PY'
import json, sys
p = sys.argv[1]
g = json.load(open(p))
gp = g["app_state"]["gov"]["params"]
gp["min_deposit"] = [{"denom": "utia", "amount": "10000000"}]
gp["expedited_min_deposit"] = [{"denom": "utia", "amount": "50000000"}]
gp["max_deposit_period"] = "60s"
gp["voting_period"] = "60s"
gp["expedited_voting_period"] = "30s"
json.dump(g, open(p, "w"), indent=1)
PY
for i in $(seq 1 $((N - 1))); do cp "$W/n0/config/genesis.json" "$W/n$i/config/genesis.json"; done
# Bounded disk on every node (Go and Rust read app.toml): prune app state
# to the last 100 versions and blocks below 500 heights back. Note that
# celestia-app raises min-retain-blocks to at least 3000 (both nodes log
# "Overriding min-retain-blocks"), so block stores only start shrinking
# after 3000 heights (~2.5 h at the soak's block time).
for i in $(seq 0 $((N - 1))); do
  sed -i.bak -e 's#^pruning = .*#pruning = "custom"#' -e 's#^pruning-keep-recent = .*#pruning-keep-recent = "100"#' \
    -e 's#^pruning-interval = .*#pruning-interval = "10"#' -e "s#^min-retain-blocks = .*#min-retain-blocks = ${SOAK_RETAIN_BLOCKS:-500}#" \
    "$W/n$i/config/app.toml"
  rm -f "$W/n$i/config/app.toml.bak"
done
# Go nodes take snapshots (state-sync source for the Rust validators); in
# the memory profile every node does (snapshot creation is part of the
# steady state being measured).
for i in $(seq 0 $((N - 1))); do
  [ "$(cat "$W/n$i/kind")" = go ] || [ "${SOAK_PROFILE:-chaos}" = memory ] || continue
  sed -i.bak -e 's#^snapshot-interval = .*#snapshot-interval = 100#' -e 's#^snapshot-keep-recent = .*#snapshot-keep-recent = 3#' "$W/n$i/config/app.toml"
  rm -f "$W/n$i/config/app.toml.bak"
done

exec python3 "$ROOT/conformance/devnet/soak/soak.py" --dir "$W" --off "$OFF" --rs "$RS" --chain-id "$CHAIN_ID" \
  --secs "${SOAK_SECS:-7200}" --min-heights "${SOAK_MIN_HEIGHTS:-1500}" --seed "${SOAK_SEED:-1}" \
  --load-accounts "${SOAK_LOAD_ACCOUNTS:-48}" --root "$ROOT" "${PROFILE_ARGS[@]}"
