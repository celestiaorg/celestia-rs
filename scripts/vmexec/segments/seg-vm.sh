#!/bin/bash
# On a segment VM: restore a format-2/3 snapshot from the bucket into
# celestia-rs (built at /opt/celestia-rs) and replay to STOP with follow,
# writing Rust snapshots every 50,000 heights.
#   seg-vm.sh SEG SRC START STOP
# SEG   segment id (the Go export height it started from; bucket paths use it)
# SRC   bucket prefix of the snapshot to restore: go-snapshots/<START> or,
#       on a restart, rs-snapshots/from-go-<SEG>/<START>
# If the bucket also holds a Rust snapshot at START from another run
# (rs-snapshots/<START>/), compare-snapshots checks both item by item first.
set -uo pipefail
SEG=$1 SRC=$2 H=$3 STOP=$4
# Block sources: pops and Numia serve every height and scale to ~40 blocks/s
# at 32 requests in flight (rs-replay/rpctest/, 2026-11-27); Polkachu's
# archive RPC answers 429 above ~4-8 in flight, so it is not used for fetching.
# PREFETCH keeps all segments together under ~64 requests per provider:
# 15 segments x 8 in flight, spread over two providers, is ~60 each.
RPC=https://rpc.celestia.pops.one,https://public-celestia-rpc.numia.xyz
PREFETCH=${PREFETCH:-8}
# Optional per-segment block sources (segments/SEG/rpc, one comma-separated
# line): follow spreads heights over the list, so repeating pops and Numia
# gives a third server a small fixed share.
vmx-gcs get segments/$SEG/rpc /data/rpc-$SEG 2>/dev/null && [ -s /data/rpc-$SEG ] && RPC=$(head -1 /data/rpc-$SEG)
B=/opt/celestia-rs/target/release/celestia-rs
S=/data/snap; mkdir -p $S/$H
vmx-gcs get $SRC/2/metadata.json /data/meta-$H.json 2>/dev/null && fmt=2 ||
  { vmx-gcs get $SRC/3/metadata.json /data/meta-$H.json && fmt=3; } || { echo "no snapshot under $SRC"; exit 1; }
n=$(jq -r .chunk_count /data/meta-$H.json)
mkdir -p $S/$H/$fmt
for ((i = 0; i < n; i++)); do [ -s $S/$H/$fmt/$i ] || vmx-gcs get $SRC/$fmt/$i $S/$H/$fmt/$i || exit 1; done
TH=$(jq -r '.block_hash // empty' /data/meta-$H.json)
if [ -z "$TH" ]; then
  a=$(curl -fsS -m 30 "https://rpc.celestia.pops.one/block?height=$H" | jq -r .result.block_id.hash)
  b=$(curl -fsS -m 30 "https://public-celestia-rpc.numia.xyz/block?height=$H" | jq -r .result.block_id.hash)
  [ -n "$a" ] && [ "$a" = "$b" ] || { echo "archive RPCs disagree on the hash of $H"; exit 1; }
  TH=$a
fi
echo "segment $SEG: restoring $SRC ($n chunks, format $fmt) at $H, trust hash $TH, to $STOP; commit $(git -C /opt/celestia-rs rev-parse HEAD)"
# Full-state check against an independent Rust snapshot at the same height.
if [ "$SRC" = go-snapshots/$H ] && vmx-gcs get rs-snapshots/$H/$fmt/metadata.json /data/rsmeta-$H.json 2>/dev/null; then
  R=/data/rs-$H; mkdir -p $R/$H/$fmt
  m=$(jq -r .chunk_count /data/rsmeta-$H.json)
  for ((i = 0; i < m; i++)); do vmx-gcs get rs-snapshots/$H/$fmt/$i $R/$H/$fmt/$i || break; done
  $B compare-snapshots --height $H --format $fmt $R $S > /data/compare-$H.txt 2>&1
  echo "compare-snapshots rc=$?" >> /data/compare-$H.txt
  vmx-gcs put /data/compare-$H.txt rs-replay/$SEG/compare-$H.txt
  tail -3 /data/compare-$H.txt
fi
# follow has a --prefetch flag that mainnet-snapshot-follow.sh does not pass.
printf '#!/bin/bash\nif [ "$1" = follow ]; then exec %s "$@" --prefetch %s; else exec %s "$@"; fi\n' "$B" "$PREFETCH" "$B" >/data/celestia-rs-follow
chmod 755 /data/celestia-rs-follow
export RPC TRUST_HASH=$TH CELESTIA_RS_BIN=/data/celestia-rs-follow CELESTIA_RS_HOME=/data/home-$SEG SNAPSHOT_INTERVAL=50000 SNAPSHOT_KEEP=0
bash /opt/celestia-rs/scripts/mainnet-snapshot-follow.sh $S $H $STOP
