#!/usr/bin/env bash
# Compare the app hash after local height H with the reference header H+1.
# Usage: check-apphash.sh [local-rpc] [reference-rpc] [--json]
# JSON mode always emits one sanitized receipt, including failed checks.
set -uo pipefail

LOCAL=${1:-http://127.0.0.1:26657}
REF=${2:-https://rpc.celestia-mocha.com}
format=${3:-text}
[[ $format == text || $format == --json ]] || { echo "usage: $0 [local-rpc] [reference-rpc] [--json]" >&2; exit 2; }

time_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)
chain= ref_chain= height= reference_height= local_hash= reference_hash= error= outcome=unavailable
candidate=${CANDIDATE_COMMIT:-}

receipt() {
  if [[ $format == --json ]]; then
    jq -nc --arg time "$time_utc" --arg chain "$chain" --arg ref_chain "$ref_chain" \
      --arg height "$height" --arg reference_height "$reference_height" \
      --arg ours "$local_hash" --arg theirs "$reference_hash" \
      --arg local "$LOCAL" --arg reference "$REF" --arg outcome "$outcome" \
      --arg error "$error" --arg candidate "$candidate" \
      '{time_utc:$time,chain_id:(if $chain=="" then null else $chain end),
        reference_chain_id:(if $ref_chain=="" then null else $ref_chain end),
        executed_height:(if $height=="" then null else ($height|tonumber) end),
        local_app_hash:(if $ours=="" then null else $ours end),
        reference_header_height:(if $reference_height=="" then null else ($reference_height|tonumber) end),
        reference_app_hash:(if $theirs=="" then null else $theirs end),
        local_rpc:$local,reference_rpc:$reference,outcome:$outcome,
        error:(if $error=="" then null else $error end),
        candidate_commit:(if $candidate=="" then null else $candidate end)}'
  elif [[ $outcome == match ]]; then
    echo "OK height=$height app_hash=$local_hash"
  elif [[ $outcome == mismatch ]]; then
    echo "MISMATCH height=$height ours=$local_hash network=$reference_hash" >&2
  else
    echo "app-hash check unavailable: $error" >&2
  fi
}

fail() { error=$1; receipt; exit 2; }

info=$(curl -fsS -m 10 "$LOCAL/abci_info" 2>/dev/null) || fail local_abci_unavailable
height=$(jq -r '.result.response.last_block_height // empty' <<<"$info" 2>/dev/null)
[[ $height =~ ^[0-9]+$ ]] || { height=; fail invalid_local_height; }
encoded=$(jq -r '.result.response.last_block_app_hash // empty' <<<"$info" 2>/dev/null)
[[ -n $encoded ]] || fail missing_local_hash
local_hash=$(printf '%s' "$encoded" | base64 -d 2>/dev/null | od -An -v -tx1 | tr -d ' \n' | tr 'a-f' 'A-F')
[[ $local_hash =~ ^[0-9A-F]{64}$ ]] || { local_hash=; fail invalid_local_hash; }

local_status=$(curl -fsS -m 10 "$LOCAL/status" 2>/dev/null) || fail local_status_unavailable
chain=$(jq -r '.result.node_info.network // empty' <<<"$local_status" 2>/dev/null)
[[ -n $chain ]] || fail missing_local_chain_id
ref_status=$(curl -fsS -m 10 "$REF/status" 2>/dev/null) || fail reference_status_unavailable
ref_chain=$(jq -r '.result.node_info.network // empty' <<<"$ref_status" 2>/dev/null)
[[ -n $ref_chain ]] || fail missing_reference_chain_id
[[ $chain == "$ref_chain" ]] || fail reference_chain_mismatch

# The next header may not exist yet. Preserve an unavailable result if it
# does not arrive, rather than treating it as a mismatch.
for _ in $(seq 1 10); do
  block=$(curl -fsS -m 10 "$REF/block?height=$((height + 1))" 2>/dev/null) || block=
  reference_hash=$(jq -r '.result.block.header.app_hash // empty' <<<"$block" 2>/dev/null)
  reference_height=$(jq -r '.result.block.header.height // empty' <<<"$block" 2>/dev/null)
  block_chain=$(jq -r '.result.block.header.chain_id // empty' <<<"$block" 2>/dev/null)
  if [[ -n $reference_hash ]]; then
    [[ $reference_height =~ ^[0-9]+$ ]] || { reference_height=; fail invalid_reference_height; }
    [[ $reference_height == $((height + 1)) ]] || fail reference_header_height_mismatch
    [[ $block_chain == "$chain" ]] || fail reference_header_chain_mismatch
    break
  fi
  sleep 3
done
[[ -n $reference_hash ]] || fail reference_header_unavailable
reference_hash=$(tr 'a-f' 'A-F' <<<"$reference_hash" | tr -d '\n')
[[ $reference_hash =~ ^[0-9A-F]{64}$ ]] || { reference_hash=; fail invalid_reference_hash; }

if [[ $local_hash == "$reference_hash" ]]; then
  outcome=match; receipt; exit 0
fi
outcome=mismatch; receipt; exit 1
