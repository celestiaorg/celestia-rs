#!/usr/bin/env bash
# Compares the app hash this node computed with the one the network
# committed (docs/operator-guide.md §11.7).
#
#   packaging/check-apphash.sh [local-rpc] [reference-rpc]
#
# The local node's `/abci_info` reports the app hash after executing its
# latest height H; the network's header H+1 carries the app hash every
# validator agreed on for H. They must be equal. Exit 0 when they match,
# 1 on a mismatch, 2 when the check could not run. Needs curl and jq.
set -euo pipefail

LOCAL=${1:-http://127.0.0.1:26657}
REF=${2:-https://rpc.celestia-mocha.com}

info=$(curl -fsS "$LOCAL/abci_info") || { echo "cannot reach $LOCAL" >&2; exit 2; }
h=$(jq -r '.result.response.last_block_height' <<<"$info")
ours=$(jq -r '.result.response.last_block_app_hash' <<<"$info" | base64 -d | od -An -v -tx1 | tr -d ' \n' | tr 'a-f' 'A-F')

# Header H+1 may not exist yet on the reference: retry for ~30 s.
net=""
for _ in $(seq 1 10); do
  net=$(curl -fsS "$REF/block?height=$((h + 1))" 2>/dev/null | jq -r '.result.block.header.app_hash // empty') || true
  [[ -n "$net" ]] && break
  sleep 3
done
[[ -n "$net" ]] || { echo "reference $REF has no header $((h + 1))" >&2; exit 2; }

if [[ "$ours" == "$net" ]]; then
  echo "OK height=$h app_hash=$ours"
else
  echo "MISMATCH height=$h ours=$ours network=$net" >&2
  exit 1
fi
