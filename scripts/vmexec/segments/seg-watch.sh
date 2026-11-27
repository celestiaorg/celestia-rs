#!/bin/bash
# Segment watcher (on the segment VM): every 5 min append
# "time height state reason" to rs-replay/SEG/heights.tsv, upload finished
# Rust snapshots to rs-snapshots/from-go-SEG/<h>/<f>/ (metadata.json last),
# and when the replay ends upload the log tail and diagnostics and write
# segments/SEG/done (reached STOP) or segments/SEG/failed (with the reason).
#   seg-watch.sh SEG STOP
set -u
SEG=$1 STOP=$2 HOME_DIR=/data/home-$1 T=/data/heights-$1.tsv HOST=$(hostname)
while true; do
  L=$(ls -t $HOME_DIR/logs/*.log 2>/dev/null | head -1)
  h=''; [ -n "$L" ] && h=$(grep -oE 'followed height=[0-9]+' "$L" | tail -1 | cut -d= -f2)
  a=$(systemctl is-active vmxjob-seg)
  reason=''
  [ "$a" = active ] || { [ -n "$L" ] && reason=$(grep -E '^Error|ERROR' "$L" | tail -1 | cut -c1-240); }
  printf '%s\t%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "${h:--}" "$a" "$HOST" "$reason" >> $T
  vmx-gcs put $T rs-replay/$SEG/heights-$HOST.tsv >/dev/null 2>&1
  for d in $HOME_DIR/data/snapshots/*/*/; do
    [ -d "$d" ] || continue
    sh=$(basename "$(dirname "$d")"); f=$(basename "$d")
    [[ $sh =~ ^[0-9]+$ && $f =~ ^[0-9]+$ ]] || continue
    [ -e /data/uploaded-$sh-$f ] && continue
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
        --arg written_at "$(date -u +%FT%TZ)" '$ARGS.named' > /data/meta-rs-$sh.json
      vmx-gcs put /data/meta-rs-$sh.json rs-snapshots/from-go-$SEG/$sh/$f/metadata.json && touch /data/uploaded-$sh-$f
    fi
  done
  if [ "$a" != active ] && [ "$a" != activating ]; then
    [ -n "$L" ] && { tail -n 300 "$L" > /data/stop-tail.log; vmx-gcs put /data/stop-tail.log rs-replay/$SEG/stop-tail-$HOST.log; }
    if ls -d $HOME_DIR/follow-diagnostics/* >/dev/null 2>&1; then
      tar -czf /data/diag.tgz -C $HOME_DIR follow-diagnostics && vmx-gcs put /data/diag.tgz rs-replay/$SEG/diagnostics-$HOST.tgz
    fi
    if [ -n "$L" ] && grep -qE "follow finished .*last=$STOP( |$)" "$L"; then
      echo "done $(date -u +%FT%TZ) $HOST $(git -C /opt/celestia-rs rev-parse HEAD)" > /data/seg-state
      vmx-gcs put /data/seg-state segments/$SEG/done
    else
      echo "failed $(date -u +%FT%TZ) $HOST height=${h:--} ${reason:-no log}" > /data/seg-state
      vmx-gcs put /data/seg-state segments/$SEG/failed
    fi
    exit 0
  fi
  sleep 300
done
