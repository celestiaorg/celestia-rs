#!/bin/bash
# Segment watcher (on the segment VM): every 5 min append
# "time height state reason" to rs-replay/SEG/heights.tsv, upload finished
# Rust snapshots to rs-snapshots/from-go-SEG/<h>/<f>/ (metadata.json last),
# and when the replay ends upload the log tail and diagnostics and write
# segments/SEG/done (reached STOP) or segments/SEG/failed (with the reason).
# A replay that is active but has not advanced for STALL_SECS (default 600)
# writes segments/SEG/stalled and uploads the log tail once per stall.
#   seg-watch.sh SEG STOP
set -u
SEG=$1 STOP=$2 DATA=${SEG_DATA:-/data} HOST=$(hostname)
HOME_DIR=$DATA/home-$SEG T=$DATA/heights-$SEG.tsv
WATCH_SECS=${WATCH_SECS:-300} STALL_SECS=${STALL_SECS:-600} last_h='' last_change=$(date +%s) stalled=0
# A detected stall (or resume) stays pending until its marker object is
# published: a failed put is retried on every poll. heights.tsv gets the
# "stalled" reason once, on the poll that detects the stall.
stall_pub=0 tail_pub=0 resume_pend=0
while true; do
  L=$(ls -t $HOME_DIR/logs/*.log 2>/dev/null | head -1)
  h=''; [ -n "$L" ] && h=$(grep -oE 'followed height=[0-9]+' "$L" | tail -1 | cut -d= -f2)
  a=$(systemctl is-active vmxjob-seg)
  reason=''
  [ "$a" = active ] || { [ -n "$L" ] && reason=$(grep -E '^Error|ERROR' "$L" | tail -1 | cut -c1-240); }
  now=$(date +%s)
  if [ "${h:-}" != "$last_h" ]; then
    last_h=${h:-} last_change=$now
    # Report the resume only if the stall itself was published.
    [ $stalled = 1 ] && { stalled=0; resume_pend=$stall_pub; }
  elif [ -n "${h:-}" ] && [ "$a" = active ] && [ $stalled = 0 ] && ((now - last_change >= STALL_SECS)); then
    stalled=1 stall_pub=0 tail_pub=0 resume_pend=0
    reason="stalled: no progress past ${h:--} for $(( (now - last_change) / 60 )) min"
    tail_pub=1; [ -n "$L" ] && { tail -n 300 "$L" > $DATA/stall-tail.log; tail_pub=0; }
    echo "stalled $(date -u +%FT%TZ) $HOST height=${h:--}" > $DATA/seg-stalled
  fi
  if [ $stalled = 1 ]; then
    [ $tail_pub = 1 ] || { vmx-gcs put $DATA/stall-tail.log rs-replay/$SEG/stall-tail-$HOST.log >/dev/null 2>&1 && tail_pub=1; }
    [ $stall_pub = 1 ] || { vmx-gcs put $DATA/seg-stalled segments/$SEG/stalled >/dev/null 2>&1 && stall_pub=1; }
  fi
  if [ $resume_pend = 1 ]; then
    vmx-gcs put /dev/null segments/$SEG/resumed-$HOST >/dev/null 2>&1 && { resume_pend=0 stall_pub=0; }
  fi
  printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "${h:--}" "$a" "$HOST" "$reason" >> $T
  vmx-gcs put $T rs-replay/$SEG/heights-$HOST.tsv >/dev/null 2>&1
  for d in $HOME_DIR/data/snapshots/*/*/; do
    [ -d "$d" ] || continue
    sh=$(basename "$(dirname "$d")"); f=$(basename "$d")
    [[ $sh =~ ^[0-9]+$ && $f =~ ^[0-9]+$ ]] || continue
    [ -e $DATA/uploaded-$sh-$f ] && continue
    # While the node runs, a snapshot being written has recent files.
    [ "$a" != active ] || [ -z "$(find "$d" -mmin -10 -print -quit)" ] || continue
    ok=1
    nchunks=0
    for c in "$d"*; do
      vmx-gcs put "$c" "rs-snapshots/from-go-$SEG/$sh/$f/${c##*/}" || { ok=0; break; }
      [[ ${c##*/} =~ ^[0-9]+$ ]] && nchunks=$((nchunks + 1))
    done
    if [ $ok = 1 ]; then
      jq -n --argjson height $sh --argjson format $f --argjson chunk_count "$nchunks" \
        --arg source "celestia-rs replay segment $SEG on $HOST, commit $(git -C /opt/celestia-rs rev-parse HEAD)" \
        --arg written_at "$(date -u +%FT%TZ)" '$ARGS.named' > $DATA/meta-rs-$sh.json
      vmx-gcs put $DATA/meta-rs-$sh.json rs-snapshots/from-go-$SEG/$sh/$f/metadata.json && touch $DATA/uploaded-$sh-$f
    fi
  done
  if [ "$a" != active ] && [ "$a" != activating ]; then
    [ -n "$L" ] && { tail -n 300 "$L" > $DATA/stop-tail.log; vmx-gcs put $DATA/stop-tail.log rs-replay/$SEG/stop-tail-$HOST.log; }
    if ls -d $HOME_DIR/follow-diagnostics/* >/dev/null 2>&1; then
      tar -czf $DATA/diag.tgz -C $HOME_DIR follow-diagnostics && vmx-gcs put $DATA/diag.tgz rs-replay/$SEG/diagnostics-$HOST.tgz
    fi
    if [ -n "$L" ] && grep -qE "follow finished .*last=$STOP( |$)" "$L"; then
      echo "done $(date -u +%FT%TZ) $HOST $(git -C /opt/celestia-rs rev-parse HEAD)" > $DATA/seg-state
      vmx-gcs put $DATA/seg-state segments/$SEG/done
    else
      echo "failed $(date -u +%FT%TZ) $HOST height=${h:--} ${reason:-no log}" > $DATA/seg-state
      vmx-gcs put $DATA/seg-state segments/$SEG/failed
    fi
    exit 0
  fi
  sleep "$WATCH_SECS"
done
