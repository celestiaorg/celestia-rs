#!/usr/bin/env bash
# Generate a devnet differential fixture: run Go celestia-appd (standalone
# ABCI) behind Go celestia-core with the abci-recorder proxy in between, run
# an optional workload, then dump committed state.
#
# usage: [DEVNET_PORT_OFFSET=n] [CHAIN_ID=id] [GENESIS_SETUP=script.sh] [GENESIS_PATCH=script.py] \
#          conformance/devnet/devnet.sh <scenario> <blocks> [workload-script]
#
# ALLOW_HALT=1 accepts a workload that halts the chain (the fixture ends at
# the last committed height; the halting FinalizeBlock is recorded).
#
# GENESIS_SETUP (bash) runs after the default keys and genesis accounts are
# added and before gentx, with HOME_DIR and CHAIN_ID set (e.g. to add
# vesting accounts). GENESIS_PATCH (python) is called with the path of the
# final SDK genesis.json and edits it in place (e.g. short gov voting
# periods). Both default to `workloads/<workload>.setup.sh` /
# `workloads/<workload>.genesis.py` when those files exist.
#
# Output: conformance/fixtures/devnet/<scenario>/
#   genesis.json        SDK app genesis fed to InitChain
#   abci.rec            every ABCI frame (abci_server::record format)
#   state/<h>.json      devnet-oracle statedump at heights 1 and <blocks>
#   meta.json           versions used
# SAVE_BLOCKS=1 also retains blocks.bin.gz: the stopped Go node's complete
# blocks and seen commits, for validated node-store / handshake replay.
#
# APP_V9=1 runs celestia-app v9.0.8 (app version 9; binary cached as
# $DEVNET_BIN/v9/celestia-appd, which also serves the workload's CLI).
# UPGRADE_TO_V10=1 (with APP_V9=1) reproduces the multiplexer's binary
# switch: when a FinalizeBlock response raises the app version, the
# recorder stops the v9 app after its Commit and starts the v10 app on the
# same home and address (see abci-recorder).
#
# Requires Go and cargo. Binaries are cached in $DEVNET_BIN (default
# target/devnet-bin). The workload script (if any) is run with
# HOME_DIR, CHAIN_ID, NODE (rpc URL) and PATH set; it may submit txs.
set -euo pipefail

SCENARIO=${1:?scenario}
BLOCKS=${2:?blocks}
WORKLOAD=${3:-}
APP_VERSION=v10.5.0
[ -n "${APP_V9:-}" ] && APP_VERSION=v9.0.8
CORE_VERSION=v0.42.3
CHAIN_ID=${CHAIN_ID:-rs-devnet-1}
# Shift every listening port so concurrent devnets don't collide.
OFF=${DEVNET_PORT_OFFSET:-0}
P_RPC=$((26657 + OFF)); P_P2P=$((26656 + OFF)); P_PROXY=$((26658 + OFF)); P_APP=$((36658 + OFF))
P_GRPC=$((9090 + OFF)); P_API=$((1317 + OFF)); P_PPROF=$((6060 + OFF)); P_PRIVVAL_GRPC=$((26669 + OFF))

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
WORK=$(mktemp -d)
OUT=$ROOT/conformance/fixtures/devnet/$SCENARIO
mkdir -p "$BIN"
export PATH=$BIN:$PATH

build_go() { # <module@version> <pkg> <name>
  if [ -x "$BIN/$3" ] && [ "$(cat "$BIN/$3.module" 2>/dev/null)" = "$1" ]; then
    return
  fi
  local src
  src=$(cd "$ROOT/conformance/devnet-oracle" && go mod download -json "$1" | sed -n 's/.*"Dir": "\(.*\)",/\1/p')
  rm -rf "$WORK/src" && cp -r "$src" "$WORK/src" && chmod -R u+w "$WORK/src"
  (cd "$WORK/src" && GOFLAGS=-mod=mod go build -buildvcs=false -o "$BIN/$3" "$2")
  printf '%s\n' "$1" >"$BIN/$3.module"
}
build_go github.com/celestiaorg/celestia-app/v10@v10.5.0 ./cmd/celestia-appd celestia-appd
if [ -n "${APP_V9:-}" ]; then
  mkdir -p "$BIN/v9"
  build_go github.com/celestiaorg/celestia-app/v9@v9.0.8 ./cmd/celestia-appd v9/celestia-appd
  export PATH=$BIN/v9:$PATH
fi
build_go github.com/celestiaorg/celestia-core@$CORE_VERSION ./cmd/cometbft cometbft
[ -x "$BIN/devnet-oracle" ] || (cd "$ROOT/conformance/devnet-oracle" && go build -o "$BIN/devnet-oracle" .)
[ -x "$BIN/abci-recorder" ] || (cd "$ROOT" && cargo build -q --release -p abci-server --bin abci-recorder && cp target/release/abci-recorder "$BIN/")
if [ -n "${SAVE_BLOCKS:-}" ] && [ ! -x "$BIN/rpc-recorder" ]; then
  (cd "$ROOT/conformance/rpccorpus/recorder" && GOFLAGS=-mod=mod go build -o "$BIN/rpc-recorder" .)
fi

H=$WORK/home
A="celestia-appd --home $H"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A keys add user0 --keyring-backend test >/dev/null 2>&1
for k in val0 user0; do
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
if [ -n "$WORKLOAD" ]; then
  GENESIS_SETUP=${GENESIS_SETUP:-$(ls "${WORKLOAD%.sh}.setup.sh" 2>/dev/null || true)}
  GENESIS_PATCH=${GENESIS_PATCH:-$(ls "${WORKLOAD%.sh}.genesis.py" 2>/dev/null || true)}
fi
if [ -n "${GENESIS_SETUP:-}" ]; then
  HOME_DIR=$H CHAIN_ID=$CHAIN_ID bash "$GENESIS_SETUP"
fi
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
if [ -n "${GENESIS_PATCH:-}" ]; then
  python3 "$GENESIS_PATCH" "$H/config/genesis.json"
fi
sed -i.bak 's/^pruning = "default"/pruning = "nothing"/' "$H/config/app.toml"
rm -f "$H/config/app.toml.bak"
sed -i.bak -e "s#tcp://127.0.0.1:26657#tcp://127.0.0.1:$P_RPC#" -e "s#tcp://0.0.0.0:26656#tcp://0.0.0.0:$P_P2P#" \
  -e "s#localhost:6060#localhost:$P_PPROF#" -e "s#127.0.0.1:26669#127.0.0.1:$P_PRIVVAL_GRPC#" -e "s#127.0.0.1:26659#127.0.0.1:$P_PRIVVAL_GRPC#" \
  -e "s#tcp://127.0.0.1:9098#tcp://127.0.0.1:$((9098 + OFF))#" "$H/config/config.toml"
rm -f "$H/config/config.toml.bak"
sed -i.bak -e "s#localhost:9090#localhost:$P_GRPC#" -e "s#tcp://localhost:1317#tcp://localhost:$P_API#" -e "s#0.0.0.0:9090#0.0.0.0:$P_GRPC#" "$H/config/app.toml"
rm -f "$H/config/app.toml.bak"
# Workloads that need tx results over RPC (e.g. the relayer) set
# DEVNET_TX_INDEX=kv; indexing is node-local and does not change ABCI traffic.
if [ -n "${DEVNET_TX_INDEX:-}" ]; then
  sed -i.bak -e "s#^indexer = \"null\"#indexer = \"$DEVNET_TX_INDEX\"#" \
    -e "s#^discard_abci_responses = true#discard_abci_responses = false#" "$H/config/config.toml"
  rm -f "$H/config/config.toml.bak"
fi

# celestia-core wants a CometBFT GenesisDoc, not the SDK app genesis.
mkdir -p "$H/core/config" "$H/core/data"
python3 - "$H" <<'PY'
import json, sys
h = sys.argv[1]
g = json.load(open(h + "/config/genesis.json"))
c = {"genesis_time": g["genesis_time"], "chain_id": g["chain_id"],
     "initial_height": str(g["initial_height"]),
     "consensus_params": g["consensus"]["params"],
     "validators": g["consensus"].get("validators") or [],
     "app_hash": g.get("app_hash") or "", "app_state": g["app_state"]}
json.dump(c, open(h + "/core/config/genesis.json", "w"), indent=2)
PY
cp "$H"/config/{config.toml,node_key.json,priv_validator_key.json} "$H/core/config/"
echo '{"height":"0","round":0,"step":0}' > "$H/core/data/priv_validator_state.json"

PIDS=()
cleanup() {
  kill "${PIDS[@]}" 2>/dev/null || true
  # The v10 app started by an UPGRADE_TO_V10 switch (also on failure).
  if [ -f "$WORK/app-v10.pid" ]; then kill "$(cat "$WORK/app-v10.pid")" 2>/dev/null || true; fi
}
trap cleanup EXIT
celestia-appd start --force-no-bbr --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP > "$WORK/app.log" 2>&1 &
APP_PID=$!; PIDS+=($APP_PID); sleep 4
SWITCH=()
if [ -n "${UPGRADE_TO_V10:-}" ]; then
  # Run by the recorder inside the held Commit: stop v9 (its last commit is
  # on disk), start the v10 app on the same home and address.
  cat > "$WORK/switch.sh" <<SH
set -e
kill $APP_PID; while kill -0 $APP_PID 2>/dev/null; do sleep 0.2; done
"$BIN/celestia-appd" start --force-no-bbr --home "$H" --with-comet=false --address tcp://127.0.0.1:$P_APP >> "$WORK/app-v10.log" 2>&1 &
echo \$! > "$WORK/app-v10.pid"
until (exec 3<>/dev/tcp/127.0.0.1/$P_APP) 2>/dev/null; do sleep 0.2; done
SH
  SWITCH=("bash $WORK/switch.sh")
fi
abci-recorder 127.0.0.1:$P_PROXY 127.0.0.1:$P_APP "$WORK/abci.rec" "${SWITCH[@]}" > "$WORK/rec.log" 2>&1 &
PIDS+=($!); sleep 1
cometbft start --home "$H/core" --proxy_app tcp://127.0.0.1:$P_PROXY > "$WORK/core.log" 2>&1 &
CORE=$!; PIDS+=($CORE)

height() { curl -s localhost:$P_RPC/status | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
until [ "$(height || echo 0)" -ge 2 ] 2>/dev/null; do kill -0 $CORE || { tail "$WORK/core.log"; exit 1; }; sleep 1; done
if [ -n "$WORKLOAD" ]; then
  HOME_DIR=$H CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:$P_RPC bash "$WORKLOAD"
fi
until [ "$(height || echo 0)" -ge "$BLOCKS" ] 2>/dev/null; do
  if ! kill -0 $CORE 2>/dev/null; then
    # ALLOW_HALT=1: a workload that halts the chain (e.g. a missing upgrade
    # handler) ends the run where Go stopped.
    [ -n "${ALLOW_HALT:-}" ] && break
    tail "$WORK/core.log"
    exit 1
  fi
  [ -n "${ALLOW_HALT:-}" ] && [ -n "$WORKLOAD" ] && break
  sleep 1
done
# Stop consensus first so the app commits nothing past what core saw.
kill $CORE 2>/dev/null || true; sleep 2; cleanup
if [ -n "${SAVE_BLOCKS:-}" ]; then
  # The dumper opens the stopped core's Pebble store exclusively.
  wait "$CORE" 2>/dev/null || true
fi
[ -f "$WORK/app-v10.pid" ] && kill "$(cat "$WORK/app-v10.pid")" 2>/dev/null || true
sleep 1
LAST=$(curl -s localhost:$P_RPC/status >/dev/null 2>&1 || true; python3 - "$WORK/abci.rec" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
# Count Commit requests (Request oneof tag 11 => 0x5a) on any connection.
i = n = 0
while i < len(d):
    conn, dr, ln = struct.unpack("<IBI", d[i:i+9]); body = d[i+9:i+9+ln]; i += 9 + ln
    if dr == 0 and body[:1] == b"\x5a": n += 1
print(n)
PY
)


# CTYPES_BLOCKS=1 also dumps the block store (blocks, commits, validator
# sets, params) as ctypes vectors; CTYPES_ONLY=1 skips the devnet fixture.
if [ -n "${CTYPES_BLOCKS:-}${CTYPES_ONLY:-}" ]; then
  (cd "$ROOT/conformance/ctypesvectors" && GOFLAGS=-mod=mod go run . \
    -out "$ROOT/conformance/fixtures/ctypes" -blockstore "$H/core/data" -scenario "$SCENARIO")
fi
if [ -n "${CTYPES_ONLY:-}" ]; then
  rm -rf "$WORK"
  exit 0
fi

rm -rf "$OUT" && mkdir -p "$OUT/state"
if [ -n "${SAVE_BLOCKS:-}" ]; then
  "$BIN/rpc-recorder" dump -data "$H/core/data" -out "$OUT/blocks.bin.gz"
  go version -m "$(command -v celestia-appd)" "$BIN/cometbft" "$BIN/rpc-recorder" > "$OUT/go-build-info.txt"
  # Copied module-cache sources appear as (devel) in Go build metadata;
  # preserve build_go's pinned source-module receipts alongside it.
  for binary in "$(command -v celestia-appd)" "$BIN/cometbft"; do
    printf 'source-module\t%s\t%s\n' "$binary" "$(cat "$binary.module")" >> "$OUT/go-build-info.txt"
  done
  if [ -n "${UPGRADE_TO_V10:-}" ]; then
    go version -m "$BIN/celestia-appd" >> "$OUT/go-build-info.txt"
    printf 'source-module\t%s\t%s\n' "$BIN/celestia-appd" "$(cat "$BIN/celestia-appd.module")" >> "$OUT/go-build-info.txt"
  fi
fi
cp "$H/config/genesis.json" "$OUT/genesis.json"
cp "$WORK/abci.rec" "$OUT/abci.rec"
for h in 1 "$LAST"; do
  devnet-oracle statedump --db "$H/data/application.db" --height "$h" > "$OUT/state/$h.json"
done
cat > "$OUT/meta.json" <<JSON
{
  "scenario": "$SCENARIO",
  "chain_id": "$CHAIN_ID",
  "celestia_app": "$APP_VERSION",
  "celestia_app_next": $(if [ -n "${UPGRADE_TO_V10:-}" ]; then echo '"v10.5.0"'; else echo null; fi),
  "celestia_core": "$CORE_VERSION",
  "committed_heights": $LAST,
  "workload": "$(basename "${WORKLOAD:-none}")",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
rm -rf "$WORK"
echo "wrote $OUT (heights 1..$LAST)"
