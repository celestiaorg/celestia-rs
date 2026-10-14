# Shared helpers for devnet workloads (sourced).
#
# Env (from devnet.sh): HOME_DIR, CHAIN_ID, NODE; celestia-appd on PATH.

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
RPC=${NODE/tcp:/http:}
K=(--keyring-backend test --home "$HOME_DIR")

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

addr() { celestia-appd keys show "$1" -a "${K[@]}"; }
valoper() { celestia-appd keys show "$1" --bech val -a "${K[@]}"; }

# tx <from> <args...>: broadcast and wait for inclusion. Fails only if the tx
# is rejected by CheckTx; failing DeliverTx results are expected by some
# workloads and only logged.
tx() {
  local from=$1
  shift
  local out hash res
  out=$(celestia-appd tx "$@" --from "$from" "${K[@]}" --chain-id "$CHAIN_ID" \
    --node "$NODE" --fees ${FEES:-20000utia} --gas ${GAS:-400000} -y -o json)
  if [ "$(echo "$out" | json 'd.get("code", 0)')" != "0" ]; then
    echo "checktx failed: $out" >&2
    return 1
  fi
  hash=$(echo "$out" | json 'd["txhash"]')
  res=$(python3 "$HERE/wait_tx.py" "$RPC" "$hash") || {
    echo "tx $hash not included" >&2
    return 1
  }
  echo "$* -> $res"
}

q() { celestia-appd query "$@" --node "$NODE" -o json; }

height() { curl -s "$RPC/status" | json 'd["result"]["sync_info"]["latest_block_height"]'; }

# wait_blocks <n>
wait_blocks() {
  local target=$(($(height) + $1))
  until [ "$(height)" -ge "$target" ]; do sleep 0.5; done
}
