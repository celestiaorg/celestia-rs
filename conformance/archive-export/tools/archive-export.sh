#!/bin/bash
# Export, verify and upload Go state-sync checkpoints from the downloaded
# QubeLabs archive application.db (/data/appdb). One height per run step;
# a height with checkpoint.json is skipped on rerun.
#   HEIGHTS="2371489:v046 6680339:v050 ..." bash archive-export.sh
set -uo pipefail
OUT=/data/ckpt; T=/data/tools; RS=/data/celestia-rs
HOME_DATA=/data/home/data
REF=(https://celestia-archive-rpc.polkachu.com https://rpc.celestia.pops.one)
# Default, in priority order: early v0.46 heights first (restore tests), the
# v1 to v3 era every 250,000 heights and its boundaries, then the v0.50 era
# boundaries and every 500,000 heights between them.
if [ -z "${HEIGHTS:-}" ]; then
  HEIGHTS="120000:v046 250000:v046 500000:v046 2371494:v046 2993219:v046 6680339:v046"
  for h in $(seq 750000 250000 6500000); do HEIGHTS+=" $h:v046"; done
  HEIGHTS+=" 2371489:v046 6680339:v050 6748821:v050 8662012:v050 10960599:v050 11771698:v050 15105000:v050"
  for h in $(seq 7000000 500000 15000000); do HEIGHTS+=" $h:v050"; done
fi
app_version() { # first mainnet height of app versions 2..9 (v7 never ran)
  local h=$1 v=1 e
  for e in 2:2371495 3:2993220 4:6680340 5:6748822 6:8662013 8:10960600 9:11771699 10:15116659; do ((${e#*:} <= h)) && v=${e%%:*}; done
  echo $v
}
mkdir -p $OUT $T $HOME_DATA
[ -e $HOME_DATA/application.db ] || ln -s /data/appdb $HOME_DATA/application.db
for v in v046 v050; do [ -x $T/archive-export-$v ] || { vmx-gcs get tools/archive-export-$v $T/archive-export-$v && chmod 755 $T/archive-export-$v; }; done
[ -d $RS/.git ] || git clone -q --depth 1 https://github.com/celestiaorg/celestia-rs $RS
command -v jq >/dev/null || { apt-get -o DPkg::Lock::Timeout=600 update -q >/dev/null; apt-get -o DPkg::Lock::Timeout=600 install -yq jq >/dev/null; }
ulimit -n 2000000
ref() { # ref PATH JQ: value both archive RPCs agree on
  local a b
  a=$(curl -fsS -m 30 "${REF[0]}/$1" | jq -r "$2") && b=$(curl -fsS -m 30 "${REF[1]}/$1" | jq -r "$2") && [ "$a" = "$b" ] && echo "$a"
}
put_retry() { # put_retry FILE OBJECT: upload, five tries
  local i
  for i in 1 2 3 4 5; do
    vmx-gcs put "$1" "$2" && return 0
    sleep $((i * 5))
  done
  return 1
}
for hv in $HEIGHTS; do
  h=${hv%%:*} v=${hv#*:} fmt=$([ $v = v046 ] && echo 2 || echo 3)
  d=$OUT/$h-$v
  [ -s $d/checkpoint.json ] && { echo "$h $v done"; continue; }
  rm -rf $d; mkdir -p $d
  echo "=== $(date -u +%T) export $h with $v"
  s=$(date +%s)
  if ! $T/archive-export-$v -db $HOME_DATA -height $h -out $d/snapshots > $d/export.log 2>&1; then
    echo "export $h $v failed:"; tail -5 $d/export.log; continue
  fi
  echo "exported in $(( $(date +%s) - s ))s: $(tail -1 $d/export.log)"
  app_hash=$(ref "block?height=$((h + 1))" .result.block.header.app_hash)
  block_hash=$(ref "block?height=$h" .result.block_id.hash)
  window=$(jq -r --argjson s $((h + 1)) '.windows[] | select(.start_height == $s) | .app_version' $RS/conformance/fixtures/mainnet/replay-windows.json)
  args=(); [ -n "$window" ] || args=(--app-hash "$app_hash")
  python3 $RS/conformance/mainnet-rpc/verify-replay-checkpoint.py $d/snapshots/$h/$fmt "${args[@]}" > $d/verify.log 2>&1; vrc=$?
  echo "verify rc=$vrc: $(tail -2 $d/verify.log | tr '\n' ' ')"
  chunks=$(ls $d/snapshots/$h/$fmt | wc -l); bytes=$(du -sb $d/snapshots/$h/$fmt | cut -f1)
  jq -n --argjson height $h --argjson format $fmt --arg exporter "archive-export-$v" --argjson app_version "$(app_version $h)" \
    --arg go_binary "$([ $v = v046 ] && echo "celestia-app v3.13.0 cosmos-sdk v1.31.2-sdk-v0.46.16 iavl v0.19.7" || echo "cosmossdk.io/store v1.1.2 iavl v1.2.6 (celestia-app v4 store)")" --arg source "snaps.qubelabs.io/celestia/celestia_2026-11-03.tar application.db" \
    --arg block_hash "$block_hash" --arg next_header_app_hash "$app_hash" --arg verified_against "$([ -n "$window" ] && echo "replay window $window" || echo "header $((h + 1))")" \
    --argjson verified "$([ $vrc = 0 ] && echo true || echo false)" --argjson chunk_count $chunks --argjson total_bytes $bytes \
    --arg export_output "$(tail -3 $d/export.log)" --arg verify_output "$(tail -c 2000 $d/verify.log)" --arg exported_at "$(date -u +%FT%TZ)" \
    '$ARGS.named' > $d/checkpoint.tmp
  name=$([ $vrc = 0 ] && echo checkpoint.json || echo failed.json)
  mv $d/checkpoint.tmp $d/$name
  # The metadata object goes last: it marks a complete upload. A failed
  # upload leaves no local done-marker, so the next run exports this
  # height again instead of skipping it.
  uploaded=1
  for f in $(cd $d && find snapshots/$h -type f | sort) export.log verify.log $name; do
    # gs://<bucket>/go-snapshots/<height>/<format>/<chunk> and .../<format>/metadata.json
    case $f in
      snapshots/*) obj=go-snapshots/$h/${f#snapshots/"$h"/} ;;
      "$name") obj=go-snapshots/$h/$fmt/$([ "$name" = checkpoint.json ] && echo metadata.json || echo failed.json) ;;
      *) obj=go-snapshots/$h/$fmt/$f ;;
    esac
    put_retry $d/$f "$obj" || { echo "upload $f failed"; uploaded=0; break; }
  done
  if [ $uploaded = 0 ]; then
    mv $d/$name $d/upload-failed.json
    echo "$h $v upload incomplete; the next run exports it again"
    continue
  fi
  echo "$h $v -> $name, $chunks chunks, $bytes bytes"
done
