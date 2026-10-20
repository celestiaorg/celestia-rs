#!/usr/bin/env bash
# Deploys the latest green commit on celestia-rs `main` to this Mac's
# nodes, one at a time, canary first (docs/operator-guide.md §8).
#
#   celestia-rs-auto-update.sh            # one pass; run hourly by launchd
#   DRY_RUN=1 celestia-rs-auto-update.sh  # build, but deploy nothing
#   celestia-rs-auto-update.sh check      # health-check every node, change nothing
#   celestia-rs-auto-update.sh watchdog   # restart a node stuck behind; run every minute
#
# One pass:
#   1. Fetch origin/main into $SRC. Stop if every node already runs it, if
#      the commit failed a deploy before, or if its CI is not green yet.
#   2. Build it (`cargo build --release --locked`) and check that the
#      binary reports that commit.
#   3. For each node in NODES order (Mocha first): keep the running binary
#      as <bin>.prev, swap the new one in atomically, restart the
#      LaunchDaemon, and wait until the node is caught up, has advanced
#      MIN_BLOCKS heights, matches the network's app hash and (for a
#      bonded validator) signs at least MIN_SIGNED of the last
#      SIGN_WINDOW commits. On any failure: restore <bin>.prev, restart,
#      record the commit in $STATE/bad-commits and stop, so later nodes
#      never get it.
#   4. Between nodes, keep checking the canary for SOAK_SECS.
#
# Restarts use `sudo -n launchctl kickstart -k`, allowed without a
# password by packaging/macos/celestia-rs.sudoers. Exit 0 when nothing
# needed doing or the deploy succeeded, 1 when a deploy was rolled back,
# 2 when the pass could not run.
set -euo pipefail

CELESTIA_RS_DIR=${CELESTIA_RS_DIR:-$HOME/.celestia-rs}
SRC=${SRC:-$CELESTIA_RS_DIR/src}
BIN=${BIN:-$CELESTIA_RS_DIR/bin}
STATE=${STATE:-$CELESTIA_RS_DIR/auto-update}
REPO=${REPO:-celestiaorg/celestia-rs}
SOAK_SECS=${SOAK_SECS:-900}
MIN_BLOCKS=${MIN_BLOCKS:-20}
SIGN_WINDOW=${SIGN_WINDOW:-20}
MIN_SIGNED=${MIN_SIGNED:-15}
HEALTH_TIMEOUT=${HEALTH_TIMEOUT:-900}
BUILD_JOBS=${BUILD_JOBS:-5}
DRY_RUN=${DRY_RUN:-0}
# watchdog: restart a node more than WATCHDOG_MAX_LAG blocks behind its
# reference RPC on WATCHDOG_STRIKES passes in a row, at most once per
# WATCHDOG_COOLDOWN seconds. A deploy lock older than WATCHDOG_LOCK_STALE
# seconds is from a killed pass and is ignored.
WATCHDOG_MAX_LAG=${WATCHDOG_MAX_LAG:-50}
WATCHDOG_STRIKES=${WATCHDOG_STRIKES:-3}
WATCHDOG_COOLDOWN=${WATCHDOG_COOLDOWN:-1800}
WATCHDOG_LOCK_STALE=${WATCHDOG_LOCK_STALE:-10800}

# name|launchd label|binary|local RPC|reference RPC
NODES=(
  "mocha|org.celestia.celestia-rs|$BIN/celestia-rs|http://127.0.0.1:26657|https://rpc.celestia-mocha.com"
  "mainnet|org.celestia.celestia-rs-mainnet|$BIN/celestia-rs-mainnet|http://127.0.0.1:26757|https://rpc.celestia.pops.one"
)

HERE=$(cd "$(dirname "$0")" && pwd)
# Next to this script when installed, packaging/ in the repo.
if [[ -z ${CHECK_APPHASH:-} ]]; then
  CHECK_APPHASH=$HERE/check-apphash.sh
  [[ -x $CHECK_APPHASH ]] || CHECK_APPHASH=$HERE/../check-apphash.sh
fi

log() { echo "$(date -u +%FT%TZ) $*"; }
die() { log "ERROR: $*"; exit 2; }

mkdir -p "$STATE"
touch "$STATE/bad-commits"
LOCK=$STATE/lock
# The watchdog only reads the lock, so a deploy never finds it held.
if [[ ${1:-} != watchdog ]]; then
  mkdir "$LOCK" 2>/dev/null || die "another pass holds $LOCK"
  trap 'rmdir "$LOCK"' EXIT
fi

binary_commit() { "$1" version --long 2>/dev/null | awk '/^commit:/ {print $2}'; }

rpc() { curl -fsS -m 10 "$1$2"; }
height() { rpc "$1" /status | jq -r '.result.sync_info.latest_block_height'; }

# Healthy: caught up, advancing, same app hash as the network and, if
# bonded, signing.
healthy() {
  local local_rpc=$1 ref=$2 start=$3 deadline=$((SECONDS + HEALTH_TIMEOUT)) st h addr power signed
  while ((SECONDS < deadline)); do
    if st=$(rpc "$local_rpc" /status 2>/dev/null) &&
      [[ $(jq -r '.result.sync_info.catching_up' <<<"$st") == false ]] &&
      (($(jq -r '.result.sync_info.latest_block_height' <<<"$st") >= start + MIN_BLOCKS)); then
      break
    fi
    sleep 5
  done
  ((SECONDS < deadline)) || { log "not caught up and advancing after ${HEALTH_TIMEOUT}s"; return 1; }
  "$CHECK_APPHASH" "$local_rpc" "$ref" || return 1

  addr=$(jq -r '.result.validator_info.address' <<<"$st")
  power=$(jq -r '.result.validator_info.voting_power' <<<"$st")
  ((power > 0)) || return 0
  h=$(jq -r '.result.sync_info.latest_block_height' <<<"$st")
  signed=0
  for ((i = h - SIGN_WINDOW + 1; i <= h; i++)); do
    rpc "$local_rpc" "/commit?height=$i" |
      jq -e --arg a "$addr" 'any(.result.signed_header.commit.signatures[]; .validator_address == $a)' \
        >/dev/null && signed=$((signed + 1))
  done
  log "signed $signed of the last $SIGN_WINDOW commits"
  ((signed >= MIN_SIGNED))
}

restart() { sudo -n /bin/launchctl kickstart -k "system/$1"; }

# Swap a binary atomically (a running process keeps its old inode).
swap_in() { cp "$1" "$2.new" && mv -f "$2.new" "$2"; }

# After a network outage a running node catches up through consensus
# gossip, barely faster than the chain, so a few hours offline leave it
# hours behind and a validator jailed. A restart begins with block sync,
# which catches up in about a minute.
watchdog() {
  local n name label local_rpc ref sf st h rh lag strikes now last
  if [[ -d $LOCK ]] && (($(date +%s) - $(stat -f %m "$LOCK") < WATCHDOG_LOCK_STALE)); then
    return 0 # a deploy is restarting the nodes itself
  fi
  mkdir -p "$STATE/watchdog"
  for n in "${NODES[@]}"; do
    IFS='|' read -r name label _ local_rpc ref <<<"$n"
    sf=$STATE/watchdog/$name
    # A node that is down, or block-syncing already, is not ours to restart.
    st=$(rpc "$local_rpc" /status 2>/dev/null) || { rm -f "$sf.strikes"; continue; }
    h=$(jq -r '.result.sync_info.latest_block_height' <<<"$st")
    if [[ ! $h =~ ^[0-9]+$ || $(jq -r '.result.sync_info.catching_up' <<<"$st") == true ]]; then
      rm -f "$sf.strikes"
      continue
    fi
    rh=$(height "$ref" 2>/dev/null) || continue
    [[ $rh =~ ^[0-9]+$ ]] || continue
    lag=$((rh - h))
    if ((lag <= WATCHDOG_MAX_LAG)); then
      rm -f "$sf.strikes"
      continue
    fi
    strikes=$(($(cat "$sf.strikes" 2>/dev/null || echo 0) + 1))
    echo "$strikes" >"$sf.strikes"
    log "$name: $lag blocks behind $ref at height $h (strike $strikes of $WATCHDOG_STRIKES)"
    ((strikes >= WATCHDOG_STRIKES)) || continue
    now=$(date +%s)
    last=$(cat "$sf.last-restart" 2>/dev/null || echo 0)
    if ((now - last < WATCHDOG_COOLDOWN)); then
      log "$name: restarted $((now - last))s ago; next restart after ${WATCHDOG_COOLDOWN}s"
      continue
    fi
    log "$name: restarting $label to block-sync"
    restart "$label" || log "$name: restart of $label failed"
    echo "$now" >"$sf.last-restart"
    rm -f "$sf.strikes"
  done
}

if [[ ${1:-} == watchdog ]]; then
  watchdog
  exit 0
fi

if [[ ${1:-} == check ]]; then
  rc=0
  for n in "${NODES[@]}"; do
    IFS='|' read -r name _ bin local_rpc ref <<<"$n"
    h=$(height "$local_rpc") || { log "$name: not answering on $local_rpc"; rc=1; continue; }
    if healthy "$local_rpc" "$ref" "$((h - MIN_BLOCKS))"; then
      log "$name: healthy on $(binary_commit "$bin" | cut -c1-7) at height $h"
    else
      log "$name: UNHEALTHY"; rc=1
    fi
  done
  exit "$rc"
fi

# --- 1. what is there to deploy ---------------------------------------

[[ -d $SRC/.git ]] || git clone --quiet "https://github.com/$REPO.git" "$SRC"
git -C "$SRC" fetch --quiet origin main
SHA=$(git -C "$SRC" rev-parse FETCH_HEAD)

stale=()
for n in "${NODES[@]}"; do
  IFS='|' read -r name _ bin _ _ <<<"$n"
  [[ $(binary_commit "$bin") == "$SHA" ]] || stale+=("$name")
done
if ((${#stale[@]} == 0)); then
  log "up to date: every node runs ${SHA:0:7}"
  exit 0
fi
if grep -qx "$SHA" "$STATE/bad-commits"; then
  log "skip ${SHA:0:7}: a deploy of it was rolled back before"
  exit 0
fi

# Every check run on the commit must have finished green.
runs=$(curl -fsS "https://api.github.com/repos/$REPO/commits/$SHA/check-runs?per_page=100") ||
  die "cannot query CI for ${SHA:0:7}"
total=$(jq '.check_runs | length' <<<"$runs")
pending=$(jq '[.check_runs[] | select(.status != "completed")] | length' <<<"$runs")
failed=$(jq -r '[.check_runs[] | select(.status == "completed"
  and (.conclusion | IN("success", "skipped", "neutral") | not)) | .name] | join(", ")' <<<"$runs")
if ((total == 0 || pending > 0)); then
  log "wait: CI for ${SHA:0:7} not finished ($pending of $total runs pending)"
  exit 0
fi
if [[ -n $failed ]]; then
  log "skip ${SHA:0:7}: CI failed ($failed)"
  exit 0
fi
log "deploy ${SHA:0:7} ($(git -C "$SRC" log -1 --format=%s "$SHA")) to: ${stale[*]}"

# --- 2. build -----------------------------------------------------------

git -C "$SRC" checkout --quiet --detach "$SHA"
log "building"
(cd "$SRC" && nice -n 10 cargo build --quiet --release --locked -j "$BUILD_JOBS" -p celestia-rs) ||
  die "build of ${SHA:0:7} failed"
NEW=$SRC/target/release/celestia-rs
[[ $(binary_commit "$NEW") == "$SHA" ]] || die "built binary does not report ${SHA:0:7}"

if [[ $DRY_RUN == 1 ]]; then
  log "DRY_RUN: built ${SHA:0:7}; would deploy to ${stale[*]}"
  exit 0
fi

# --- 3. deploy ----------------------------------------------------------

canary=""
for n in "${NODES[@]}"; do
  IFS='|' read -r name label bin local_rpc ref <<<"$n"
  [[ $(binary_commit "$bin") == "$SHA" ]] && continue

  # Soak the canary before touching the next node.
  if [[ -n $canary ]]; then
    IFS='|' read -r cname _ _ crpc cref <<<"$canary"
    log "soaking $cname for ${SOAK_SECS}s"
    end=$((SECONDS + SOAK_SECS))
    while ((SECONDS < end)); do
      sleep 60
      "$CHECK_APPHASH" "$crpc" "$cref" >/dev/null || {
        echo "$SHA" >>"$STATE/bad-commits"
        die "$cname app hash diverged during soak; ${SHA:0:7} marked bad, $name untouched"
      }
    done
  fi

  start=$(height "$local_rpc") || die "$name is not answering on $local_rpc; not deploying"
  log "$name: $(binary_commit "$bin" | cut -c1-7) -> ${SHA:0:7} at height $start"
  cp -p "$bin" "$bin.prev"
  swap_in "$NEW" "$bin"
  restart "$label"

  if healthy "$local_rpc" "$ref" "$start"; then
    log "$name: healthy on ${SHA:0:7}"
    canary=$n
    continue
  fi

  log "$name: unhealthy on ${SHA:0:7}; rolling back"
  echo "$SHA" >>"$STATE/bad-commits"
  swap_in "$bin.prev" "$bin"
  restart "$label"
  if healthy "$local_rpc" "$ref" "$(height "$local_rpc" 2>/dev/null || echo 0)"; then
    log "$name: rolled back to $(binary_commit "$bin" | cut -c1-7), healthy"
  else
    log "$name: STILL UNHEALTHY after rollback; needs a human"
  fi
  exit 1
done
log "done: every node runs ${SHA:0:7}"
