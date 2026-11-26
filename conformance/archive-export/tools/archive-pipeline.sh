#!/bin/bash
# Fetch the application.db of the QubeLabs mainnet archive tar to /data/appdb.
# Each step leaves a .done marker, so a rerun resumes.
set -euo pipefail
U=https://snaps.qubelabs.io/celestia/celestia_2026-11-03.tar
TOTAL=10305672898560
W=/data/work; mkdir -p $W; cd $W
G=archive/qubelabs-celestia_2026-11-03
for f in tarmap.py tarindex.py archive-fetch.py; do [ -s $f ] || vmx-gcs get tools/$f $f; done
if [ ! -e tarmap.done ]; then
  python3 tarmap.py $U $TOTAL 2000 4194304 > tarmap.tsv
  gzip -kf tarmap.tsv && vmx-gcs put tarmap.tsv.gz $G/tarmap.tsv.gz; touch tarmap.done
fi
if [ ! -e appdb.idx.done ]; then
  start=$(awk -F'\t' '$4 ~ /^\.\/snapshots\// {o=$1} $4 ~ /^\.\/application\.db\// {print o; exit}' <(sort -n tarmap.tsv))
  end=$(awk -F'\t' '$4 ~ /^\.\/blockstore\.db\// {print $1; exit}' <(sort -n tarmap.tsv))
  echo "indexing $start..$end"
  python3 tarindex.py $U tarmap.tsv $start $end 48 > appdb.idx
  [ "$(grep -c BAD appdb.idx)" = 0 ] || { echo "index has BAD entries"; exit 1; }
  gzip -kf appdb.idx && vmx-gcs put appdb.idx.gz $G/appdb.idx.gz; touch appdb.idx.done
fi
SKIP=LOCK,MANIFEST-4824452 python3 archive-fetch.py $U appdb.idx /data/appdb ./application.db/ 48
touch fetch.done
