#!/bin/bash
# Segment watcher (on the segment VM): every 5 min append
# "time height state host reason rss_kib" to rs-replay/SEG/heights-HOST.tsv, upload finished
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
# A detected stall stays pending until its marker object is published, also
# after the height moves again: a failed put is retried on every poll. The
# resume marker follows only after the stall marker, and a new stall is not
# recorded until both are out, so the markers appear in order. heights.tsv
# gets the "stalled" reason once, on the poll that detects the stall.
stall_pend=0 tail_pend=0 resume_pend=0
while true; do
  L=$(ls -t $HOME_DIR/logs/*.log 2>/dev/null | head -1)
  h=''; [ -n "$L" ] && h=$(grep -oE 'followed height=[0-9]+' "$L" | tail -1 | cut -d= -f2)
  a=$(systemctl is-active vmxjob-seg)
  reason=''
  [ "$a" = active ] || { [ -n "$L" ] && reason=$(grep -E '^Error|ERROR' "$L" | tail -1 | cut -c1-240); }
  now=$(date +%s)
  if [ "${h:-}" != "$last_h" ]; then
    last_h=${h:-} last_change=$now
    [ $stalled = 1 ] && { stalled=0; resume_pend=1; }
  elif [ -n "${h:-}" ] && [ "$a" = active ] && [ $stalled = 0 ] && [ $stall_pend = 0 ] && [ $resume_pend = 0 ] &&
    ((now - last_change >= STALL_SECS)); then
    stalled=1 stall_pend=1
    reason="stalled: no progress past ${h:--} for $(( (now - last_change) / 60 )) min"
    [ -n "$L" ] && { tail -n 300 "$L" > $DATA/stall-tail.log; tail_pend=1; }
    echo "stalled $(date -u +%FT%TZ) $HOST height=${h:--}" > $DATA/seg-stalled
  fi
  [ $tail_pend = 0 ] || { vmx-gcs put $DATA/stall-tail.log rs-replay/$SEG/stall-tail-$HOST.log >/dev/null 2>&1 && tail_pend=0; }
  [ $stall_pend = 0 ] || { vmx-gcs put $DATA/seg-stalled segments/$SEG/stalled >/dev/null 2>&1 && stall_pend=0; }
  if [ $resume_pend = 1 ] && [ $stall_pend = 0 ]; then
    vmx-gcs put /dev/null segments/$SEG/resumed-$HOST >/dev/null 2>&1 && resume_pend=0
  fi
  # Last column: the node's RSS in KiB (largest celestia-rs process), so
  # segments on different builds can be compared over a whole segment.
  rss=$(ps -o rss= -C celestia-rs 2>/dev/null | sort -n | tail -1 | tr -d ' ')
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "${h:--}" "$a" "$HOST" "$reason" "${rss:--}" >> $T
  # Memory breakdown of the follow process (KiB): smaps_rollup Rss and
  # Anonymous, status RssAnon/RssFile/VmHWM and the thread count.
  pid=$(pgrep -f 'celestia-rs follow' | head -1)
  if [ -n "$pid" ] && [ -r /proc/$pid/smaps_rollup ]; then
    printf '%s\t%s\t%s%s\n' "$(date +%s)" "${h:--}" \
      "$(awk '$1 ~ /^(Rss|Anonymous):$/ {k=$1; sub(/:$/, "", k); printf "%s=%s ", k, $2}' /proc/$pid/smaps_rollup)" \
      "$(awk '$1 ~ /^(RssAnon|RssFile|VmHWM|Threads):$/ {k=$1; sub(/:$/, "", k); printf "%s=%s ", k, $2}' /proc/$pid/status)" >> $DATA/mem-$SEG.tsv
    vmx-gcs put $DATA/mem-$SEG.tsv rs-replay/$SEG/mem-$HOST.tsv >/dev/null 2>&1
  fi
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
  # The replay has ended only if its job is down on two consecutive polls: a
  # deliberate restart (stop, then start again) shows it down for one.
  if [ "$a" = active ] || [ "$a" = activating ]; then down=0; else down=$((${down:-0} + 1)); fi
  if [ "$down" -ge 2 ]; then
    [ -n "$L" ] && { tail -n 300 "$L" > $DATA/stop-tail.log; vmx-gcs put $DATA/stop-tail.log rs-replay/$SEG/stop-tail-$HOST.log; }
    if ls -d $HOME_DIR/follow-diagnostics/* >/dev/null 2>&1; then
      tar -czf $DATA/diag.tgz -C $HOME_DIR follow-diagnostics && vmx-gcs put $DATA/diag.tgz rs-replay/$SEG/diagnostics-$HOST.tgz
    fi
    # follow logs its final counters on both success and error. Only the
    # next authenticated header proves the effects of the final block.
    if [ -n "$L" ] && grep -qE "(^|[[:space:]])INFO celestia_rs::follow: stop height reached height=$STOP verified_header_height=$((STOP + 1))( |$)" "$L" &&
      grep -qE "(^|[[:space:]])INFO celestia_rs::follow: follow finished .*last=$STOP( |$)" "$L"; then
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
