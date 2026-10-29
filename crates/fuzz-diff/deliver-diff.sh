#!/usr/bin/env bash
# Delivery differential (crates/fuzz-diff/src/deliver.rs): starts a Go
# celestia-appd as a standalone ABCI app, re-drives the recorded scenario
# into it and into the Rust app with funded fuzz accounts added to the
# genesis, then finalizes generated blocks of signed txs on both, comparing
# CheckTx, PrepareProposal, ProcessProposal, every tx result and the app
# hash of every block.
#
# usage: GO_APPD=<celestia-appd of the scenario's version> \
#        crates/fuzz-diff/deliver-diff.sh <scenario> [blocks] [txs-per-block] [seed]
#
# v10 scenarios need celestia-appd v10.5.0, v9-* scenarios v9.0.8 (both as
# built by conformance/devnet/devnet.sh into $DEVNET_BIN and $DEVNET_BIN/v9).
# Environment: DELIVER_TYPES=<substr,...> narrows the synthesized message
# types.
# DELIVER_TYPE_REQUIRE=<exact-url,...> fails unless every listed top-level
# message type has at least one successfully finalized Go transaction.
# DELIVER_HL=1 drives incoming/outgoing Hyperlane transfers. Optional
# DELIVER_HL_ISM=noop|message-id|merkle-root selects the mailbox verifier
# (default noop). Multisig modes use harness-held 2-of-3 validators and
# fail as incomplete evidence unless both incoming collateral and synthetic
# transfers succeed. HL_VERIFIED_VERSION and DELIVER_STATS attribute successes
# to the app version. Signed upgrade runs require both paths at v9 before the
# held signal is released.
# DELIVER_HL_ALLOW_POSTUPGRADE=governance-fixture is the narrow exception for
# a fixed DELIVER_SCRIPT + DELIVER_GOV_FAILURE_REQUIRE state regression.
# DELIVER_HL_REJECT=1 requires a signed ISM; it first rejects malformed
# metadata then retries the same message in adjacent transactions. Every
# required pair must reject at verification and then succeed, or exit nonzero.
# Cannot be combined with DELIVER_UPGRADE (its early fixture windows).
#
# The v9 -> v10 upgrade (GO_APPD = v9.0.8, GO_APPD_NEXT = v10.5.0): when Go
# raises the app version, the harness stops the Go app after that Commit
# and starts GO_APPD_NEXT on the same home and address (the multiplexer's
# switch, as abci-recorder reproduces it); the Rust app continues.
#   GO_APPD=.../v9/celestia-appd GO_APPD_NEXT=.../celestia-appd \
#     DELIVER_UPGRADE=20 crates/fuzz-diff/deliver-diff.sh v9-upgrade 120
# DELIVER_UPGRADE=<n> stops the replay before the scenario's signal txs and
# sends them from generated block n (see crates/fuzz-diff/src/deliver/upgrade.rs);
# without it the recorded upgrade of v9-upgrade is replayed (switch included).
set -euo pipefail
SCENARIO=${1:?scenario}
BLOCKS=${2:-200}
PER=${3:-8}
SEED=${4:-1}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APPD=${GO_APPD:?path to celestia-appd}
PORT=$((37858 + ${DEVNET_PORT_OFFSET:-0}))
IN=$ROOT/conformance/fixtures/devnet/$SCENARIO
WORK=$(mktemp -d)
H=$WORK/home
CHAIN_ID=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['chain_id'])" "$IN/meta.json")
"$APPD" init node --chain-id "$CHAIN_ID" --home "$H" >"$WORK/init.log" 2>&1 || {
  echo "$APPD init failed:" >&2
  tail -n 20 "$WORK/init.log" >&2
  exit 4
}
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
# Scenarios recorded without a genesis file (the ibctesting fixtures): one
# from the recorded InitChain.
if [ -f "$IN/genesis.json" ]; then
  cp "$IN/genesis.json" "$H/config/genesis.json"
else
  "$FUZZ_DIFF" genesis "$SCENARIO" >"$H/config/genesis.json"
fi
# The gRPC server on: the SDK then registers the tx service on the gRPC
# query router, so ABCI Query serves `Service/Simulate` (`DELIVER_SIM`) as
# a node with comet and gRPC enabled does.
GRPC=$((39690 + ${DEVNET_PORT_OFFSET:-0}))
python3 - "$H/config/app.toml" "$GRPC" <<'PY'
import re, sys
p, port = sys.argv[1], sys.argv[2]
s = open(p).read()
i = s.index("[grpc]")
j = s.index("[", i + 1)
sec = re.sub(r'(?m)^enable = .*$', 'enable = true', s[i:j])
sec = re.sub(r'(?m)^address = .*$', 'address = "127.0.0.1:%s"' % port, sec)
open(p, "w").write(s[:i] + sec + s[j:])
PY
GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$APPD" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >"$WORK/app.log" 2>&1 &
APP=$!
echo $APP >"$WORK/app.pid"
if [ -n "${GO_APPD_NEXT:-}" ]; then
  # Run by the harness after the Commit that raised the app version.
  cat >"$WORK/switch.sh" <<SH
set -e
OLD=\$(cat "$WORK/app.pid")
kill \$OLD; while kill -0 \$OLD 2>/dev/null; do sleep 0.2; done
GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$GO_APPD_NEXT" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >>"$WORK/app.log" 2>&1 &
echo \$! >"$WORK/app.pid"
until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null; do sleep 0.2; done
SH
  export DELIVER_SWITCH_CMD="bash $WORK/switch.sh"
fi
# GO_LOG=<file> keeps the Go app's log.
# The Go app must be gone before the next run binds the same ports: its
# shutdown can stall, so SIGKILL it after 10 s.
app_running() {
  kill -0 "$1" 2>/dev/null || return 1
  # After an upgrade, the new Go app is a child of switch.sh. A minimal
  # container init may leave it as a zombie after exit; kill -0 still
  # succeeds then, even though it has released its ports and files.
  case "$(ps -o stat= -p "$1" 2>/dev/null)" in
    "" | *Z*) return 1 ;;
  esac
}
stop_app() {
  local pid
  pid=$(cat "$WORK/app.pid" 2>/dev/null) || return 0
  kill "$pid" 2>/dev/null || return 0
  for _ in $(seq 1 100); do
    app_running "$pid" || return 0
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null || true
  for _ in $(seq 1 100); do
    app_running "$pid" || return 0
    sleep 0.1
  done
}
trap 'stop_app; if [ -n "${GO_LOG:-}" ]; then cp "$WORK/app.log" "$GO_LOG"; fi; rm -rf "$WORK"' EXIT
UP=
for _ in $(seq 1 300); do
  (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null && UP=1 && break
  kill -0 $APP 2>/dev/null || break
  sleep 1
done
if [ -z "$UP" ]; then
  echo "Go celestia-appd exited or did not listen on $PORT within 300 s" >&2
  tail -n 20 "$WORK/app.log" >&2
  exit 3
fi
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
"$FUZZ_DIFF" deliver "$SCENARIO" "127.0.0.1:$PORT" "$BLOCKS" "$PER" "$SEED"
