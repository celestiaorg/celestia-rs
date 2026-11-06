#!/usr/bin/env bash
# Compare consensus-relevant transaction result fields at every height in
# one finite inclusive range. RPC reads only; no validator state is changed.
# Usage: compare-block-results.sh LOCAL_RPC REFERENCE_RPC FIRST LAST RECEIPTS_FILE [CANDIDATE_COMMIT]
set -uo pipefail

if (( $# < 5 || $# > 6 )); then
  echo "usage: $0 LOCAL_RPC REFERENCE_RPC FIRST LAST RECEIPTS_FILE [CANDIDATE_COMMIT]" >&2
  exit 2
fi
local_rpc=$1 ref_rpc=$2 first=$3 last=$4 receipts=$5 candidate=${6:-}
if [[ ! $first =~ ^[0-9]+$ || ! $last =~ ^[0-9]+$ ]] ||
  (( first < 1 || last < first || last - first >= 10000 )); then
  echo "range must be 1–10,000 inclusive positive heights" >&2
  exit 2
fi

mkdir -p "$(dirname "$receipts")" || exit 2
(set -C; umask 077; : > "$receipts") || {
  echo "receipt file already exists or is unwritable: $receipts" >&2
  exit 2
}

# Only code, data and gas enter the consensus LastResultsHash. Keep the
# comparison scope explicit in each receipt, and never imply that logs,
# events or all ABCI fields were compared.
normalize='
  if ((.result.height | tonumber?) == $h) and
     ((.result.txs_results == null) or ((.result.txs_results | type) == "array"))
  then [.result.txs_results[]? |
    {code:(.code // 0 | tostring), data:(.data // ""),
     gas_wanted:(.gas_wanted // 0 | tostring),
     gas_used:(.gas_used // 0 | tostring)}]
  else empty end'

chain= ref_chain= status_error=
local_status=$(curl -fsS -m 10 "$local_rpc/status" 2>/dev/null) || status_error=local_status_unavailable
if [[ -z $status_error ]]; then
  chain=$(jq -r '.result.node_info.network // empty' <<<"$local_status" 2>/dev/null)
  [[ -n $chain ]] || status_error=invalid_local_chain_id
fi
if [[ -z $status_error ]]; then
  ref_status=$(curl -fsS -m 10 "$ref_rpc/status" 2>/dev/null) || status_error=reference_status_unavailable
fi
if [[ -z $status_error ]]; then
  ref_chain=$(jq -r '.result.node_info.network // empty' <<<"$ref_status" 2>/dev/null)
  [[ -n $ref_chain ]] || status_error=invalid_reference_chain_id
fi
[[ -z $status_error && $chain != "$ref_chain" ]] && status_error=reference_chain_mismatch

overall=0
for ((h=first; h<=last; h++)); do
  time_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  outcome=unavailable error=$status_error local_norm= ref_norm=
  local_count= ref_count= local_digest= ref_digest= difference=
  if [[ -z $error ]]; then
    local_response=$(curl -fsS -m 10 "$local_rpc/block_results?height=$h" 2>/dev/null) ||
      error=local_block_results_unavailable
  fi
  if [[ -z $error ]]; then
    local_norm=$(jq -cS --argjson h "$h" "$normalize" <<<"$local_response" 2>/dev/null)
    [[ -n $local_norm ]] || error=invalid_local_block_results
  fi
  if [[ -z $error ]]; then
    ref_response=$(curl -fsS -m 10 "$ref_rpc/block_results?height=$h" 2>/dev/null) ||
      error=reference_block_results_unavailable
  fi
  if [[ -z $error ]]; then
    ref_norm=$(jq -cS --argjson h "$h" "$normalize" <<<"$ref_response" 2>/dev/null)
    [[ -n $ref_norm ]] || error=invalid_reference_block_results
  fi
  if [[ -z $error ]]; then
    local_count=$(jq 'length' <<<"$local_norm")
    ref_count=$(jq 'length' <<<"$ref_norm")
    local_digest=$(printf '%s' "$local_norm" | shasum -a 256 | cut -d ' ' -f 1)
    ref_digest=$(printf '%s' "$ref_norm" | shasum -a 256 | cut -d ' ' -f 1)
    if [[ $local_norm == "$ref_norm" ]]; then
      outcome=match
    else
      outcome=mismatch
      difference=$(jq -n --argjson a "$local_norm" --argjson b "$ref_norm" \
        '[range(0; [($a|length),($b|length)]|max) | select($a[.] != $b[.])][0]')
      overall=1
    fi
  else
    overall=2
  fi
  line=$(jq -nc --arg time "$time_utc" --arg chain "$chain" --arg ref_chain "$ref_chain" \
    --argjson h "$h" --arg local "$local_rpc" --arg ref "$ref_rpc" --arg candidate "$candidate" \
    --arg outcome "$outcome" --arg error "$error" --arg lc "$local_count" --arg rc "$ref_count" \
    --arg ld "$local_digest" --arg rd "$ref_digest" --arg difference "$difference" \
    '{time_utc:$time,chain_id:(if $chain=="" then null else $chain end),
      reference_chain_id:(if $ref_chain=="" then null else $ref_chain end),
      executed_height:$h,local_rpc:$local,reference_rpc:$ref,
      candidate_commit:(if $candidate=="" then null else $candidate end),
      compared_fields:["code","data","gas_wanted","gas_used"],
      local_tx_count:(if $lc=="" then null else ($lc|tonumber) end),
      reference_tx_count:(if $rc=="" then null else ($rc|tonumber) end),
      local_results_sha256:(if $ld=="" then null else $ld end),
      reference_results_sha256:(if $rd=="" then null else $rd end),
      first_difference_index:(if $difference=="" then null else ($difference|tonumber) end),
      outcome:$outcome,error:(if $error=="" then null else $error end)}') || exit 2
  printf '%s\n' "$line" | tee -a "$receipts" || exit 2
done
exit "$overall"
