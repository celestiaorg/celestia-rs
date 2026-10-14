#!/usr/bin/env bash
# End-to-end check of the Rust consensus node against Go:
#
#   1. a single-validator genesis from Go `celestia-appd init/gentx/collect-gentxs`;
#   2. signed bank sends (from the Go CLI, offline) dropped into the Rust
#      node's tx inbox;
#   3. `celestia-rs start` produces <blocks> blocks (restarted halfway, so
#      the handshake and WAL replay run too);
#   4. `celestia-rs export-blocks` dumps the store;
#   5. the Go verifier checks every block, commit and hash with celestia-core
#      v0.42.3 code, replays the chain through a fresh Go celestia-appd over
#      ABCI (ProcessProposal must accept, app/results hashes must match) and
#      decodes the Rust WAL with Go's WALDecoder.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/consensusvectors/verify/run.sh [blocks] [txs]
set -euo pipefail

BLOCKS=${1:-12}
NTXS=${2:-6}
OFF=${DEVNET_PORT_OFFSET:-0}
P_APP=$((36658 + OFF)); P_GRPC=$((9090 + OFF)); P_API=$((1317 + OFF))
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
WORK=$(mktemp -d)
CHAIN_ID=rs-consensus-1
H=$WORK/rust
G=$WORK/go

APPD="celestia-appd --home $H"
$APPD init val0 --chain-id $CHAIN_ID >/dev/null 2>&1
$APPD keys add val0 --keyring-backend test >/dev/null 2>&1
$APPD keys add user0 --keyring-backend test >/dev/null 2>&1
VAL=$($APPD keys show val0 -a --keyring-backend test)
USER=$($APPD keys show user0 -a --keyring-backend test)
for a in $VAL $USER; do
  $APPD genesis add-genesis-account "$a" 1000000000000utia >/dev/null 2>&1
done
$APPD genesis gentx val0 5000000000utia --chain-id $CHAIN_ID --keyring-backend test --fees 500000utia >/dev/null 2>&1
$APPD genesis collect-gentxs >/dev/null 2>&1
# The Go app replays from a pristine copy of the same home.
cp -r "$H" "$G"

# Signed txs (offline): user0 -> val0 bank sends with consecutive sequences.
ACC=$(python3 - "$H/config/genesis.json" "$USER" <<'PY'
import json, sys
g = json.load(open(sys.argv[1]))
for a in g["app_state"]["auth"]["accounts"]:
    if a.get("address") == sys.argv[2]:
        print(a.get("account_number", "0")); break
PY
)
mkdir -p "$H/data/tx-inbox"
for i in $(seq 0 $((NTXS - 1))); do
  $APPD tx bank send "$USER" "$VAL" "$((1000 + i))utia" --chain-id $CHAIN_ID --keyring-backend test \
    --fees 20000utia --gas 200000 --generate-only > "$WORK/u$i.json" 2>/dev/null
  $APPD tx sign "$WORK/u$i.json" --from user0 --chain-id $CHAIN_ID --keyring-backend test \
    --offline --account-number "$ACC" --sequence "$i" > "$WORK/s$i.json" 2>/dev/null
  $APPD tx encode "$WORK/s$i.json" 2>/dev/null | base64 -d > "$H/data/tx-inbox/$(printf %04d "$i").tx"
done

(cd "$ROOT" && cargo build -q -p celestia-rs)
RS=$ROOT/target/debug/celestia-rs
HALF=$((BLOCKS / 2))
$RS start --home "$H" --halt-height "$HALF" > "$WORK/node1.log" 2>&1
$RS start --home "$H" --halt-height "$BLOCKS" > "$WORK/node2.log" 2>&1
grep -q "Catchup by replaying consensus messages" "$WORK/node2.log"
$RS export-blocks --home "$H" --out "$WORK/blocks.json" > /dev/null 2>&1

sed -i -e "s#localhost:9090#localhost:$P_GRPC#" -e "s#0.0.0.0:9090#0.0.0.0:$P_GRPC#" \
  -e "s#tcp://localhost:1317#tcp://localhost:$P_API#" "$G/config/app.toml"
celestia-appd start --home "$G" --with-comet=false --address tcp://127.0.0.1:$P_APP > "$WORK/app.log" 2>&1 &
APP=$!
trap 'kill $APP 2>/dev/null || true' EXIT
for _ in $(seq 1 60); do
  (exec 3<>/dev/tcp/127.0.0.1/$P_APP) 2>/dev/null && break
  sleep 1
done

cd "$ROOT/conformance/consensusvectors"
GOFLAGS=-mod=mod go run ./verify -blocks "$WORK/blocks.json" -genesis "$H/config/genesis.json" \
  -app tcp://127.0.0.1:$P_APP -wal "$H/data/cs.wal/wal"
echo "work dir: $WORK"
