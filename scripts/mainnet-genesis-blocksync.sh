#!/usr/bin/env bash
# Block sync Celestia mainnet from genesis over P2P (no state sync).
#
# Usage:
#   ARCHIVE_PEERS=<id>@<host>:<port>[,...] scripts/mainnet-genesis-blocksync.sh [stop-height]
#
# Environment:
#   ARCHIVE_PEERS    required: persistent peers whose block store starts at
#                    height 1 (archive nodes); pruned peers cannot serve
#                    the early heights and are skipped by block sync.
#   CELESTIA_RS_BIN  binary (default: celestia-rs on PATH)
#   CELESTIA_RS_HOME node home (default: ~/.celestia-rs/mainnet-genesis)
#   KEEP_ALL_BLOCKS  1 = min-retain-blocks 0 (serve history to others; disk grows)
#   REFERENCE_RPC    RPC named in the final app hash hint
#                    (default: https://rpc.celestia.pops.one)
#
# Block sync checks every header against the state the node computed, so a
# node that reaches height H matched every app hash through H-1. The first
# divergence stops the node with an error ("wrong Block.Header.AppHash").
set -euo pipefail

BIN=${CELESTIA_RS_BIN:-celestia-rs}
HOME_DIR=${CELESTIA_RS_HOME:-$HOME/.celestia-rs/mainnet-genesis}
STOP=${1:-0}
REF=${REFERENCE_RPC:-https://rpc.celestia.pops.one}
[[ -n ${ARCHIVE_PEERS:-} ]] || { echo "set ARCHIVE_PEERS to archive peers (id@host:port,...)" >&2; exit 2; }
[[ $STOP =~ ^[0-9]+$ ]] || { echo "stop height must be a number" >&2; exit 2; }

ulimit -n 65536 2>/dev/null || echo "warning: could not raise the open-files limit" >&2

if [[ ! -f $HOME_DIR/config/genesis.json ]]; then
  # Downloads and verifies the mainnet genesis (SHA-256 pinned in networks.rs).
  "$BIN" init genesis-sync --network mainnet --home "$HOME_DIR"
  # No state-sync snapshots while catching up (they cost I/O and disk).
  sed -i.bak 's/^snapshot-interval = .*/snapshot-interval = 0/' "$HOME_DIR/config/app.toml"
  if [[ ${KEEP_ALL_BLOCKS:-0} == 1 ]]; then
    sed -i.bak 's/^min-retain-blocks = .*/min-retain-blocks = 0/' "$HOME_DIR/config/app.toml"
  fi
fi

mkdir -p "$HOME_DIR/logs"
log=$HOME_DIR/logs/blocksync-$(date -u +%Y%m%dT%H%M%SZ).log
echo "logging to $log (progress: grep 'Block Sync Rate')"
# No --statesync-rpc-servers: a fresh home block-syncs from genesis.
# Restarting this script resumes from the last committed height.
command=("$BIN" start --home "$HOME_DIR" --p2p.persistent_peers "$ARCHIVE_PEERS" --stop-at-height "$STOP")
# A bounded run needs the plain INFO stop record even with a quiet
# RUST_LOG or a JSON log_format in the existing home.
if [[ $STOP =~ [1-9] ]]; then
  command=(env RUST_LOG=info "${command[@]}" --log_format plain)
fi
set +e
"${command[@]}" >"$log" 2>&1
status=$?
set -e
grep 'Block Sync Rate' "$log" | tail -n 1 || true
if [[ $status -ne 0 ]]; then
  echo "node exited with status $status; last lines:" >&2
  tail -n 20 "$log" >&2
  exit "$status"
fi

grep 'synced block' "$log" | tail -n 1 || true
if [[ $STOP =~ [1-9] ]]; then
  # The block-sync reactor can stop for either a signal or the requested
  # height. The node also emits "stop height reached" in consensus and when
  # an existing home is already at or beyond the requested height.
  height=$(sed -nE \
    -e 's/^([^ ]+[[:space:]]+)?INFO celestia_rs::node: block sync finished height=([0-9]+) blocks_synced=[0-9]+ reason=Stopped$/\2/p' \
    -e 's/^([^ ]+[[:space:]]+)?INFO celestia_rs::node: stop height reached height=([0-9]+)$/\2/p' "$log" | tail -n 1)
  if [[ -z $height ]] || ((10#$height < 10#$STOP)); then
    echo "node stopped before confirming the requested height $STOP" >&2
    exit 1
  fi
  echo "reached height $height; its header commits the app hash for height $((10#$height - 1))"
else
  echo "unbounded node stopped; see $log for the last executed height"
fi
echo "To compare the last app hash with the network, run the node again"
echo "and use: packaging/check-apphash.sh http://127.0.0.1:26657 $REF"
