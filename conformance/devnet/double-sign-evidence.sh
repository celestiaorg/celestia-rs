#!/usr/bin/env bash
# Live double-sign evidence on a mixed network: 2 Go (celestia-appd) and 2
# Rust (celestia-rs start) validators. A byzantine peer
# (`cargo run -p celestia-rs --example double_sign`) signs two conflicting
# prevotes with Go validator V1's key and sends them only to Rust
# validator V2. The Rust consensus must detect the duplicate vote, its
# evidence pool must turn it into verified `DuplicateVoteEvidence` and its
# evidence reactor must gossip it, so that the Go nodes verify it too and
# the network commits it in a block; the double signer is then slashed and
# removed from the validator set by both state machines, and every node
# keeps agreeing on block and app hashes.
#
# usage: [DEVNET_PORT_OFFSET=n] [DEVNET_BIN=dir] conformance/devnet/double-sign-evidence.sh
# Set EVIDENCE_FIXTURE_OUT to an empty directory to capture the authentic
# evidence block and its pre-height validation history before cleanup.
# DEVNET_NO_DEFAULT_FEATURES=1 builds the Rust node with redb only.
# Exit 0 iff all checks pass.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
# Match build-bins.sh's Go pin for fixture recorder builds as well.
export GOTOOLCHAIN=${GOTOOLCHAIN:-$(awk '$1 == "toolchain" { print $2; exit }' "$ROOT/conformance/devnet-oracle/go.mod")}
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=$((${DEVNET_PORT_OFFSET:-15000} + 200))
RS=$ROOT/target/debug/celestia-rs
BYZ=$ROOT/target/debug/examples/double_sign
W=$(mktemp -d)
FAIL=()

if [ -n "${EVIDENCE_FIXTURE_OUT:-}" ]; then
  mkdir -p "$EVIDENCE_FIXTURE_OUT"
  [ -z "$(ls -A "$EVIDENCE_FIXTURE_OUT")" ] || { echo "fixture directory must be empty" >&2; exit 1; }
  EVIDENCE_FIXTURE_OUT=$(cd "$EVIDENCE_FIXTURE_OUT" && pwd)
  (cd "$ROOT/conformance/rpccorpus/recorder" && go build -o "$BIN/rpc-recorder" .)
fi

CARGO_FEATURES=()
[ "${DEVNET_NO_DEFAULT_FEATURES:-0}" != 1 ] || CARGO_FEATURES=(--no-default-features)
(cd "$ROOT" && cargo build -q "${CARGO_FEATURES[@]}" -p celestia-rs && cargo build -q "${CARGO_FEATURES[@]}" -p celestia-rs --example double_sign)

# shellcheck source=lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"

CHAIN_ID=rs-double-sign-1
D=$W/d
mkdir -p "$D"
make_net "$D" "$CHAIN_ID" "go go rs rs" "5000000000 5000000000 5000000000 5000000000"
if [ -n "${EVIDENCE_FIXTURE_OUT:-}" ]; then
  # Recover pre-evidence app/results hashes and timeouts from the original
  # FinalizeBlock response, which the normal Go config discards.
  sed -i.bak 's/^discard_abci_responses = true/discard_abci_responses = false/' "$D/n0/config/config.toml"
  rm -f "$D/n0/config/config.toml.bak"
fi
for i in 0 1 2 3; do start_node "$D" $i; done
ensure_up "$D" 0 1 2 3
wait_height "$D" 0 5 240 || FAIL+=("network did not start")

# Validator index of V1 in the current set.
V1_ADDR=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["address"].upper())' "$D/n1/config/priv_validator_key.json")
V1_IDX=$(curl -s "$(rpc "$D" 0)/validators" | python3 -c '
import json, sys
vals = json.load(sys.stdin)["result"]["validators"]
print([v["address"].upper() for v in vals].index(sys.argv[1]))' "$V1_ADDR")
AT=$(( $(height "$D" 0) + 2 ))
echo "== byzantine peer: conflicting prevotes of validator $V1_ADDR (index $V1_IDX) to rust validator 2 at height $AT"
"$BYZ" --key "$D/n1/config/priv_validator_key.json" --val-index "$V1_IDX" --chain-id "$CHAIN_ID" \
  --dial "$(cat "$D/n2/node_id")@127.0.0.1:$((26656 + OFF + 20))" \
  --listen "127.0.0.1:$((26656 + OFF + 90))" --at-height "$AT" > "$D/byzantine.log" 2>&1 || FAIL+=("byzantine peer failed")
grep "sent conflicting prevote" "$D/byzantine.log" || FAIL+=("byzantine peer sent nothing")

# The evidence must be committed within 20 blocks.
EV_H=$(python3 - "$(rpc "$D" 0)" "$AT" "$V1_ADDR" <<'PY'
import json, sys, time, urllib.request
rpc, start, addr = sys.argv[1], int(sys.argv[2]), sys.argv[3]
get = lambda p: json.load(urllib.request.urlopen(rpc + p, timeout=10))["result"]
deadline = time.time() + 180
h = start
while time.time() < deadline and h < start + 20:
    if h > int(get("/status")["sync_info"]["latest_block_height"]):
        time.sleep(0.5)
        continue
    for ev in get(f"/block?height={h}")["block"]["evidence"]["evidence"] or []:
        v = ev.get("value", ev)
        a = v.get("vote_a", {}).get("validator_address", "").upper()
        if ev.get("type", "").endswith("DuplicateVoteEvidence") and a == addr:
            print(h)
            sys.exit(0)
    h += 1
sys.exit(1)
PY
) || FAIL+=("no DuplicateVoteEvidence for $V1_ADDR committed")
echo "evidence committed at height ${EV_H:-none}"

clean() { sed 's/\x1b\[[0-9;]*m//g' "$1"; }
clean "$D/n2/node.log" | grep -i "verified new evidence of byzantine behavior" | head -2 || FAIL+=("rust validator 2 did not verify the evidence")
# The Go nodes only saw the votes through the Rust node's evidence gossip
# (or the block a Rust proposer built): they must have verified it.
for i in 0 1; do
  clean "$D/n$i/node.log" | grep -qi "verified new evidence of byzantine behavior\|Attempting to add evidence" \
    && echo "go node $i verified the evidence" \
    || echo "go node $i did not log evidence verification (block from a rust proposer?)"
done

# Slashing: the double signer leaves the validator set; the network goes on.
TOP=$(( ${EV_H:-0} + 8 ))
wait_height "$D" 0 "$TOP" 240 || FAIL+=("network stalled after the evidence")
curl -s "$(rpc "$D" 0)/validators?height=$TOP" | grep -q "$V1_ADDR" && FAIL+=("double signer still in the validator set")
for i in 1 2 3; do wait_height "$D" $i "$TOP" 60 || FAIL+=("node $i stuck"); done
ALLOW_EVIDENCE=1 check_chain "$D" $((TOP - 1)) E || FAIL+=("chain checks")

for i in 0 1 2 3; do kill_node "$D" $i; done
if [ ${#FAIL[@]} -eq 0 ] && [ -n "${EVIDENCE_FIXTURE_OUT:-}" ]; then
  "$ROOT/conformance/devnet/capture-evidence-fixture.sh" "$D/n0" "$EV_H" "$V1_ADDR" "$EVIDENCE_FIXTURE_OUT"
fi
trap - EXIT
cleanup
if [ ${#FAIL[@]} -eq 0 ]; then
  echo "double-sign evidence OK"
  rm -rf "$W"
  exit 0
fi
printf 'FAILED: %s\n' "${FAIL[@]}"
echo "logs: $W"
exit 1
