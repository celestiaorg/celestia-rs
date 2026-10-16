#!/usr/bin/env bash
# Rust CLI (`celestia-rs tx …`) against a Go node, checked against the Go
# CLI (`celestia-appd tx …`) for devnet.sh.
#
# For every command, both CLIs build the same tx with --generate-only and
# the unsigned tx JSON must be identical (fees from --fees or --gas-prices,
# --gas auto via Simulate with --gas-adjustment, the estimate printed on
# stderr); the Rust CLI then signs and broadcasts it and the tx must be
# committed with code 0. Covers bank send, staking create-validator /
# edit-validator / delegate / unbond / redelegate, and the generic
# `tx <module> <msg> --json` escape hatch (bank multi-send, distribution
# withdraw-delegator-reward, staking cancel-unbonding-delegation). Then
# --generate-only with a bech32 --from on a passphrase-encrypted `file`
# keyring without a TTY or passphrase (create-validator, bank send,
# slashing unjail, signal, the generic --json), and the --gas auto error
# with an address --from.
#
# Env (from devnet.sh): HOME_DIR, CHAIN_ID, NODE; celestia-appd on PATH;
# RS_CLI: the celestia-rs binary (default target/debug/celestia-rs).
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
RS=${RS_CLI:-$ROOT/target/debug/celestia-rs}
RPC=${NODE/tcp:/http:}
K=(--keyring-backend test --home "$HOME_DIR")
C=(--chain-id "$CHAIN_ID" --node "$NODE")
VAL0=$(celestia-appd keys show val0 -a "${K[@]}")
VAL0_OPER=$(celestia-appd keys show val0 --bech val -a "${K[@]}")
USER0=$(celestia-appd keys show user0 -a "${K[@]}")
USER0_OPER=$(celestia-appd keys show user0 --bech val -a "${K[@]}")
W=$HOME_DIR/cli-workload
mkdir -p "$W"

json() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }
CHECKED=0

# same <name> <go-args…> -- <rust-args…>: both CLIs' --generate-only
# output (and gas estimate) must match.
same() {
  local name=$1
  shift
  local go=() rs=()
  while [ "$1" != "--" ]; do go+=("$1"); shift; done
  shift
  rs=("$@")
  celestia-appd tx "${go[@]}" "${K[@]}" "${C[@]}" --generate-only -o json \
    >"$W/$name.go.json" 2>"$W/$name.go.err"
  "$RS" tx "${rs[@]}" "${K[@]}" "${C[@]}" --generate-only \
    >"$W/$name.rs.json" 2>"$W/$name.rs.err"
  python3 - "$W/$name.go.json" "$W/$name.rs.json" <<'PY'
import json, sys
a, b = (json.loads(open(p).read().strip().splitlines()[-1]) for p in sys.argv[1:])
if a != b:
    print("MISMATCH\ngo:   " + json.dumps(a, sort_keys=True) + "\nrust: " + json.dumps(b, sort_keys=True))
    sys.exit(1)
PY
  local ge re
  ge=$(grep -o 'gas estimate: [0-9]*' "$W/$name.go.err" || true)
  re=$(grep -o 'gas estimate: [0-9]*' "$W/$name.rs.err" || true)
  if [ "$ge" != "$re" ]; then
    echo "$name: gas estimate go=[$ge] rust=[$re]" >&2
    exit 1
  fi
  CHECKED=$((CHECKED + 1))
  echo "same: $name ${ge:+($ge)}"
}

# rs_tx <args…>: Rust CLI broadcast with --wait (tx_status); the tx must
# commit with execution code 0.
rs_tx() {
  local out
  out=$("$RS" tx "$@" "${K[@]}" "${C[@]}" --wait) || {
    echo "rust tx failed: $out" >&2
    return 1
  }
  if [ "$(echo "$out" | json 'd.get("code", 0)')" != "0" ]; then
    echo "rust tx failed: $out" >&2
    return 1
  fi
  echo "rust: $* -> height $(echo "$out" | json 'd["height"]') gas $(echo "$out" | json 'd["gas_used"]')"
}

# 1. bank send: --fees, --gas-prices, --gas auto with adjustment.
same send-fees bank send val0 "$USER0" 1000utia --fees 3000utia \
  -- bank send val0 "$USER0" 1000utia --fees 3000utia
rs_tx bank send val0 "$USER0" 1000utia --fees 3000utia
same send-prices bank send val0 "$USER0" 777utia --gas 123457 --gas-prices 0.0025utia --note "rust cli" \
  -- bank send val0 "$USER0" 777utia --gas 123457 --gas-prices 0.0025utia --note "rust cli"
rs_tx bank send val0 "$USER0" 777utia --gas 123457 --gas-prices 0.0045utia --note "rust cli"
same send-auto bank send val0 "$USER0" 5utia --from val0 --gas auto --gas-adjustment 1.37 --gas-prices 0.004utia \
  -- bank send val0 "$USER0" 5utia --gas auto --gas-adjustment 1.37 --gas-prices 0.004utia
rs_tx bank send val0 "$USER0" 5utia --gas auto --gas-adjustment 1.37 --gas-prices 0.004utia
same send-auto-fees bank send val0 "$USER0" 6utia --from val0 --gas auto --fees 2000utia \
  -- bank send val0 "$USER0" 6utia --gas auto --fees 2000utia

# 2. staking: create-validator (a fresh consensus key), edit-validator,
# delegate, unbond, redelegate.
TMPV=$W/valhome
celestia-appd init tmpval --chain-id "$CHAIN_ID" --home "$TMPV" >/dev/null 2>&1
PK=$(celestia-appd comet show-validator --home "$TMPV")
cat >"$W/validator.json" <<JSON
{
  "pubkey": $PK,
  "amount": "1000000utia",
  "moniker": "rust-cli-validator",
  "identity": "ID123",
  "website": "https://example.org",
  "security": "sec@example.org",
  "details": "created by celestia-rs",
  "commission-rate": "0.1",
  "commission-max-rate": "0.2",
  "commission-max-change-rate": "0.01",
  "min-self-delegation": "1"
}
JSON
same create-validator staking create-validator "$W/validator.json" --from user0 --fees 30000utia --gas 300000 \
  -- staking create-validator "$W/validator.json" --from user0 --fees 30000utia --gas 300000
rs_tx staking create-validator "$W/validator.json" --from user0 --fees 30000utia --gas auto --gas-adjustment 1.5
sleep 2
same edit-validator staking edit-validator --from user0 --details "edited by celestia-rs" --website "https://celestia.org" --fees 20000utia \
  -- staking edit-validator --from user0 --details "edited by celestia-rs" --website "https://celestia.org" --fees 20000utia
same edit-validator-rate staking edit-validator --from user0 --new-moniker renamed --min-self-delegation 2 --fees 20000utia \
  -- staking edit-validator --from user0 --new-moniker renamed --min-self-delegation 2 --fees 20000utia
rs_tx staking edit-validator --from user0 --details "edited by celestia-rs" --website "https://celestia.org" --fees 20000utia
same delegate staking delegate "$USER0_OPER" 5000utia --from val0 --gas auto --gas-adjustment 1.2 --gas-prices 0.004utia \
  -- staking delegate "$USER0_OPER" 5000utia --from val0 --gas auto --gas-adjustment 1.2 --gas-prices 0.004utia
rs_tx staking delegate "$USER0_OPER" 5000utia --from val0 --gas auto --gas-adjustment 1.2 --gas-prices 0.004utia
same unbond staking unbond "$USER0_OPER" 1000utia --from val0 --fees 20000utia --gas 300000 \
  -- staking unbond "$USER0_OPER" 1000utia --from val0 --fees 20000utia --gas 300000
rs_tx staking unbond "$USER0_OPER" 1000utia --from val0 --fees 20000utia --gas 300000
same redelegate staking redelegate "$VAL0_OPER" "$USER0_OPER" 2000utia --from val0 --fees 30000utia --gas 400000 \
  -- staking redelegate "$VAL0_OPER" "$USER0_OPER" 2000utia --from val0 --fees 30000utia --gas 400000
rs_tx staking redelegate "$VAL0_OPER" "$USER0_OPER" 2000utia --from val0 --fees 30000utia --gas 400000

# 3. The generic escape hatch vs the Go CLI's dedicated commands.
same multi-send bank multi-send val0 "$USER0" "$VAL0" 10utia --fees 20000utia \
  -- bank multi-send --from val0 --fees 20000utia --json "{\"inputs\":[{\"address\":\"$VAL0\",\"coins\":[{\"denom\":\"utia\",\"amount\":\"20\"}]}],\"outputs\":[{\"address\":\"$USER0\",\"coins\":[{\"denom\":\"utia\",\"amount\":\"10\"}]},{\"address\":\"$VAL0\",\"coins\":[{\"denom\":\"utia\",\"amount\":\"10\"}]}]}"
rs_tx bank multi-send --from val0 --fees 20000utia --json "{\"inputs\":[{\"address\":\"$VAL0\",\"coins\":[{\"denom\":\"utia\",\"amount\":\"20\"}]}],\"outputs\":[{\"address\":\"$USER0\",\"coins\":[{\"denom\":\"utia\",\"amount\":\"10\"}]},{\"address\":\"$VAL0\",\"coins\":[{\"denom\":\"utia\",\"amount\":\"10\"}]}]}"
same withdraw distribution withdraw-rewards "$VAL0_OPER" --from val0 --fees 20000utia \
  -- distribution withdraw-delegator-reward --from val0 --fees 20000utia --json "{\"delegator_address\":\"$VAL0\",\"validator_address\":\"$VAL0_OPER\"}"
rs_tx distribution withdraw-delegator-reward --from val0 --gas auto --gas-adjustment 1.3 --fees 20000utia \
  --json "{\"delegator_address\":\"$VAL0\",\"validator_address\":\"$VAL0_OPER\"}"
H=$(curl -s "$RPC/status" | json 'd["result"]["sync_info"]["latest_block_height"]')
UH=$(curl -s "$RPC/abci_query?path=%22/cosmos.staking.v1beta1.Query/DelegatorUnbondingDelegations%22&data=0x$(python3 - "$VAL0" <<'PY'
import sys
a = sys.argv[1].encode()
print((b"\x0a" + bytes([len(a)]) + a).hex())
PY
)" | json 'd["result"]["response"]["value"]')
CH=$(python3 - "$UH" <<'PY'
import base64, sys
b = base64.b64decode(sys.argv[1])
# QueryDelegatorUnbondingDelegationsResponse.unbonding_responses[0].entries[0].creation_height:
# a varint after field 1 (creation_height) of the first entry; parse minimally.
def varint(b, i):
    x = s = 0
    while True:
        c = b[i]; i += 1; x |= (c & 0x7f) << s; s += 7
        if c < 0x80: return x, i
def fields(b):
    i = 0
    while i < len(b):
        k, i = varint(b, i)
        f, t = k >> 3, k & 7
        if t == 0:
            v, i = varint(b, i)
        elif t == 2:
            n, i = varint(b, i); v = b[i:i+n]; i += n
        else:
            raise SystemExit("unexpected wire type")
        yield f, v
ubd = next(v for f, v in fields(b) if f == 1)
entry = next(v for f, v in fields(ubd) if f == 3)
print(next(v for f, v in fields(entry) if f == 1))
PY
)
same cancel-unbond staking cancel-unbond "$USER0_OPER" 500utia "$CH" --from val0 --fees 20000utia \
  -- staking cancel-unbonding-delegation --from val0 --fees 20000utia \
  --json "{\"delegator_address\":\"$VAL0\",\"validator_address\":\"$USER0_OPER\",\"amount\":{\"denom\":\"utia\",\"amount\":\"500\"},\"creation_height\":\"$CH\"}"
rs_tx staking cancel-unbonding-delegation --from val0 --fees 20000utia \
  --json "{\"delegator_address\":\"$VAL0\",\"validator_address\":\"$USER0_OPER\",\"amount\":{\"denom\":\"utia\",\"amount\":\"500\"},\"creation_height\":\"$CH\"}"

# 4. --generate-only with a bech32 --from (Go's GetFromFields): the address
# is used as-is and the keyring is never opened, so a passphrase-encrypted
# `file` keyring needs no passphrase and no TTY (stdin is /dev/null and
# CELESTIA_RS_KEYRING_PASSPHRASE is unset). A key name still asks the
# passphrase in both CLIs.
FK=$W/filekr
PASS=filekr-passphrase
printf '%s\n%s\n' "$PASS" "$PASS" | celestia-appd keys add op --keyring-backend file --home "$FK" >/dev/null 2>&1
OP=$(printf '%s\n' "$PASS" | celestia-appd keys show op -a --keyring-backend file --home "$FK" 2>/dev/null | tail -1)
OP_OPER=$(printf '%s\n' "$PASS" | celestia-appd keys show op -a --bech val --keyring-backend file --home "$FK" 2>/dev/null | tail -1)
FKF=(--keyring-backend file --home "$FK" --chain-id "$CHAIN_ID" --node "$NODE" --generate-only)
# same_addr <name> <go-args…> -- <rust-args…>: like `same`, on the file
# keyring, without a passphrase.
same_addr() {
  local name=$1
  shift
  local go=() rs=()
  while [ "$1" != "--" ]; do go+=("$1"); shift; done
  shift
  rs=("$@")
  celestia-appd tx "${go[@]}" "${FKF[@]}" -o json </dev/null >"$W/$name.go.json" 2>"$W/$name.go.err" || {
    echo "$name: go failed: $(cat "$W/$name.go.err")" >&2
    exit 1
  }
  env -u CELESTIA_RS_KEYRING_PASSPHRASE "$RS" tx "${rs[@]}" "${FKF[@]}" </dev/null \
    >"$W/$name.rs.json" 2>"$W/$name.rs.err" || {
    echo "$name: rust failed: $(cat "$W/$name.rs.err")" >&2
    exit 1
  }
  python3 - "$W/$name.go.json" "$W/$name.rs.json" <<'PY'
import json, sys
a, b = (json.loads(open(p).read().strip().splitlines()[-1]) for p in sys.argv[1:])
if a != b:
    print("MISMATCH\ngo:   " + json.dumps(a, sort_keys=True) + "\nrust: " + json.dumps(b, sort_keys=True))
    sys.exit(1)
PY
  CHECKED=$((CHECKED + 1))
  echo "same (address --from, file keyring, no passphrase): $name"
}
same_addr addr-create-validator staking create-validator "$W/validator.json" --from "$OP" --fees 30000utia --gas 300000 \
  -- staking create-validator "$W/validator.json" --from "$OP" --fees 30000utia --gas 300000
same_addr addr-send bank send "$OP" "$USER0" 1000utia --fees 3000utia --note "address from" \
  -- bank send "$OP" "$USER0" 1000utia --fees 3000utia --note "address from"
same_addr addr-unjail slashing unjail --from "$OP" --fees 20000utia \
  -- slashing unjail --from "$OP" --fees 20000utia
same_addr addr-unjail-json slashing unjail --from "$OP" --fees 20000utia \
  -- slashing unjail --from "$OP" --fees 20000utia --json "{\"validator_addr\":\"$OP_OPER\"}"
same_addr addr-signal signal signal 11 --from "$OP" --fees 20000utia \
  -- signal signal 11 --from "$OP" --fees 20000utia
same_addr addr-try-upgrade signal try-upgrade --from "$OP" --gas-prices 0.004utia \
  -- signal try-upgrade --from "$OP" --gas-prices 0.004utia
same_addr addr-withdraw distribution withdraw-rewards "$VAL0_OPER" --from "$OP" --fees 20000utia \
  -- distribution withdraw-delegator-reward --from "$OP" --fees 20000utia \
  --json "{\"delegator_address\":\"$OP\",\"validator_address\":\"$VAL0_OPER\"}"
# A key name opens the keyring: both CLIs fail without the passphrase.
if celestia-appd tx bank send op "$USER0" 1utia --fees 3000utia "${FKF[@]}" </dev/null >/dev/null 2>&1 \
  || env -u CELESTIA_RS_KEYRING_PASSPHRASE "$RS" tx bank send op "$USER0" 1utia --fees 3000utia "${FKF[@]}" </dev/null >/dev/null 2>&1; then
  echo "generate-only with a key name must ask the file keyring passphrase" >&2
  exit 1
fi
# --gas auto with an address --from: the simulation needs the key's public
# key, which Go looks up by the (empty) key name after the account query;
# both CLIs fail with the same error (on a funded account).
for cli in go rs; do
  if [ $cli = go ]; then
    celestia-appd tx bank send "$USER0" "$VAL0" 1utia --gas auto --gas-prices 0.004utia "${K[@]}" "${C[@]}" \
      --generate-only </dev/null >/dev/null 2>"$W/addr-gas-auto.$cli.err" && exit 1
  else
    "$RS" tx bank send "$USER0" "$VAL0" 1utia --gas auto --gas-prices 0.004utia "${K[@]}" "${C[@]}" \
      --generate-only </dev/null >/dev/null 2>"$W/addr-gas-auto.$cli.err" && exit 1
  fi
  grep -q '^Error: .info: key not found' "$W/addr-gas-auto.$cli.err" || {
    echo "addr-gas-auto ($cli): $(cat "$W/addr-gas-auto.$cli.err")" >&2
    exit 1
  }
done
echo "same: --gas auto --generate-only with an address --from fails in both CLIs (.info: key not found)"

echo "cli workload: $CHECKED Go/Rust CLI pairs identical, all Rust txs committed (code 0) at height >= $H"
