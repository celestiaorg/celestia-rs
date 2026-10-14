#!/usr/bin/env bash
# Generate a devnet differential fixture: run Go celestia-appd (standalone
# ABCI) behind Go celestia-core with the abci-recorder proxy in between, run
# an optional workload, then dump committed state.
#
# usage: conformance/devnet/devnet.sh <scenario> <blocks> [workload-script]
#
# Output: conformance/fixtures/devnet/<scenario>/
#   genesis.json        SDK app genesis fed to InitChain
#   abci.rec            every ABCI frame (abci_server::record format)
#   state/<h>.json      devnet-oracle statedump at heights 1 and <blocks>
#   meta.json           versions used
#
# Requires Go and cargo. Binaries are cached in $DEVNET_BIN (default
# target/devnet-bin). The workload script (if any) is run with
# HOME_DIR, CHAIN_ID, NODE (rpc URL) and PATH set; it may submit txs.
set -euo pipefail

SCENARIO=${1:?scenario}
BLOCKS=${2:?blocks}
WORKLOAD=${3:-}
APP_VERSION=v10.4.0
CORE_VERSION=v0.42.3
CHAIN_ID=rs-devnet-1

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
WORK=$(mktemp -d)
OUT=$ROOT/conformance/fixtures/devnet/$SCENARIO
mkdir -p "$BIN"
export PATH=$BIN:$PATH

build_go() { # <module@version> <pkg> <name>
  [ -x "$BIN/$3" ] && return
  local src
  src=$(cd "$ROOT/conformance/devnet-oracle" && go mod download -json "$1" | sed -n 's/.*"Dir": "\(.*\)",/\1/p')
  rm -rf "$WORK/src" && cp -r "$src" "$WORK/src" && chmod -R u+w "$WORK/src"
  (cd "$WORK/src" && GOFLAGS=-mod=mod go build -o "$BIN/$3" "$2")
}
build_go github.com/celestiaorg/celestia-app/v10@$APP_VERSION ./cmd/celestia-appd celestia-appd
build_go github.com/celestiaorg/celestia-core@$CORE_VERSION ./cmd/cometbft cometbft
[ -x "$BIN/devnet-oracle" ] || (cd "$ROOT/conformance/devnet-oracle" && go build -o "$BIN/devnet-oracle" .)
(cd "$ROOT" && cargo build -q --release -p abci-server --bin abci-recorder && cp target/release/abci-recorder "$BIN/")

H=$WORK/home
A="celestia-appd --home $H"
$A init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$A keys add val0 --keyring-backend test >/dev/null 2>&1
$A keys add user0 --keyring-backend test >/dev/null 2>&1
for k in val0 user0; do
  $A genesis add-genesis-account "$($A keys show $k -a --keyring-backend test)" 1000000000000utia >/dev/null 2>&1
done
$A genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$A genesis collect-gentxs >/dev/null 2>&1
sed -i 's/^pruning = "default"/pruning = "nothing"/' "$H/config/app.toml"

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
cleanup() { kill "${PIDS[@]}" 2>/dev/null || true; }
trap cleanup EXIT
celestia-appd start --home "$H" --with-comet=false --address tcp://127.0.0.1:36658 > "$WORK/app.log" 2>&1 &
PIDS+=($!); sleep 4
abci-recorder 127.0.0.1:26658 127.0.0.1:36658 "$WORK/abci.rec" > "$WORK/rec.log" 2>&1 &
PIDS+=($!); sleep 1
cometbft start --home "$H/core" --proxy_app tcp://127.0.0.1:26658 > "$WORK/core.log" 2>&1 &
CORE=$!; PIDS+=($CORE)

height() { curl -s localhost:26657/status | sed -n 's/.*"latest_block_height":"\([0-9]*\)".*/\1/p'; }
until [ "$(height || echo 0)" -ge 2 ] 2>/dev/null; do kill -0 $CORE || { tail "$WORK/core.log"; exit 1; }; sleep 1; done
if [ -n "$WORKLOAD" ]; then
  HOME_DIR=$H CHAIN_ID=$CHAIN_ID NODE=tcp://127.0.0.1:26657 bash "$WORKLOAD"
fi
until [ "$(height || echo 0)" -ge "$BLOCKS" ] 2>/dev/null; do kill -0 $CORE || { tail "$WORK/core.log"; exit 1; }; sleep 1; done
# Stop consensus first so the app commits nothing past what core saw.
kill $CORE; sleep 2; cleanup; sleep 1
LAST=$(curl -s localhost:26657/status >/dev/null 2>&1 || true; python3 - "$WORK/abci.rec" <<'PY'
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

rm -rf "$OUT" && mkdir -p "$OUT/state"
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
  "celestia_core": "$CORE_VERSION",
  "committed_heights": $LAST,
  "workload": "$(basename "${WORKLOAD:-none}")",
  "generated_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
}
JSON
rm -rf "$WORK"
echo "wrote $OUT (heights 1..$LAST)"
