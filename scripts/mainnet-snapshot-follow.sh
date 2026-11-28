#!/usr/bin/env bash
# Restore a Go state-sync snapshot of Celestia mainnet and execute the
# blocks after it, checking each against the network's signed headers.
#
# Usage:
#   RPC=https://archive-rpc[,witness...] scripts/mainnet-snapshot-follow.sh <snapshot-dir> <height> [stop-height]
#
# <snapshot-dir> holds Go's snapshot layout, <height>/<format>/<chunk>
# (a celestia-appd data/snapshots directory, or the output of an archive
# exporter): format 2 for heights of app versions 1 to 3 (cosmos-sdk
# v0.46, heights below 6,680,340), format 3 from app version 4. The chunk
# files are all that is read; Go's metadata.db is not needed.
#
# Environment:
#   RPC              required: comma-separated RPC servers that still hold
#                    <height> (archive nodes for old heights); the first is
#                    the primary, the others witness the light client.
#   TRUST_HASH       hash of header <height> from a source you trust (hex);
#                    without it the primary's header is trusted.
#   CELESTIA_RS_BIN  binary (default: celestia-rs on PATH)
#   CELESTIA_RS_HOME node home (default: ~/.celestia-rs/mainnet-snapshot-<height>)
#   SNAPSHOT_INTERVAL write a snapshot to <home>/data/snapshots every N
#                    heights (resumable checkpoints; default 0 = none):
#                    format 2 up to app version 3, format 3 after.
#   SNAPSHOT_KEEP    snapshots kept (default 0 = all)
#   DB_CACHE_MIB     state database cache in MiB (default 4096, the
#                    catch-up setting; follow's own default is 512).
#                    follow has no flag yet for the IAVL node cache or the
#                    pruning interval, so those keep their defaults.
#
# The restored app hash must equal header <height>+1's, then every block
# from <height>+1 executes and must match the next header's app hash and
# results hash. The first divergence stops the follower and writes a
# diagnostic bundle to <home>/follow-diagnostics/<height>/.
set -euo pipefail

SNAP=${1:?usage: $0 <snapshot-dir> <height> [stop-height]}
HEIGHT=${2:?usage: $0 <snapshot-dir> <height> [stop-height]}
STOP=${3:-0}
BIN=${CELESTIA_RS_BIN:-celestia-rs}
HOME_DIR=${CELESTIA_RS_HOME:-$HOME/.celestia-rs/mainnet-snapshot-$HEIGHT}
[[ -n ${RPC:-} ]] || { echo "set RPC to archive RPC servers holding height $HEIGHT" >&2; exit 2; }
[[ $HEIGHT =~ ^[0-9]+$ && $STOP =~ ^[0-9]+$ ]] || { echo "heights must be numbers" >&2; exit 2; }
# Without the snapshot the run still verifies the light-client data at
# <height> over RPC and then stops at the missing snapshot: a check of the
# RPC servers and trust hash before the snapshot arrives.
ls -d "$SNAP/$HEIGHT"/[23] >/dev/null 2>&1 ||
  echo "warning: no $SNAP/$HEIGHT/2 or $SNAP/$HEIGHT/3: checking the RPC side only" >&2

ulimit -n 65536 2>/dev/null || echo "warning: could not raise the open-files limit" >&2
mkdir -p "$HOME_DIR/logs"
log=$HOME_DIR/logs/follow-$(date -u +%Y%m%dT%H%M%SZ).log
echo "logging to $log"
# The light client measures its trusting period from now; the header at
# an old snapshot height is years old, so the period covers it. The trust
# comes from --trust-hash (or the primary) at the snapshot height, not
# from the period.
args=(follow --home "$HOME_DIR" --chain-id celestia --rpc "$RPC"
  --snapshot "$SNAP" --snapshot-height "$HEIGHT"
  --trust-period-hours 200000 --stop-at-height "$STOP"
  --snapshot-interval "${SNAPSHOT_INTERVAL:-0}" --snapshot-keep-recent "${SNAPSHOT_KEEP:-0}"
  --db-cache-mib "${DB_CACHE_MIB:-4096}")
[[ -n ${TRUST_HASH:-} ]] && args+=(--trust-hash "$TRUST_HASH")
# A restarted run resumes from the home's state (the snapshot is used on
# the first start only).
command=("$BIN" "${args[@]}")
# Bounded runs use the plain INFO stop record as proof. RUST_LOG takes
# precedence over --log_level, and config.toml may select JSON output.
if [[ $STOP =~ [1-9] ]]; then
  command=(env RUST_LOG=info "${command[@]}" --log_format plain)
fi
set +e
"${command[@]}" >"$log" 2>&1
status=$?
set -e
grep -E 'light-client data at the snapshot height verified|snapshot restored' "$log" | tail -n 2 || true
if [[ $status -ne 0 ]]; then
  echo "follower exited with status $status; last lines:" >&2
  tail -n 20 "$log" >&2
  exit "$status"
fi
# The CLI accepted this decimal i64, including any leading zeroes.
STOP=$((10#$STOP))
if ((STOP == 0)); then
  echo "unbounded follower stopped; see $log for the last executed height"
elif grep -qE "(^|[[:space:]])INFO celestia_rs::follow: stop height reached height=$STOP verified_header_height=$((STOP + 1))( |$)" "$log"; then
  echo "verified the network header at $((STOP + 1)) for stop height $STOP"
else
  echo "follower stopped before it authenticated the output of block $STOP" >&2
  exit 1
fi
