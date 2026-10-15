#!/usr/bin/env bash
# x/zkism + x/forwarding workload for devnet.sh / mixed.sh / rust-proposer.sh.
#
# zkism: the SP1 Groth16 vectors celestia-app's own keeper tests use
# (x/zkism/internal/testdata, copied to crates/tier-3-app-modules/x/zkism/
# testdata) are valid on any chain, so the success paths run for real:
# create an ISM with the transition's trusted state, update it with the
# state-transition proof, submit the membership proof. Failures: tampered
# proofs (point encoding, public input), replayed transition, resubmitted membership proof, and a
# hyperlane message processed through a mailbox whose default ISM is the
# zkism (not authorized -> rejected).
#
# forwarding: a collateral (utia) warp token with a remote router and an
# IGP default hook (positive fee); utia sent to derived forwarding
# addresses is forwarded by a relayer (IGP fee collected, excess refunded),
# with a custom hook bound into the address, and failures: empty address,
# fee below the quote, wrong address, no warp route.
#
# Ids are deterministic (router sequences start at 0):
#   noop ISM   router_ism type 0 seq 0, zk ISM router_ism type 42 seq 1
#   noop hook  router_post_dispatch type 0 seq 0, merkle type 3 seq 1, IGP type 4 seq 2
#   mailboxes  hyperlane type 0 seq 0 (noop ISM), seq 1 (zk ISM)
#   tokens     router_app type 1 seq 0 (mailbox 0), seq 1 (mailbox 1)
set -euo pipefail
. "$(dirname "$0")/lib.sh"

ROOT=$(cd "$HERE/../../.." && pwd)
TD=$ROOT/crates/tier-3-app-modules/x/zkism/testdata
A="celestia-appd --home $HOME_DIR"
TXF="--keyring-backend test --chain-id $CHAIN_ID --node $NODE --gas 600000 --fees 6000utia --yes --output json"

id() { # <name> <type> <seq>
  python3 -c 'import sys; n=sys.argv[1].encode().ljust(20, b"\0"); print("0x" + (n + int(sys.argv[2]).to_bytes(4, "big") + int(sys.argv[3]).to_bytes(8, "big")).hex())' "$@"
}
NOOP_ISM=$(id router_ism 0 0)
ZK_ISM=$(id router_ism 42 1)
NOOP_HOOK=$(id router_post_dispatch 0 0)
MERKLE=$(id router_post_dispatch 3 1)
IGP=$(id router_post_dispatch 4 2)
MAILBOX=$(id hyperlane 0 0)
MAILBOX2=$(id hyperlane 0 1)
TOKEN=$(id router_app 1 0)
TOKEN2=$(id router_app 1 1)
LOCAL_DOMAIN=69420
REMOTE_DOMAIN=1
REMOTE_ROUTER=0x000000000000000000000000ee00000000000000000000000000000000000001
RECIPIENT=0x0000000000000000000000000000000000000000000000000000000000000abc

run() { # <from> <tx args...>: CheckTx code, then wait for inclusion
  local from=$1 out code hash
  shift
  out=$($A tx "$@" --from "$from" $TXF 2>&1) || true
  code=$(echo "$out" | json 'd.get("code", 0)' 2>/dev/null || echo "cli-error: ${out:0:160}")
  echo "tx $1 $2: checktx code=$code"
  [ "$code" = 0 ] || return 0
  hash=$(echo "$out" | json 'd["txhash"]')
  python3 "$HERE/wait_tx.py" "$RPC" "$hash" >/dev/null || echo "tx $hash not included"
}

hexfile() { python3 -c 'import sys; print(open(sys.argv[1], "rb").read().hex())' "$1"; }
ST_PROOF=$(hexfile "$TD/state_transition/proof.bin")
ST_VALUES=$(hexfile "$TD/state_transition/public_values.bin")
SM_PROOF=$(hexfile "$TD/state_membership/proof.bin")
SM_VALUES=$(hexfile "$TD/state_membership/public_values.bin")
# Trusted state (first length-prefixed field of the transition values) and
# the merkle tree address the membership proof commits to.
STATE=$(python3 -c 'import sys; b=bytes.fromhex(sys.argv[1]); n=int.from_bytes(b[:8], "little"); print(b[8:8+n].hex())' "$ST_VALUES")
MERKLE_ADDR=${SM_VALUES:64:64}
ST_VKEY=004ac29c473e811dece0f8dd76c8eda80f886d263efb393ec81f54173e54f160
SM_VKEY=00982fb21526d096c8bf58eda36b5e293ee9ea0f36df441f6a996a974f8feb63
# A corrupted proof point (decoding fails) and a corrupted proof nonce (a
# public input: the pairing check fails).
BAD_POINT=${ST_PROOF:0:400}$(printf '%02x' $(( 0x${ST_PROOF:400:2} ^ 1 )))${ST_PROOF:402}
BAD_NONCE=${ST_PROOF:0:190}$(printf '%02x' $(( 0x${ST_PROOF:190:2} ^ 1 )))${ST_PROOF:192}
if [ -n "${APP_V9:-}" ]; then
  # celestia-app v9.0.8 (SP1 v5): its own vectors (testdata/v9, program
  # vkeys from its keeper tests); proofs are prefix + 256 bytes, so the
  # second failure flips a bit of Krs (the pairing check fails).
  TD=$TD/v9
  ST_PROOF=$(hexfile "$TD/state_transition/proof.bin")
  ST_VALUES=$(hexfile "$TD/state_transition/public_values.bin")
  SM_PROOF=$(hexfile "$TD/state_membership/proof.bin")
  SM_VALUES=$(hexfile "$TD/state_membership/public_values.bin")
  STATE=$(python3 -c 'import sys; b=bytes.fromhex(sys.argv[1]); n=int.from_bytes(b[:8], "little"); print(b[8:8+n].hex())' "$ST_VALUES")
  MERKLE_ADDR=${SM_VALUES:64:64}
  ST_VKEY=0017bc91d53b93c46eb842d7f9020a94ea13d8877a21608b34b71fcc4da64f29
  SM_VKEY=004959d5fb2c3d5bc1f98e032188dd94fbb5c6b6152df356c7c20be23be824a2
  BAD_POINT=${ST_PROOF:0:8}$(printf '%02x' $(( 0x${ST_PROOF:8:2} ^ 1 )))${ST_PROOF:10}
  BAD_NONCE=${ST_PROOF:0:500}$(printf '%02x' $(( 0x${ST_PROOF:500:2} ^ 1 )))${ST_PROOF:502}
fi

# ---- hyperlane infrastructure ----
run user0 hyperlane ism create-noop
run user0 zkism create "$STATE" "$MERKLE_ADDR" "$TD/groth16_vk.bin" "$ST_VKEY" "$SM_VKEY"
run user0 hyperlane hooks noop create
run user0 hyperlane mailbox create "$NOOP_ISM" $LOCAL_DOMAIN
run user0 hyperlane hooks merkle create "$MAILBOX"
run user0 hyperlane hooks igp create utia
run user0 hyperlane hooks igp set-destination-gas-config "$IGP" $REMOTE_DOMAIN 10000000000 1 100
run user0 hyperlane mailbox set "$MAILBOX" --required-hook "$MERKLE" --default-hook "$IGP"
run user0 hyperlane mailbox create "$ZK_ISM" $LOCAL_DOMAIN
run user0 warp create-collateral-token "$MAILBOX" utia
run user0 warp enroll-remote-router "$TOKEN" $REMOTE_DOMAIN "$REMOTE_ROUTER" 50000
run user0 warp create-collateral-token "$MAILBOX2" utia
run user0 warp enroll-remote-router "$TOKEN2" $REMOTE_DOMAIN "$REMOTE_ROUTER" 50000

# ---- zkism ----
run user0 zkism update "$ZK_ISM" "$BAD_POINT" "$ST_VALUES"
run user0 zkism update "$ZK_ISM" "$BAD_NONCE" "$ST_VALUES"
run user0 zkism submit-messages "$ZK_ISM" "$SM_PROOF" "$SM_VALUES"
run user0 zkism update "$ZK_ISM" "$ST_PROOF" "$ST_VALUES"
run user0 zkism update "$ZK_ISM" "$ST_PROOF" "$ST_VALUES"
run val0 zkism submit-messages "$ZK_ISM" "$SM_PROOF" "$SM_VALUES"
run val0 zkism submit-messages "$ZK_ISM" "$SM_PROOF" "$SM_VALUES"
# A message for the zk-ISM mailbox's token: the zkism ISM does not
# authorize it.
USER_HEX=$($A keys show user0 -a --keyring-backend test | xargs $A debug addr | sed -n 's/^Address (hex): //p' | tr 'A-F' 'a-f')
BODY=$(printf '%024x%s%064x' 0 "$USER_HEX" 1000)
MSG=0x03$(printf '%08x%08x' 0 $REMOTE_DOMAIN)${REMOTE_ROUTER#0x}$(printf '%08x' $LOCAL_DOMAIN)${TOKEN2#0x}$BODY
run user0 hyperlane mailbox process "$MAILBOX2" 0x "$MSG"

# ---- forwarding ----
# fwd <domain> <recipient> [hook id]: derived forwarding address (bech32).
fwd() {
  python3 - "$@" <<'PY'
import hashlib, sys
def bech32(hrp, data):
    CH = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
    def polymod(v):
        g = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3]
        c = 1
        for x in v:
            b = c >> 25
            c = (c & 0x1ffffff) << 5 ^ x
            for i in range(5):
                c ^= g[i] if (b >> i) & 1 else 0
        return c
    acc, bits, out = 0, 0, []
    for b in data:
        acc = (acc << 8) | b
        bits += 8
        while bits >= 5:
            bits -= 5
            out.append((acc >> bits) & 31)
    if bits:
        out.append((acc << (5 - bits)) & 31)
    hv = [ord(x) >> 5 for x in hrp] + [0] + [ord(x) & 31 for x in hrp]
    pm = polymod(hv + out + [0] * 6) ^ 1
    return hrp + "1" + "".join(CH[i] for i in out + [(pm >> 5 * (5 - i)) & 31 for i in range(6)])
domain, recipient, token = int(sys.argv[1]), bytes.fromhex(sys.argv[2][2:]), bytes.fromhex(sys.argv[3][2:])
hook = bytes.fromhex(sys.argv[4][2:]) if len(sys.argv) > 4 else b""
h = hashlib.sha256(domain.to_bytes(32, "big") + recipient + token)
if hook and any(hook):
    h.update(hook)
call = h.digest()
salt = hashlib.sha256(b"\x01" + call).digest()
addr = hashlib.sha256(hashlib.sha256(b"module").digest() + b"forwarding\x00" + salt).digest()[:20]
print(bech32("celestia", addr))
PY
}
F1=$(fwd $REMOTE_DOMAIN $RECIPIENT "$TOKEN")
F2=$(fwd 2 $RECIPIENT "$TOKEN")
F3=$(fwd $REMOTE_DOMAIN $RECIPIENT "$TOKEN" "$NOOP_HOOK")
echo "forwarding addresses: $F1 $F2 $F3"

run user0 bank multi-send "$(addr user0)" "$F1" "$F2" "$F3" 5000000utia
# Forwarded (IGP fee 50100utia paid by the relayer, excess refunded).
run val0 forwarding forward "$F1" "$TOKEN" $REMOTE_DOMAIN $RECIPIENT --max-igp-fee 1000000utia
# Nothing left to forward.
run val0 forwarding forward "$F1" "$TOKEN" $REMOTE_DOMAIN $RECIPIENT --max-igp-fee 1000000utia
run user0 bank send "$(addr user0)" "$F1" 300000utia
# Fee below the quote; address not derived from these parameters; no route.
run val0 forwarding forward "$F1" "$TOKEN" $REMOTE_DOMAIN $RECIPIENT --max-igp-fee 10utia
run val0 forwarding forward "$F1" "$TOKEN" $REMOTE_DOMAIN 0x0000000000000000000000000000000000000000000000000000000000000abd --max-igp-fee 1000000utia
run val0 forwarding forward "$F2" "$TOKEN" 2 $RECIPIENT --max-igp-fee 1000000utia
# Bound to a custom (noop, zero-fee) hook.
run val0 forwarding forward "$F3" "$TOKEN" $REMOTE_DOMAIN $RECIPIENT --max-igp-fee 1utia --custom-hook-id "$NOOP_HOOK"
run user0 forwarding forward "$F1" "$TOKEN" $REMOTE_DOMAIN $RECIPIENT --max-igp-fee 60000utia
q bank balances "$F1" || true
