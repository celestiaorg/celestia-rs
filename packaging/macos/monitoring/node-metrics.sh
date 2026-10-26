#!/usr/bin/env bash
# Writes celestia-rs node health as Prometheus text files for
# node_exporter's textfile collector (docs/operator-guide.md §11.9).
#
#   node-metrics.sh collect <nodes.conf> <textfile-dir>   # every 30 s
#   node-metrics.sh apphash <nodes.conf> <textfile-dir>   # every 5 min
#
# `collect` adds what the node's own /metrics lacks on macOS: process
# memory and CPU, data dir size, /status, and the chain's slashing view
# of our validator (missed blocks in the window, jailed_until,
# tombstoned) and staking's jailed flag for it.
# `apphash` runs check-apphash.sh against each node's reference RPC.
#
# nodes.conf has one node per line: <network> <home> <local-rpc>
# <reference-rpc> [<log file> [<valoper>]]. With a log file (`-` for
# none), `collect` also counts the double-sign guard's refusals in it.
# With the validator's operator address, it reports whether staking has
# the validator jailed.
# Needs curl and jq. Runs under macOS's bash 3.2.
set -uo pipefail
export PATH=$PATH:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin

mode=${1:?usage: node-metrics.sh collect|apphash <nodes.conf> <textfile-dir>}
conf=${2:?nodes.conf}
dir=${3:?textfile dir}
here=$(cd "$(dirname "$0")" && pwd)
CELESTIA_RS_BIN=${CELESTIA_RS_BIN:-$HOME/.celestia-rs/bin/celestia-rs}
CHECK_APPHASH=${CHECK_APPHASH:-$here/check-apphash.sh}

out=""
emit() { out+="$1{network=\"$2\"} $3"$'\n'; }
help() { out+="# HELP $1 $3"$'\n'"# TYPE $1 $2"$'\n'; }

# "2026-10-19T14:26:41.178Z" -> unix seconds (sub-seconds dropped).
epoch() { date -j -u -f "%Y-%m-%dT%H:%M:%S" "${1%%.*}" +%s 2>/dev/null; }

# ps TIME/ETIME "[[dd-]hh:]mm:ss[.cc]" -> seconds.
duration() {
  local t=$1 d=0 h=0 m s
  case $t in *-*) d=${t%%-*}; t=${t#*-} ;; esac
  IFS=: read -r a b c <<<"$t"
  if [[ -n ${c:-} ]]; then h=$a; m=$b; s=$c; else m=$a; s=$b; fi
  awk -v d="$d" -v h="$h" -v m="$m" -v s="$s" 'BEGIN { printf "%.2f", d*86400 + h*3600 + m*60 + s }'
}

publish() { # atomic replace, so node_exporter never reads a partial file
  mkdir -p "$dir"
  local tmp
  tmp=$(mktemp "$dir/.$1.XXXXXX")
  printf '%s' "$out" >"$tmp" && chmod 644 "$tmp" && mv "$tmp" "$dir/$1"
}

nodes() { grep -vE '^[[:space:]]*(#|$)' "$conf"; }

# Log lines of a refusal by the double-sign guard (operator guide §11.11).
REFUSAL='failed signing vote|failed signing proposal|found signature from the same key'

# sign_refusals <network> <log>: the number of refusal lines the log has
# had, counted from where the last run stopped ($dir/.sign-refusals-<net>
# keeps "<inode> <offset> <count>"). Only complete lines are read; a new
# inode or a shorter file (rotated, truncated) is read from the start.
sign_refusals() {
  local f=$dir/.sign-refusals-$1 log=$2 ino off=0 n=0 size sino chunk len part
  read -r sino off n 2>/dev/null <"$f" || { sino=; off=0; n=0; }
  ino=$(stat -f %i "$log") || return 1
  size=$(stat -f %z "$log") || return 1
  [[ $ino == "$sino" ]] && ((size >= off)) || off=0
  if ((size > off)); then
    chunk=$(mktemp "$dir/.chunk.XXXXXX")
    tail -c +$((off + 1)) "$log" | head -c $((size - off)) >"$chunk"
    len=$((size - off))
    # Leave a trailing partial line for the next run.
    part=0
    [[ $(tail -c 1 "$chunk" | od -An -c | tr -d ' ') == '\n' ]] || part=$(tail -n 1 "$chunk" | wc -c)
    len=$((len - part))
    n=$((n + $(head -c "$len" "$chunk" | grep -cE "$REFUSAL")))
    off=$((off + len))
    rm -f "$chunk"
  fi
  echo "$ino $off $n" >"$f"
  echo "$n"
}

collect() {
  local procs now
  procs=$(ps -axo pid=,rss=,time=,args=)
  now=$(date -u +%s)
  help celestia_rs_rpc_up gauge "1 if the node's local RPC /status answered."
  help celestia_rs_catching_up gauge "1 while the node is block-syncing or state-syncing."
  help celestia_rs_latest_block_height gauge "latest_block_height from /status."
  help celestia_rs_latest_block_time_seconds gauge "latest_block_time from /status, unix seconds."
  help celestia_rs_process_up gauge "1 if a 'start --home <home>' process exists."
  help celestia_rs_process_resident_memory_bytes gauge "RSS of the node process."
  help celestia_rs_process_cpu_seconds_total counter "CPU time of the node process."
  help celestia_rs_data_dir_bytes gauge "Size of <home>/data."
  help celestia_rs_validator_signing_info_found gauge "1 if x/slashing has signing info for this node's consensus key."
  help celestia_rs_validator_missed_blocks_window gauge "missed_blocks_counter in the current signed_blocks_window."
  help celestia_rs_validator_signed_blocks_window gauge "x/slashing signed_blocks_window."
  help celestia_rs_validator_min_signed_per_window gauge "x/slashing min_signed_per_window."
  help celestia_rs_validator_jailed gauge "1 if x/staking has the validator jailed or it is tombstoned; absent if unknown."
  help celestia_rs_validator_jailed_until_seconds gauge "jailed_until, unix seconds: the earliest block time at which unjail is allowed."
  help celestia_rs_validator_tombstoned gauge "1 if tombstoned."
  help celestia_rs_sign_refusals_total counter "Double-sign guard refusals in the node's log (nodes.conf log column)."
  help celestia_rs_collector_timestamp_seconds gauge "When this file was written."

  local net home rpc ref log valoper
  mkdir -p "$dir"
  while read -r net home rpc ref log valoper; do
    [[ $log == - ]] && log=
    local st h bt cu line bin valcons si params
    st=$(curl -fsS -m 5 "$rpc/status" 2>/dev/null)
    h=$(jq -r '.result.sync_info.latest_block_height // empty' <<<"$st" 2>/dev/null)
    if [[ -n $h ]]; then
      emit celestia_rs_rpc_up "$net" 1
      cu=$(jq -r '.result.sync_info.catching_up' <<<"$st")
      emit celestia_rs_catching_up "$net" "$([[ $cu == true ]] && echo 1 || echo 0)"
      emit celestia_rs_latest_block_height "$net" "$h"
      bt=$(epoch "$(jq -r '.result.sync_info.latest_block_time' <<<"$st")")
      [[ -n $bt ]] && emit celestia_rs_latest_block_time_seconds "$net" "$bt"
    else
      emit celestia_rs_rpc_up "$net" 0
    fi

    # Match the exact home so mocha and mainnet processes are told apart.
    line=$(awk -v h="$home" '$0 ~ (" start --home " h "$")' <<<"$procs" | head -1)
    bin=$CELESTIA_RS_BIN
    if [[ -n $line ]]; then
      local _pid rss cpu args
      read -r _pid rss cpu args <<<"$line"
      emit celestia_rs_process_up "$net" 1
      emit celestia_rs_process_resident_memory_bytes "$net" $((rss * 1024))
      emit celestia_rs_process_cpu_seconds_total "$net" "$(duration "$cpu")"
      [[ -x ${args%% *} ]] && bin=${args%% *}
    else
      emit celestia_rs_process_up "$net" 0
    fi

    [[ -d $home/data ]] && emit celestia_rs_data_dir_bytes "$net" $(($(du -sk "$home/data" | cut -f1) * 1024))
    if [[ -n $log && -f $log ]]; then
      local refusals
      refusals=$(sign_refusals "$net" "$log") && emit celestia_rs_sign_refusals_total "$net" "$refusals"
    fi

    # Signing info exists once the node's key is in a validator; a full
    # node gets NotFound and reports signing_info_found 0 only.
    [[ -d $home/config ]] || continue
    valcons=$("$bin" comet show-address --home "$home" </dev/null 2>/dev/null) || continue
    si=$("$bin" query slashing signing-info "$valcons" --home "$home" </dev/null 2>/dev/null)
    if [[ -z $si ]]; then
      emit celestia_rs_validator_signing_info_found "$net" 0
      continue
    fi
    emit celestia_rs_validator_signing_info_found "$net" 1
    emit celestia_rs_validator_missed_blocks_window "$net" "$(jq -r '.val_signing_info.missed_blocks_counter' <<<"$si")"
    local ju jus tomb jailed=
    ju=$(epoch "$(jq -r '.val_signing_info.jailed_until' <<<"$si")"); jus=${ju:-0}
    tomb=$(jq -r '.val_signing_info.tombstoned' <<<"$si")
    emit celestia_rs_validator_jailed_until_seconds "$net" "$jus"
    emit celestia_rs_validator_tombstoned "$net" "$([[ $tomb == true ]] && echo 1 || echo 0)"
    # jailed_until only says when unjail becomes allowed: the validator
    # stays jailed past it until an unjail tx, and during catch-up it can
    # already be in the wall-clock past. Staking's flag is the answer; if
    # it cannot be read, the metric is left out rather than reported 0.
    if [[ -n $valoper ]]; then
      jailed=$("$bin" query staking validator "$valoper" --home "$home" </dev/null 2>/dev/null |
        jq -r 'if .validator then (if .validator.jailed == true then 1 else 0 end) else empty end' 2>/dev/null)
    fi
    [[ $tomb == true ]] && jailed=1
    [[ -n $jailed ]] && emit celestia_rs_validator_jailed "$net" "$jailed"
    params=$("$bin" query slashing params --home "$home" </dev/null 2>/dev/null)
    if [[ -n $params ]]; then
      emit celestia_rs_validator_signed_blocks_window "$net" "$(jq -r '.params.signed_blocks_window' <<<"$params")"
      emit celestia_rs_validator_min_signed_per_window "$net" "$(jq -r '.params.min_signed_per_window' <<<"$params")"
    fi
  done < <(nodes)
  out+="celestia_rs_collector_timestamp_seconds $now"$'\n'
  publish celestia_rs.prom
}

apphash() {
  help celestia_rs_apphash_match gauge "1 app hash matches the reference RPC, 0 MISMATCH (parity bug), -1 check could not run."
  help celestia_rs_apphash_check_timestamp_seconds gauge "When the app hash was last checked."
  local net home rpc ref _rest rc
  while read -r net home rpc ref _rest; do
    "$CHECK_APPHASH" "$rpc" "$ref" </dev/null >/dev/null 2>&1; rc=$?
    case $rc in 0) emit celestia_rs_apphash_match "$net" 1 ;; 1) emit celestia_rs_apphash_match "$net" 0 ;; *) emit celestia_rs_apphash_match "$net" -1 ;; esac
    emit celestia_rs_apphash_check_timestamp_seconds "$net" "$(date -u +%s)"
  done < <(nodes)
  publish celestia_rs_apphash.prom
}

case $mode in
  collect) collect ;;
  apphash) apphash ;;
  *) echo "unknown mode: $mode" >&2; exit 2 ;;
esac
