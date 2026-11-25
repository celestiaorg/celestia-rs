# shellcheck shell=bash disable=SC2034 # constants are used by checkpoints-vm.sh and the workflow
# Pure helpers for the Go mainnet checkpoint campaign
# (conformance/README.md, "Where checkpoints come from"). Sourced after
# lib.sh by checkpoints-vm.sh (on the VM), by the genesis-checkpoints
# workflow and by checkpoints-test.sh. No function here touches the network.

# The state before each replay window after v1, plus 2,371,489: five v1
# blocks before the v1 to v2 upgrade, which runs in the Commit of 2,371,494.
CKPT_DEFAULT_HEIGHTS=2371489,2371494,2993219,6680339,6748821,8662012,10960599,11771698,15105000
# First mainnet height of each app version after 1. v7 never ran on mainnet.
# Windows 2 to 8 of conformance/fixtures/mainnet/replay-windows.json start
# at these heights; the v10 height is the first block after v9's last.
CKPT_UPGRADES=(2:2371495 3:2993220 4:6680340 5:6748822 6:8662013 8:10960600 9:11771699 10:15116658)
# Embedded binaries of the multiplexer at BENCH_GO_TAG
# (internal/embedding/data.go). App versions 1 to 3 run in v3.13.0.
CKPT_EMBEDDED=(1:v3.13.0 2:v3.13.0 3:v3.13.0 4:v4.1.0 5:v5.0.12 6:v6.4.10 8:v8.0.8 9:v9.0.8 10:v10.6.0)
# Archive RPCs whose /status reported earliest_block_height 1 on 2026-11-25.
CKPT_REF_RPCS=(https://celestia-archive-rpc.polkachu.com https://rpc.celestia.pops.one)
# P2P addresses of archive nodes: Polkachu's archive RPC node (its /status
# listen address) and the node with moniker celestia-archive-CroutonDigital
# in that node's /net_info. Whether they serve blocks from height 1 over P2P
# is checked only by the first sync.
CKPT_ARCHIVE_PEERS=c04516cd6cb0e8980dd95a8ea4d6d4aabfd4d984@65.109.19.189:11656,fb779c787dda02b66c061ce030f77682494cc4c8@65.21.231.168:11656
CKPT_EGRESS_CAP=4mbit # the operator benchmark's cap; see the workflow's price table

# ckpt_valid KIND VALUE: strict format check for every external input.
ckpt_valid() {
  case $1 in
    name) [[ $2 =~ ^[a-z][a-z0-9-]{0,19}$ ]] ;;
    heights) [[ $2 =~ ^[1-9][0-9]{0,7}(,[1-9][0-9]{0,7}){0,15}$ ]] ;;
    disk_gb) [[ $2 =~ ^[1-9][0-9]{2,3}$ ]] && (($2 >= 200 && $2 <= 1000)) ;;
    hours) [[ $2 =~ ^[1-9][0-9]{0,2}$ ]] && (($2 >= 6 && $2 <= 168)) ;;
    disks) [[ $2 =~ ^(none|data|all)$ ]] ;;
    hash) [[ $2 =~ ^[0-9A-F]{64}$ ]] ;;
    *) return 1 ;;
  esac
}

# ckpt_app_version H: the app version that executed block H on mainnet.
ckpt_app_version() {
  local v=1 e
  for e in "${CKPT_UPGRADES[@]}"; do ((${e#*:} <= $1)) && v=${e%%:*}; done
  echo "$v"
}

# ckpt_embedded_tag V: the embedded celestia-app release that runs app version V.
ckpt_embedded_tag() {
  local e
  for e in "${CKPT_EMBEDDED[@]}"; do [[ ${e%%:*} == "$1" ]] && { echo "${e#*:}"; return; }; done
  return 1
}

# ckpt_export_cmd H: the celestia-appd subcommand prefix whose binary holds
# the state at H. Each embedded app reads the same home; v1 to v3 state is
# only readable by v3.13.0.
ckpt_export_cmd() {
  local v
  v=$(ckpt_app_version "$1")
  echo "passthrough v$((v < 3 ? 3 : v))"
}

# ckpt_format V: the SDK snapshot format of app version V (v0.46: 2, v0.50: 3).
ckpt_format() { if (($1 <= 3)); then echo 2; else echo 3; fi; }

# ckpt_plan HEIGHTS: "H app_version" per line, ascending and unique.
ckpt_plan() {
  local h
  tr , '\n' <<<"$1" | sort -nu | while read -r h; do echo "$h $(ckpt_app_version "$h")"; done
}

# ckpt_next DIR HEIGHTS: the lowest planned height with neither
# DIR/H/checkpoint.json nor DIR/H/skipped.json.
ckpt_next() {
  local h _
  while read -r h _; do
    [[ -s $1/$h/checkpoint.json || -s $1/$h/skipped.json ]] || { echo "$h"; return 0; }
  done < <(ckpt_plan "$2")
  return 1
}

# ckpt_halted LOG H: LOG shows the multiplexer refusing block H+1 because of
# halt-height = H+1. It refuses FinalizeBlock at heights >= halt-height, so
# the last committed height is H.
ckpt_halted() { grep -qE "halting node per configuration at height $(($2 + 1))([^0-9]|$)" "$1"; }

# ckpt_chunks DIR: JSON array of {index, bytes, sha256} for the chunk files
# 0..n-1 of one snapshot; fails on gaps or no chunks.
ckpt_chunks() {
  local n=0 f i
  [[ -d $1 ]] || return 1
  while [[ -f $1/$n ]]; do n=$((n + 1)); done
  ((n > 0)) || return 1
  for f in "$1"/*; do [[ ${f##*/} =~ ^[0-9]+$ ]] && ((${f##*/} >= n)) && return 1; done
  for ((i = 0; i < n; i++)); do
    jq -nc --argjson i "$i" --argjson b "$(stat -c %s "$1/$i")" --arg s "$(sha256sum "$1/$i" | cut -d' ' -f1)" \
      '{index: $i, bytes: $b, sha256: $s}'
  done | jq -sc .
}

# ckpt_metadata DIR KEY=VALUE...: checkpoint metadata for the chunk directory
# <root>/<height>/<format>, with each KEY=VALUE added as a string field.
ckpt_metadata() {
  local d=$1 kv chunks args=()
  shift
  for kv in "$@"; do
    [[ $kv == *=* ]] || return 1
    args+=(--arg "${kv%%=*}" "${kv#*=}")
  done
  chunks=$(ckpt_chunks "$d") || return 1
  jq -n --argjson height "$(basename "$(dirname "$d")")" --argjson format "$(basename "$d")" \
    --argjson chunks "$chunks" "${args[@]}" \
    '$ARGS.named + {height: $height, format: $format, chunk_count: ($chunks | length),
      total_bytes: ([$chunks[].bytes] | add), chunks: $chunks}'
}

# ckpt_rate POLLS WINDOW_S: blocks per second over the last WINDOW_S seconds
# of POLLS lines "epoch_s height ...", or "-".
ckpt_rate() {
  awk -v w="$2" 'BEGIN { n = 0 } $1 ~ /^[0-9]+$/ && $2 ~ /^[0-9]+$/ { t[n] = $1 + 0; h[n] = $2 + 0; n++ }
    END {
      if (n < 2) { print "-"; exit }
      for (i = 0; i < n - 1 && t[i] < t[n - 1] - w; i++) {}
      if (t[n - 1] == t[i]) { print "-"; exit }
      printf "%.2f\n", (h[n - 1] - h[i]) / (t[n - 1] - t[i])
    }' "$1"
}

# ckpt_eta RATE HEIGHT TARGET: seconds to TARGET at RATE, or "-".
ckpt_eta() {
  awk -v r="$1" -v h="$2" -v g="$3" 'BEGIN {
    if (h >= g) { print 0; exit }
    if (r !~ /^[0-9.]+$/ || r + 0 <= 0) { print "-"; exit }
    printf "%d\n", (g - h) / r }'
}
