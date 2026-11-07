#!/usr/bin/env bash
# Delivery differential (crates/fuzz-diff/src/deliver.rs): starts a Go
# celestia-appd as a standalone ABCI app, re-drives the recorded scenario
# into it and into the Rust app with funded fuzz accounts added to the
# genesis, then finalizes generated blocks of signed txs on both, comparing
# CheckTx, PrepareProposal, ProcessProposal, every tx result and the app
# hash of every block.
#
# usage: GO_APPD=<celestia-appd of the scenario's version> \
#        crates/fuzz-diff/deliver-diff.sh <scenario> [blocks] [txs-per-block] [seed]
#
# v10 scenarios need celestia-appd v10.6.0, v9-* scenarios v9.0.8, supplied
# through $DEVNET_BIN and $DEVNET_BIN/v9 by the differential CI build.
# Environment: DELIVER_TYPES=<substr,...> narrows the synthesized message
# types.
# DELIVER_TYPE_REQUIRE=<exact-url,...> fails unless every listed top-level
# message type has at least one successfully finalized Go transaction.
# DELIVER_SCRIPT=<path> sends signed transactions from a fixed script before
# generated traffic. DELIVER_SCRIPT_REQUIRE=<JSON path> requires every exact
# scripted transaction to finalize in order with the listed type_url,
# codespace, code and optional log_contains. Missing or reordered transactions,
# unfinished scripts and unexpected Go outcomes fail even when both apps agree.
# DELIVER_GOV_FAILURE_REQUIRE=<substring> requires that a tracked proposal fail
# with that Go log marker. DELIVER_GOV_FAILURE_TYPE_REQUIRE=<type-url> further
# binds the match to a proposal carrying that exact payload type.
# DELIVER_EVIDENCE_REQUIRE=stale|removed-validator|stake-lifecycle|zero-double-sign-slash sends exactly
# one controlled begin-block misbehavior and fails unless that boundary reaches
# FinalizeBlock. Stale evidence is supported on v9/v10; removed-validator is
# v10-only. Stake-lifecycle requires its exact DELIVER_SCRIPT and
# DELIVER_SCRIPT_REQUIRE setup and checks historical-stake slash accounting.
# Zero-double-sign-slash installs a valid zero double-sign fraction, requires
# evidence persistence, jail and tombstone with zero burn and no distribution
# slash record, then commits a signed recovery send in exactly three blocks.
# DELIVER_UNBONDED_REDELEGATION_ROLLBACK_REQUIRE=1 creates an out-of-set
# validator, attempts to redelegate its entire self-delegation, and requires
# the late source-not-found error to roll back every message-state write before
# a signed recovery send. It needs exactly four blocks and no random traffic.
# DELIVER_GOV_PARAMS_REJECTION_REQUIRE=1 submits 29 single-field invalid
# MsgUpdateParams payloads, deposits and passes each proposal, then requires
# exact deterministic failed reasons, queue/index cleanup, refunds and a signed
# recovery send. It needs exactly six blocks, no random traffic, and the
# controlled block-4 3600-second time jump.
# DELIVER_GOV_HANDLER_REJECTION_REQUIRE=1 exercises the metadata fallback,
# nil legacy content, weighted-vote overflow and legacy
# title/description bounds with exact rollback, proposal lifecycle, refunds
# and a signed recovery send. It needs exactly six blocks, no random traffic,
# and the controlled block-4 3600-second time jump.
# DELIVER_NEVER_BONDED_UNJAIL_REQUIRE=1 constrains staking to one active
# validator, creates a lower-power validator that never bonds, jails it by
# dropping below minimum self-delegation, restores the stake, and requires
# MsgUnjail to succeed without creating slashing signing info. It needs exactly
# five generated blocks and zero random transactions.
# DELIVER_VALIDATOR_REMOVAL_REWARDS_REQUIRE=1 creates an out-of-set validator,
# sets a distinct commission withdraw account, deposits 11utia, fully
# undelegates it and requires exact commission payout, decimal-dust accounting,
# retained user rewards and deletion of every validator reward record. It needs
# exactly five generated blocks and zero random transactions.
# DELIVER_DIRECT_AUX_REQUIRE=1 sends an implicit-fee-payer direct-aux
# transaction that must fail CheckTx with sdk/4, followed by the same sender
# using direct-aux while a separate direct signer pays the fee. It requires
# exactly two generated blocks and zero random transactions, and checks exact
# proposal filtering, execution, balances, and signer sequences.
# DELIVER_MALFORMED_MULTISIG_REQUIRE=1 sends three controlled malformed
# multisig shapes after bootstrap. It requires exactly four generated blocks
# and zero random transactions; each tx must fail CheckTx, be excluded by
# PrepareProposal and rejected by ProcessProposal, then is applied directly to
# FinalizeBlock to compare the exact ante result and committed-state rollback.
# DELIVER_ZERO_THRESHOLD_MULTISIG_REQUIRE=1 sends two controlled transactions
# from a wire-decoded threshold-zero multisig. It requires exactly three
# generated blocks and zero random transactions, and checks successful proposal
# handling, execution, balances, sequences, and exact public-key persistence.
# DELIVER_P256_REQUIRE=1 sends short, zero-scalar, wrong-sign-doc, and high-S
# P-256 signatures from a fresh account, followed by two valid low-S recovery
# transactions. It requires exactly seven generated blocks and zero random
# transactions, and checks proposal and execution outcomes, controlled
# forced-Finalize rejection rollback, and exact public-key persistence.
# DELIVER_PUBKEY_REJECTION_REQUIRE=1 sends malformed-length secp256k1 and
# Ed25519 keys, successful recovery sends from both accounts, and two valid
# Ed25519 rejection controls. It requires exactly seven generated blocks and
# zero random transactions, and checks exact panic/error outcomes, proposal
# handling, check-state recovery, and public-key persistence. Malformed-key
# panics must be rejected by ProcessProposal and are not force-finalized;
# returned Ed25519 errors are force-finalized to compare rollback.
# DELIVER_UPGRADE_PLAN_REQUIRE=1 first passes a proposal that resets the two
# upgrade circuit breakers, then passes separate proposals to schedule plan A,
# overwrite it with plan B, cancel it, and cancel again with no current plan.
# Five more passed-tally proposals must fail execution at the distinct time,
# upgraded-client-state, empty-name, nonpositive-height and past-height plan
# boundaries. It requires exactly 240 blocks and zero random transactions.
# Every stored proposal, final status/reason/tally, exact current/applied-plan
# query, app version and a final signed bank send is gated at both app versions.
# DELIVER_UPGRADE_HALT_REQUIRE=1 passes the circuit reset and an unknown-name
# upgrade plan due on the last of exactly 80 generated blocks. Both apps must
# stop at that height with the exact upgrade-needed error, write identical
# canonical upgrade-info.json files, retain the committed plan and leave the
# last committed height/app hash at H-1. Any earlier paired error fails.
# DELIVER_UPGRADE_SKIP_REQUIRE=1 passes the same two governance proposals, but
# starts Go with its real `--unsafe-skip-upgrades H` flag and configures Rust
# through ordinary NodeOptions. The signed send in block H must commit, the
# current plan must clear without an applied marker or version change, and
# neither app may write upgrade-info.json. It requires 80 blocks and no random
# transactions.
# DELIVER_UPGRADE_RESTART_REQUIRE=1 starts from the pinned v9 executable,
# schedules the canonical v10 plan for the last of 80 generated blocks, and
# requires GO_APPD_NEXT. Both apps must halt at H, write identical upgrade
# info, restart on their persistent homes with v10 execution semantics, retry
# the identical block, commit the handler result, restart once more, and
# commit a signed recovery send at H+1 while Info.app_version remains 9.
# DELIVER_IBC_PFM_METADATA_REQUIRE=1 uses the signed mock counterparty to
# relay exact typed-parse and identifier-validation failures plus a
# forward:null transfer control. It requires the official Go Ack bytes,
# committed Ack hashes and packet receipts, no forwarded packet or minted
# balance for either failure, and the exact control voucher balance.
# DELIVER_IBC_ICA_ORDER_REQUIRE=1 upgrades the ICA host channel from
# Unordered to Ordered and back, requiring an exact successful baseline
# packet and one after each transition. Both sequence-rewrite paths and all
# five lifecycle credits must complete exactly once.
# DELIVER_IBC_UPGRADE_TIMEOUT_REQUIRE=1 binds a real ICA upgrade Try and its
# raw state, requires an exact already-upgraded timeout rejection with no
# state mutation, then a distinct successful timeout proof and recovery Ack.
# It requires zero generated transactions so unrelated IBC activity cannot
# satisfy or perturb the checkpoint.
# DELIVER_IBC_UPGRADE_TIMEOUT_BOUNDARY_REQUIRE=1 advances the counterparty
# channel from OPEN to FLUSHING, proves timeout rejection at T-1 with no raw
# state mutation, then proves successful cleanup at T and an exact recovery
# Ack. It also requires zero generated transactions.
# DELIVER_IBC_UPGRADE_CONFIRM_TIMEOUT_REQUIRE=1 proves a matching FLUSHING
# channel and Upgrade at the exact counterparty timeout, requires Confirm's
# code-zero FAILURE abort, then an exact recovery Ack. It requires zero
# generated transactions.
# DELIVER_IBC_UPGRADE_TRY_FAST_FORWARD_REQUIRE=1 proves a clean OPEN ICA
# channel at sequence S, fast-forwards Try to S+2, completes proof-backed
# Confirm, rejects a proof-backed Open at S+1 without state mutation, then
# completes Open at S+2 and an exact post-upgrade ICA Ack. It requires zero
# generated transactions.
# DELIVER_IBC_UPGRADE_OPEN_FLUSHCOMPLETE_REQUIRE=1 advances a clean ICA
# channel through Try and Confirm, proves counterparty FLUSHCOMPLETE at the
# same sequence for Open, and requires an exact post-upgrade ICA Ack. It
# requires zero generated transactions.
# DELIVER_HL=1 drives incoming/outgoing Hyperlane transfers. Optional
# DELIVER_HL_ISM=noop|message-id|merkle-root selects the mailbox verifier
# (default noop). Multisig modes use harness-held 2-of-3 validators and
# fail as incomplete evidence unless both incoming collateral and synthetic
# transfers succeed. HL_VERIFIED_VERSION and DELIVER_STATS attribute successes
# to the app version. Signed upgrade runs require both paths at v9 before the
# held signal is released. During that mandatory setup, unrelated generated
# traffic (including parameter-changing governance) waits. After both paths
# succeed, normal traffic resumes. DELIVER_STATS records the reserved block
# count as hl_preupgrade_blocks; the requested block budget still bounds setup.
# DELIVER_HL_REGISTRY_REQUIRE=1 additionally requires the reserved sequence's
# merkle-root ISM creation and signed validator announcement to finalize
# successfully; unrelated generated handler successes do not count.
# DELIVER_HL_RECOVERY_ID_REQUIRE=1 isolates signed validator announcements
# with recovery wire IDs 29 and 30. App version 9 must accept and store both;
# app version 10 must reject both without state mutation, then accept a
# canonical wire-ID-27 announcement. It requires DELIVER_HL=1, zero generated
# transactions and no upgrade or rejection fixture.
# The upgrade driver's block counter pauses during isolated setup (including
# rejection setup), preserving its one-shot validator/vesting/grant setup
# and DELIVER_UPGRADE signal delay.
# DELIVER_HL_ALLOW_POSTUPGRADE=governance-fixture is the narrow exception for
# a fixed DELIVER_SCRIPT + DELIVER_GOV_FAILURE_REQUIRE state regression.
# DELIVER_HL_REJECT=1 requires a signed ISM; it first rejects malformed
# metadata then retries the same message in adjacent transactions. Every
# required pair must reject at verification and then succeed, or exit nonzero.
# Cannot be combined with DELIVER_UPGRADE (its early fixture windows).
#
# The v9 -> v10 upgrade (GO_APPD = v9.0.8, GO_APPD_NEXT = v10.6.0): when Go
# raises the app version, the harness stops the Go app after that Commit
# and starts GO_APPD_NEXT on the same home and address (the multiplexer's
# switch, as abci-recorder reproduces it); the Rust app continues.
#   GO_APPD=.../v9/celestia-appd GO_APPD_NEXT=.../celestia-appd \
#     DELIVER_UPGRADE=20 crates/fuzz-diff/deliver-diff.sh v9-upgrade 120
# DELIVER_UPGRADE=<n> stops the replay before the scenario's signal txs and
# sends them from driver block n (generated blocks excluding isolated setup;
# see crates/fuzz-diff/src/deliver/upgrade.rs);
# without it the recorded upgrade of v9-upgrade is replayed (switch included).
set -euo pipefail
SCENARIO=${1:?scenario}
BLOCKS=${2:-200}
PER=${3:-8}
SEED=${4:-1}
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
APPD=${GO_APPD:?path to celestia-appd}
PORT=$((37858 + ${DEVNET_PORT_OFFSET:-0}))
IN=$ROOT/conformance/fixtures/devnet/$SCENARIO
WORK=$(mktemp -d)
H=$WORK/home
if [ -n "${DELIVER_UPGRADE_HALT_REQUIRE:-}${DELIVER_UPGRADE_SKIP_REQUIRE:-}${DELIVER_UPGRADE_RESTART_REQUIRE:-}" ]; then
  export DELIVER_UPGRADE_GO_HOME=$H
  export DELIVER_UPGRADE_RUST_HOME=$WORK/rust-home
fi
if [ -n "${DELIVER_UPGRADE_RESTART_REQUIRE:-}" ] && [ -z "${GO_APPD_NEXT:-}" ]; then
  echo "DELIVER_UPGRADE_RESTART_REQUIRE needs GO_APPD_NEXT" >&2
  exit 2
fi
if [ -n "${DELIVER_UPGRADE_SKIP_REQUIRE:-}" ]; then
  META_HEIGHT=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['committed_heights'])" "$IN/meta.json")
  export DELIVER_UPGRADE_SKIP_HEIGHT=$((META_HEIGHT + BLOCKS))
fi
CHAIN_ID=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['chain_id'])" "$IN/meta.json")
"$APPD" init node --chain-id "$CHAIN_ID" --home "$H" >"$WORK/init.log" 2>&1 || {
  echo "$APPD init failed:" >&2
  tail -n 20 "$WORK/init.log" >&2
  exit 4
}
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
# Scenarios recorded without a genesis file (the ibctesting fixtures): one
# from the recorded InitChain.
if [ -f "$IN/genesis.json" ]; then
  cp "$IN/genesis.json" "$H/config/genesis.json"
else
  "$FUZZ_DIFF" genesis "$SCENARIO" >"$H/config/genesis.json"
fi
# The gRPC server on: the SDK then registers the tx service on the gRPC
# query router, so ABCI Query serves `Service/Simulate` (`DELIVER_SIM`) as
# a node with comet and gRPC enabled does.
GRPC=$((39690 + ${DEVNET_PORT_OFFSET:-0}))
python3 - "$H/config/app.toml" "$GRPC" <<'PY'
import re, sys
p, port = sys.argv[1], sys.argv[2]
s = open(p).read()
i = s.index("[grpc]")
j = s.index("[", i + 1)
sec = re.sub(r'(?m)^enable = .*$', 'enable = true', s[i:j])
sec = re.sub(r'(?m)^address = .*$', 'address = "127.0.0.1:%s"' % port, sec)
open(p, "w").write(s[:i] + sec + s[j:])
PY
# Bash 3.2 treats an empty array expansion as unset under `set -u`, so keep
# the ordinary launch argument-free instead of collecting optional arguments.
if [ -n "${DELIVER_UPGRADE_SKIP_REQUIRE:-}" ]; then
  GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$APPD" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" --unsafe-skip-upgrades "$DELIVER_UPGRADE_SKIP_HEIGHT" >"$WORK/app.log" 2>&1 &
else
  GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$APPD" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >"$WORK/app.log" 2>&1 &
fi
APP=$!
echo $APP >"$WORK/app.pid"
if [ -n "${GO_APPD_NEXT:-}" ]; then
  # Run by the harness after the Commit that raised the app version.
  cat >"$WORK/switch.sh" <<SH
set -e
OLD=\$(cat "$WORK/app.pid")
kill \$OLD
while kill -0 \$OLD 2>/dev/null; do
  case \$(ps -o stat= -p \$OLD 2>/dev/null) in
    "" | *Z*) break ;;
  esac
  sleep 0.2
done
GOMEMLIMIT=${GOMEMLIMIT:-1500MiB} "$GO_APPD_NEXT" start --force-no-bbr --home "$H" --with-comet=false --address "tcp://127.0.0.1:$PORT" >>"$WORK/app.log" 2>&1 &
echo \$! >"$WORK/app.pid"
until (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null; do sleep 0.2; done
SH
  export DELIVER_SWITCH_CMD="bash $WORK/switch.sh"
fi
# GO_LOG=<file> keeps the Go app's log.
# The Go app must be gone before the next run binds the same ports: its
# shutdown can stall, so SIGKILL it after 10 s.
app_running() {
  kill -0 "$1" 2>/dev/null || return 1
  # After an upgrade, the new Go app is a child of switch.sh. A minimal
  # container init may leave it as a zombie after exit; kill -0 still
  # succeeds then, even though it has released its ports and files.
  case "$(ps -o stat= -p "$1" 2>/dev/null)" in
    "" | *Z*) return 1 ;;
  esac
}
stop_app() {
  local pid
  pid=$(cat "$WORK/app.pid" 2>/dev/null) || return 0
  kill "$pid" 2>/dev/null || return 0
  for _ in $(seq 1 100); do
    app_running "$pid" || return 0
    sleep 0.1
  done
  kill -9 "$pid" 2>/dev/null || true
  for _ in $(seq 1 100); do
    app_running "$pid" || return 0
    sleep 0.1
  done
}
trap 'stop_app; if [ -n "${GO_LOG:-}" ]; then cp "$WORK/app.log" "$GO_LOG"; fi; rm -rf "$WORK"' EXIT
UP=
for _ in $(seq 1 300); do
  (exec 3<>/dev/tcp/127.0.0.1/$PORT) 2>/dev/null && UP=1 && break
  kill -0 $APP 2>/dev/null || break
  sleep 1
done
if [ -z "$UP" ]; then
  echo "Go celestia-appd exited or did not listen on $PORT within 300 s" >&2
  tail -n 20 "$WORK/app.log" >&2
  exit 3
fi
FUZZ_DIFF=${FUZZ_DIFF:-$ROOT/target/release/fuzz-diff}
"$FUZZ_DIFF" deliver "$SCENARIO" "127.0.0.1:$PORT" "$BLOCKS" "$PER" "$SEED"
