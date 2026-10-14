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

# gen <file> <tx args...>: an unsigned tx (for authz exec / raw messages).
gen() {
  local f=$1
  shift
  celestia-appd tx "$@" "${K[@]}" --chain-id "$CHAIN_ID" --node "$NODE" \
    --fees 20000utia --gas 400000 --generate-only -o json >"$f"
}

# rawtx <from> <messages-json>: sign and broadcast a tx with custom messages
# (for messages the CLI cannot build).
rawtx() {
  local from=$1 msgs=$2 a out hash res r=$HOME_DIR/rawtx
  mkdir -p "$r"
  a=$(addr "$from")
  gen "$r/raw.json" bank send "$a" "$a" 1utia --from "$from"
  python3 - "$r/raw.json" "$msgs" <<'PY'
import json, sys
f, msgs = sys.argv[1], json.loads(sys.argv[2])
t = json.load(open(f))
t["body"]["messages"] = msgs
json.dump(t, open(f, "w"))
PY
  celestia-appd tx sign "$r/raw.json" --from "$from" "${K[@]}" --chain-id "$CHAIN_ID" \
    --node "$NODE" -o json >"$r/raw.signed.json"
  out=$(celestia-appd tx broadcast "$r/raw.signed.json" --node "$NODE" -o json)
  if [ "$(echo "$out" | json 'd.get("code", 0)')" != "0" ]; then
    echo "checktx failed: $out" >&2
    return 1
  fi
  hash=$(echo "$out" | json 'd["txhash"]')
  res=$(python3 "$HERE/wait_tx.py" "$RPC" "$hash")
  echo "rawtx $msgs -> $res"
}

height() { curl -s "$RPC/status" | json 'd["result"]["sync_info"]["latest_block_height"]'; }

# wait_blocks <n>
wait_blocks() {
  local target=$(($(height) + $1))
  until [ "$(height)" -ge "$target" ]; do sleep 0.5; done
}
