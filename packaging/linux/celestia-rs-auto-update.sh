#!/usr/bin/env bash
# Keeps a Linux celestia-rs node on the release its network's channel
# recommends (docs/releasing.md, "Release channels"). Run daily by
# celestia-rs-auto-update.timer, as root.
#
#   CHANNEL=mocha celestia-rs-auto-update.sh
#   CHANNEL=mocha DRY_RUN=1 celestia-rs-auto-update.sh   # change nothing
#
# One pass:
#   1. Read the channel's tag. Stop if the node already runs it, or a
#      deploy of it was rolled back here before.
#   2. Install it with that release's own scripts/install.sh into $STATE
#      (it verifies the archive's SHA-256), and check that the binary
#      reports the release.
#   3. Keep the running binary as $BIN.prev, swap the new one in
#      atomically and restart $SERVICE.
#   4. Wait until the node is caught up and has advanced MIN_BLOCKS
#      heights. Otherwise restore $BIN.prev, restart, record the tag in
#      $STATE/bad-tags and exit 1.
#
# Exit 0 when nothing needed doing or the update succeeded, 1 when it was
# rolled back, 2 when the pass could not run.
set -euo pipefail

CHANNEL=${CHANNEL:?set CHANNEL to the channel of the node network: quarto, mocha or mainnet}
REPO=${REPO:-celestiaorg/celestia-rs}
CHANNELS_URL=${CHANNELS_URL:-https://raw.githubusercontent.com/$REPO/release-channels/channels}
BIN=${BIN:-/usr/local/bin/celestia-rs}
SERVICE=${SERVICE:-celestia-rs}
RPC=${RPC:-http://127.0.0.1:26657}
STATE=${STATE:-/var/lib/celestia-rs-auto-update}
MIN_BLOCKS=${MIN_BLOCKS:-20}
HEALTH_TIMEOUT=${HEALTH_TIMEOUT:-900}
HEALTH_POLL=${HEALTH_POLL:-5}
DRY_RUN=${DRY_RUN:-0}

log() { echo "$(date -u +%FT%TZ) $*"; }
die() { log "ERROR: $*"; exit 2; }

mkdir -p "$STATE"
touch "$STATE/bad-tags"

# --- 1. what is there to deploy ---------------------------------------

[[ $CHANNEL =~ ^[a-z0-9-]+$ ]] || die "invalid channel: $CHANNEL"
TAG=$(curl -fsS --connect-timeout 10 --max-time 30 "$CHANNELS_URL/$CHANNEL" | head -n 1 | tr -d '\r') ||
  die "cannot read channel $CHANNEL from $CHANNELS_URL/$CHANNEL"
[[ -n $TAG ]] || die "cannot read channel $CHANNEL from $CHANNELS_URL/$CHANNEL"
[[ $TAG =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] || die "channel $CHANNEL names an invalid release tag: $TAG"

running=$("$BIN" version 2>/dev/null || true)
if [[ "v$running" == "$TAG" ]]; then
  log "up to date: $BIN runs $TAG (channel $CHANNEL)"
  exit 0
fi
if grep -qx "$TAG" "$STATE/bad-tags"; then
  log "skip $TAG: a deploy of it was rolled back before (remove it from $STATE/bad-tags to retry)"
  exit 0
fi
if [[ $DRY_RUN == 1 ]]; then
  log "DRY_RUN: would update $BIN from ${running:-nothing} to $TAG"
  exit 0
fi

# --- 2. install into the staging home -----------------------------------

log "installing $TAG (channel $CHANNEL)"
stage=$STATE/stage
rm -rf "$stage"
installer=$(curl -fsSL --connect-timeout 10 --max-time 60 \
  "https://raw.githubusercontent.com/$REPO/$TAG/scripts/install.sh") || die "cannot download the $TAG installer"
CELESTIA_RS_HOME=$stage CELESTIA_RS_REPO=$REPO sh -c "$installer" install.sh \
  --version "$TAG" --no-modify-path || die "install of $TAG failed"
NEW=$stage/bin/celestia-rs
[[ "v$("$NEW" version 2>/dev/null)" == "$TAG" ]] || die "installed binary does not report $TAG"

# --- 3. swap and restart ------------------------------------------------

height() { curl -fsS -m 10 "$RPC/status" | jq -r '.result.sync_info.latest_block_height'; }
swap_in() { cp "$1" "$2.new" && mv -f "$2.new" "$2"; }

healthy() {
  local start=$1 deadline=$((SECONDS + HEALTH_TIMEOUT)) st
  while ((SECONDS < deadline)); do
    if st=$(curl -fsS -m 10 "$RPC/status" 2>/dev/null) &&
      [[ $(jq -r '.result.sync_info.catching_up' <<<"$st") == false ]] &&
      (($(jq -r '.result.sync_info.latest_block_height' <<<"$st") >= start + MIN_BLOCKS)); then
      return 0
    fi
    sleep "$HEALTH_POLL"
  done
  log "not caught up and advancing $MIN_BLOCKS heights after ${HEALTH_TIMEOUT}s"
  return 1
}

start=$(height 2>/dev/null) || start=0
[[ $start =~ ^[0-9]+$ ]] || start=0
[[ -e $BIN ]] && cp -p "$BIN" "$BIN.prev"
swap_in "$NEW" "$BIN"
log "$SERVICE: ${running:-nothing} -> ${TAG#v} at height $start"
systemctl restart "$SERVICE"

# --- 4. health check, or roll back --------------------------------------

if healthy "$start"; then
  log "done: $SERVICE healthy on $TAG"
  exit 0
fi
log "$SERVICE unhealthy on $TAG; rolling back"
echo "$TAG" >>"$STATE/bad-tags"
if [[ -e $BIN.prev ]]; then
  swap_in "$BIN.prev" "$BIN"
  systemctl restart "$SERVICE"
  log "rolled back to $("$BIN" version 2>/dev/null || echo unknown)"
fi
exit 1
