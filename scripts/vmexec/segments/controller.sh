#!/bin/bash
# Segment controller: runs on a small VM (cloud-platform scope) as a vmexec
# job, so replay segments do not depend on any workstation. Every 5 minutes:
# - launches a segment VM for each Go export go-snapshots/<H>/{2,3}/metadata.json
#   (at most MAX_RUNNING at once), replaying H to the next planned height;
# - deletes a segment's VM once segments/<H>/done or /failed is written;
# - relaunches a segment whose VM vanished (local SSD lost on stop or
#   preemption) from its newest Rust snapshot rs-snapshots/from-go-<H>/<h>,
#   at most MAX_RESTARTS times.
# State lives in the bucket under segments/<H>/ (launched, zone, restarts,
# done, failed). Usage: controller.sh  (from the directory holding vmexec/)
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
V=$here/../vmexec
BUCKET=gs://fibreda-celestia-rs-genesis
MAX_RUNNING=${MAX_RUNNING:-3} MAX_RESTARTS=${MAX_RESTARTS:-3} MAX_LAUNCHES=${MAX_LAUNCHES:-40}
ZONES=${ZONES:-"us-central1-a us-central1-b us-central1-c us-central1-f"}
# Planned chain: each segment replays to the next height. v0.46 checkpoints
# (format 2) to the v4 upgrade, then v0.50 ones (format 3): its boundaries,
# the two double-sign evidence heights and every 500,000.
PLAN="120000 250000 500000 750000 1000000 1250000 1500000 1750000 2000000 2250000 2371494 2500000 2750000 2993219 3000000 3250000 3500000 3750000 4000000 4250000 4500000 4750000 5000000 5250000 5500000 5750000 6000000 6250000 6500000 6680339"
PLAN+=" 6748821 7000000 7500000 8000000 8218900 8500000 8662012 9000000 9217850 9500000 10000000 10500000 10960599 11000000 11500000 11771698 12000000 12500000 13000000 13500000 14000000 14500000 15000000 15105000"
# Heights launched first when slots free (the rest follow in plan order): the
# first v4 block and the evidence blocks exercise the v0.50 code early.
PRIORITY=${PRIORITY:-"6680339 8218900 9217850"}
log() { echo "$(date -u +%FT%TZ) $*"; }
gs_get() { gcloud storage cat "$BUCKET/$1" 2>/dev/null; }
gs_put() { printf '%s\n' "$2" | gcloud storage cp - "$BUCKET/$1" >/dev/null 2>&1; }
gs_has() { gcloud storage ls "$BUCKET/$1" >/dev/null 2>&1; }
next_of() { local prev='' h; for h in $PLAN; do [ "$prev" = "$1" ] && { echo "$h"; return; }; prev=$h; done; }
segment_vms() { gcloud compute instances list --project=fibreda --filter="labels.purpose=rs-segment-replay AND labels.owner=claude-genesis" --format='value(name,zone.basename(),status)' 2>/dev/null; }

launch() { # launch SEG SRC START STOP NAME
  local seg=$1 src=$2 start=$3 stop=$4 name=$5 z ok=0
  local n; n=$(gs_get segments/launch-count || echo 0); n=${n:-0}
  ((n < MAX_LAUNCHES)) || { log "launch cap $MAX_LAUNCHES reached; not launching $name"; return 1; }
  gs_put segments/launch-count $((n + 1))
  for z in $ZONES; do
    if VMX_ZONE=$z $V create "$name" --purpose rs-segment-replay --machine-type c3-standard-8-lssd --boot-gb 50 --hours 48 --gcs; then ok=1; break; fi
    log "$name: c3-standard-8-lssd not available in $z"
  done
  if ((ok == 0)); then
    z=us-central1-a
    VMX_ZONE=$z $V create "$name" --purpose rs-segment-replay --machine-type c3-standard-8 --boot-gb 50 --data-gb 200 --hours 48 --gcs || { log "$name: create failed"; return 1; }
  fi
  # Provision; on any failure delete the VM so it cannot hold a running slot
  # with no replay. The launched marker is written only once both jobs run
  # (the attempt already counted toward MAX_LAUNCHES or MAX_RESTARTS).
  export VMX_ZONE=$z
  # main, unless segments/SEG/ref pins the segment to a commit (a PR under test).
  local ref pin=''
  ref=$(gs_get segments/$seg/ref | head -1 | tr -cd '0-9a-zA-Z/._-')
  [ -n "$ref" ] && pin=" && git -C /opt/celestia-rs fetch -q origin $ref && git -C /opt/celestia-rs checkout -q --force $ref"
  if ! { $V exec "$name" -- "git clone -q --depth 1 https://github.com/celestiaorg/celestia-rs /opt/celestia-rs$pin && git -C /opt/celestia-rs log -1 --format='%H %s'" &&
    $V build "$name" | tail -n 3 && [ "${PIPESTATUS[0]}" = 0 ] &&
    $V push "$name" "$here/seg-vm.sh" /data/seg-vm.sh >/dev/null && $V push "$name" "$here/seg-watch.sh" /data/seg-watch.sh >/dev/null &&
    $V job "$name" start seg --interval 300 --status-cmd 'L=$(ls -t /data/home-*/logs/*.log 2>/dev/null | head -1); [ -n "$L" ] && grep -E "restored|followed height|ERROR|diverg" $L | tail -n 2 | cut -c1-250' -- env SEG_EXTRA_RPC="${SEG_EXTRA_RPC:-}" bash /data/seg-vm.sh "$seg" "$src" "$start" "$stop" &&
    $V job "$name" start segwatch --interval 600 --status-cmd "tail -n 2 /data/heights-$seg.tsv" -- bash /data/seg-watch.sh "$seg" "$stop"; }; then
    log "$name: provisioning failed; deleting the VM"
    $V delete "$name" >/dev/null 2>&1 || true
    unset VMX_ZONE
    return 1
  fi
  unset VMX_ZONE
  gs_put segments/$seg/zone "$z"
  gs_put segments/$seg/launched "$(date -u +%FT%TZ) $name $src $start->$stop${ref:+ ref $ref}"
  log "launched $name: segment $seg from $src at $start to $stop in $z"
}

tick() {
  local vms running h stop name z r last src
  vms=$(segment_vms)
  running=$(awk '$3 ~ /RUNNING|PROVISIONING|STAGING/' <<<"$vms" | wc -l)
  for h in $PRIORITY $(for p in $PLAN; do [[ " $PRIORITY " == *" $p "* ]] || echo "$p"; done); do
    stop=$(next_of "$h"); [ -n "$stop" ] || continue
    name=$(awk -v p="rs-seg-$h" '$1 == p || index($1, p "-r") == 1 {print $1; exit}' <<<"$vms")
    if gs_has segments/$h/done || gs_has segments/$h/failed; then
      if [ -n "$name" ]; then
        z=$(gs_get segments/$h/zone); log "segment $h finished; deleting $name"
        VMX_ZONE=${z:-us-central1-a} $V delete "$name" >/dev/null 2>&1 || true
      fi
      continue
    fi
    [ -z "$name" ] || continue # running
    if gs_has segments/$h/launched; then
      # Launched before but its VM is gone: restart from the newest Rust snapshot.
      r=$(gs_get segments/$h/restarts || echo 0); r=${r:-0}
      ((r < MAX_RESTARTS)) || { gs_put segments/$h/failed "failed $(date -u +%FT%TZ) restart limit $MAX_RESTARTS"; continue; }
      ((running < MAX_RUNNING)) || continue
      last=$(gcloud storage ls "$BUCKET/rs-snapshots/from-go-$h/*/*/metadata.json" 2>/dev/null |
        sed -E "s|.*/from-go-$h/([0-9]+)/([0-9]+)/metadata.json|\1|" | sort -n | awk -v s="$stop" '$1 < s' | tail -1)
      if [ -n "$last" ]; then src=rs-snapshots/from-go-$h/$last; else src=go-snapshots/$h; last=$h; fi
      gs_put segments/$h/restarts $((r + 1))
      log "segment $h: VM gone before done; restart $((r + 1)) from $src"
      launch "$h" "$src" "$last" "$stop" "rs-seg-$h-r$((r + 1))" && running=$((running + 1))
      continue
    fi
    gs_has go-snapshots/$h/2/metadata.json || gs_has go-snapshots/$h/3/metadata.json || continue
    ((running < MAX_RUNNING)) || continue
    launch "$h" "go-snapshots/$h" "$h" "$stop" "rs-seg-$h" && running=$((running + 1))
  done
}

# Tests source this file for its functions only.
[ "${CONTROLLER_SOURCE_ONLY:-}" = 1 ] && return 0
command -v jq >/dev/null || { apt-get -o DPkg::Lock::Timeout=600 update -q >/dev/null; apt-get -o DPkg::Lock::Timeout=600 install -yq jq >/dev/null; }
while true; do
  tick
  [ -e /var/lib/vmexec/controller-stop ] && { log "stop file"; exit 0; }
  sleep 300
done
