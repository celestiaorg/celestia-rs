#!/usr/bin/env bash
# Tests the pure helpers in scripts/bench/checkpoints-lib.sh and a dry run of
# checkpoints-vm.sh. Needs bash, jq, python3, sha256sum and awk; no cloud
# access. CI runs it (ci.yml).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=scripts/bench/lib.sh
. "$here/lib.sh"
# shellcheck source=scripts/bench/checkpoints-lib.sh
. "$here/checkpoints-lib.sh"
fixture=$here/../../conformance/fixtures/mainnet/replay-windows.json
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0
check() { # check NAME COMMAND...: the command must succeed
  local name=$1; shift
  if "$@" >/dev/null 2>&1; then echo "ok   $name"; else echo "FAIL $name"; fails=$((fails + 1)); fi
}
refute() { # refute NAME COMMAND...: the command must fail
  local name=$1; shift
  if "$@" >/dev/null 2>&1; then echo "FAIL $name"; fails=$((fails + 1)); else echo "ok   $name"; fi
}
eq() { # eq NAME EXPECTED ACTUAL
  if [[ $2 == "$3" ]]; then echo "ok   $1"; else echo "FAIL $1: want '$2' got '$3'"; fails=$((fails + 1)); fi
}

# --- input validation ---------------------------------------------------------
check  "name ok"                ckpt_valid name mainnet-1
refute "name upper"             ckpt_valid name Mainnet
refute "name leading digit"     ckpt_valid name 1mainnet
refute "name too long"          ckpt_valid name abcdefghijklmnopqrstu
refute "name injection"         ckpt_valid name 'a;b'
check  "heights ok"             ckpt_valid heights 2371489,2993219
check  "heights default"        ckpt_valid heights "$CKPT_DEFAULT_HEIGHTS"
refute "heights empty"          ckpt_valid heights ""
refute "heights trailing comma" ckpt_valid heights 2371489,
refute "heights zero"           ckpt_valid heights 0
refute "heights space"          ckpt_valid heights '2371489, 2993219'
refute "heights too many"       ckpt_valid heights "$(printf '1%.0s,' {1..16})1"
check  "disk ok"                ckpt_valid disk_gb 1000
check  "disk min"               ckpt_valid disk_gb 200
refute "disk over approval"     ckpt_valid disk_gb 1001
refute "disk small"             ckpt_valid disk_gb 199
refute "disk alpha"             ckpt_valid disk_gb 1e3
check  "hours seven days"       ckpt_valid hours 168
refute "hours over a week"      ckpt_valid hours 169
refute "hours too short"        ckpt_valid hours 5
check  "disks all"              ckpt_valid disks all
refute "disks other"            ckpt_valid disks boot
check  "hash ok"                ckpt_valid hash "$(printf 'A%.0s' {1..64})"
refute "hash lower"             ckpt_valid hash "$(printf 'a%.0s' {1..64})"
refute "unknown kind"           ckpt_valid colour red

# --- app version at a height: must agree with the pinned replay windows ------
eq "version at genesis"         1 "$(ckpt_app_version 1)"
eq "version before v2"          1 "$(ckpt_app_version 2371494)"
eq "version at v2"              2 "$(ckpt_app_version 2371495)"
eq "version v7 skipped"         8 "$(ckpt_app_version 10960600)"
eq "version v9 window"          9 "$(ckpt_app_version 15105000)"
while read -r start version; do
  eq "window $version first height" "$version" "$(ckpt_app_version "$start")"
  ((start == 1)) || [[ $version == 9 || $version == 10 ]] ||
    refute "window $version starts an app version" test "$(ckpt_app_version $((start - 1)))" = "$version"
done < <(jq -r '.windows[] | "\(.start_height) \(.app_version)"' "$fixture")

# --- which binary exports a height --------------------------------------------
eq "export v1 with v3"          "passthrough v3" "$(ckpt_export_cmd 2371489)"
eq "export v3 last block"       "passthrough v3" "$(ckpt_export_cmd 6680339)"
eq "export v4 last block"       "passthrough v4" "$(ckpt_export_cmd 6748821)"
eq "export v8 last block"       "passthrough v8" "$(ckpt_export_cmd 11771698)"
eq "export v9"                  "passthrough v9" "$(ckpt_export_cmd 15105000)"
eq "embedded tag v1"            v3.13.0 "$(ckpt_embedded_tag 1)"
eq "embedded tag v6"            v6.4.10 "$(ckpt_embedded_tag 6)"
refute "embedded tag v7"        ckpt_embedded_tag 7
eq "snapshot format v3"         2 "$(ckpt_format 3)"
eq "snapshot format v4"         3 "$(ckpt_format 4)"

# --- the checkpoint plan --------------------------------------------------------
eq "plan sorted unique"  $'2371489 1\n2993219 2\n6680339 3' "$(ckpt_plan 6680339,2371489,2993219,2371489)"
eq "plan default count"  9 "$(ckpt_plan "$CKPT_DEFAULT_HEIGHTS" | wc -l | tr -d ' ')"
eq "plan default first"  "2371489 1" "$(ckpt_plan "$CKPT_DEFAULT_HEIGHTS" | head -1)"
# The default plan covers the state before every pinned window after v1.
for start in $(jq -r '.windows[] | select(.start_height > 1 and .app_version < 10) | .start_height' "$fixture"); do
  check "default plan has $((start - 1))" grep -qx "$((start - 1)) [0-9]*" <(ckpt_plan "$CKPT_DEFAULT_HEIGHTS")
done
mkdir -p "$t/out/2371489" "$t/out/2993219"
echo '{}' >"$t/out/2371489/checkpoint.json"
eq "next skips done"     2993219 "$(ckpt_next "$t/out" 2371489,2993219,6680339)"
echo '{}' >"$t/out/2993219/checkpoint.json"
eq "next after two"      6680339 "$(ckpt_next "$t/out" 2371489,2993219,6680339)"
mkdir -p "$t/out/6680339" && echo '{}' >"$t/out/6680339/skipped.json"
refute "next none left"  ckpt_next "$t/out" 2371489,2993219,6680339

# --- halt detection ---------------------------------------------------------------
cat >"$t/node.log" <<'EOF'
INF committed state height=2371489
panic: Failed to process committed block (2371490:0AB1): failed to finalize block because the node should halt: halting node per configuration at height 2371490
EOF
check  "halt seen"              ckpt_halted "$t/node.log" 2371489
refute "halt for another height" ckpt_halted "$t/node.log" 2371494
refute "halt prefix is not a match" ckpt_halted "$t/node.log" 237148

# --- chunk inventory and metadata -------------------------------------------------
d=$t/snap/2371489/2
mkdir -p "$d"
printf 'zero' >"$d/0"; printf 'one!!' >"$d/1"
chunks=$(ckpt_chunks "$d")
eq "chunks count"     2 "$(jq length <<<"$chunks")"
eq "chunks sha"       "$(printf 'one!!' | sha256sum | cut -d' ' -f1)" "$(jq -r '.[1].sha256' <<<"$chunks")"
eq "chunks bytes"     5 "$(jq -r '.[1].bytes' <<<"$chunks")"
printf 'x' >"$d/3"
refute "chunks gap"   ckpt_chunks "$d"
rm "$d/3"
refute "chunks none"  ckpt_chunks "$t/empty-does-not-exist"
meta=$(ckpt_metadata "$d" app_version=1 block_hash=ABC exporter="passthrough v3")
check "metadata fields" jq -e '.height == 2371489 and .format == 2 and .chunk_count == 2 and .total_bytes == 9
  and .app_version == "1" and .block_hash == "ABC" and .exporter == "passthrough v3" and (.chunks | length) == 2' <<<"$meta"
refute "metadata bad arg" ckpt_metadata "$d" noequals

# --- sync rate and ETA from polls (epoch_s height ...) ---------------------------
printf '1000 100\n1600 400\n2200 1300\n' >"$t/polls"
eq "rate over window"   1.00 "$(ckpt_rate "$t/polls" 1200)"
eq "rate recent"        1.50 "$(ckpt_rate "$t/polls" 600)"
eq "rate no data"       - "$(ckpt_rate /dev/null 600)"
eq "eta"                400 "$(ckpt_eta 1.50 1300 1900)"
eq "eta unknown rate"   - "$(ckpt_eta - 1300 1900)"
eq "eta reached"        0 "$(ckpt_eta 1.50 1900 1900)"

# --- VM dry run: validates metadata and writes the plan, no cloud calls ----------
dry() { # dry DIR HEIGHTS: one dry run of checkpoints-vm.sh
  env CKPT_DRY_RUN="$1" CKPT_META_name=mainnet-1 CKPT_META_heights="$2" \
    CKPT_META_rs_commit=58be8ae90d00518800df5506b3a1e988f009c1c6 CKPT_META_run_id=7 \
    CKPT_META_peers="$CKPT_ARCHIVE_PEERS" CKPT_META_deadline=4102444800 \
    bash "$here/checkpoints-vm.sh"
}
rc=0; out=$(dry "$t/dry" 2371489,6748821 2>&1) || rc=$?
eq "dry run exits ok" 0 "$rc"
[[ $rc == 0 ]] || printf '%s\n' "$out" | sed 's/^/     | /'
p=$t/dry/plan
check "dry run multiplexer build"  grep -q 'make -C .*celestia-app build' "$p"
check "dry run v3 export"          grep -q 'passthrough v3 snapshots export --height 2371489 --home' "$p"
check "dry run v4 export"          grep -q 'passthrough v4 snapshots export --height 6748821 --home' "$p"
check "dry run halts after H"      grep -q 'halt-height = 2371490' "$t/dry/plan.app.toml.2371489"
check "dry run halts after H 2"    grep -q 'halt-height = 6748822' "$t/dry/plan.app.toml.6748821"
check "dry run window verify"      grep -q 'verify-replay-checkpoint.py .*/6748821/3$' "$p"
check "dry run header verify"      grep -q 'verify-replay-checkpoint.py .*/2371489/2 --app-hash' "$p"
check "dry run egress cap"         grep -q 'htb rate 4mbit ceil 4mbit' "$p"
# Cloud Storage uploads and the metadata server bypass the cap.
check "dry run gcs uncapped"       grep -q 'u32 match ip dst 199.36.153.8/30 flowid 1:10' "$p"
check "dry run vpc uncapped"       grep -q 'u32 match ip dst 10.128.0.0/20 flowid 1:10' "$p"
check "dry run metadata uncapped"  grep -q 'u32 match ip dst 169.254.169.254/32 flowid 1:10' "$p"
check "dry run gcs via private vip" grep -q '199.36.153.8 storage.googleapis.com' "$t/dry/hosts"
check "dry run bbr"                grep -q 'tcp_congestion_control=bbr' "$p"
# Unattended upgrades once restarted the driver through needrestart; the
# restarted driver then failed on the apt lock and stopped the node.
check "dry run no unattended upgrades" grep -q 'systemctl disable --now unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer' "$p"
check "dry run uploads each checkpoint" grep -q 'vmx-gcs put .*/ckpt/6748821/3/0 checkpoints/mainnet-1/6748821/3/0$' "$p"
check "dry run raw results to bucket" grep -q 'vmx-gcs put .*/raw.tar checkpoints/mainnet-1/raw/[0-9TZ]*.tar$' "$p"
check "dry run raw not in attributes" test ! -e "$t/dry/attrs/raw-0"
check "dry run uploads metadata"  grep -q 'vmx-gcs put .*/ckpt/2371489/checkpoint.json checkpoints/mainnet-1/2371489/checkpoint.json$' "$p"
check "dry run apt waits for the lock" grep -q 'apt-get -o DPkg::Lock::Timeout=900 update' "$p"
check "dry run checkpoint copied"  test -s "$t/dry/ckpt/2371489/2/0"
check "dry run frees the node copy" grep -q 'passthrough v3 snapshots delete 2371489 2 --home' "$p"
check "dry run metadata"           jq -e '.height == 2371489 and .format == 2 and .app_version == "1" and .embedded_tag == "v3.13.0"' "$t/dry/ckpt/2371489/checkpoint.json"
check "dry run metadata v4"        jq -e '.format == 3 and .exporter == "passthrough v4"' "$t/dry/ckpt/6748821/checkpoint.json"
check "dry run summary"            jq -e '.outcome == "dry_run" and (.checkpoints | length) == 2' "$t/dry/attrs/summary"
check "dry run publishes each"     test -s "$t/dry/attrs/ckpt-6748821"
check "dry run never formats used disks" bash -c "! grep -q mkfs '$p'"
check "dry run discards abci responses" grep -q 'discard_abci_responses = true' "$t/dry/data/home/config/config.toml"
check "dry run no iavl fast nodes"  grep -q '^iavl-disable-fastnode = true' "$t/dry/data/home/config/app.toml"
check "dry run archive peers"      grep -q "persistent_peers = \".*${CKPT_ARCHIVE_PEERS%%,*}" "$t/dry/data/home/config/config.toml"
# A rerun with the same disks resumes: done checkpoints are not exported again.
rc=0; dry "$t/dry" 2371489,6748821,8662012 >/dev/null 2>&1 || rc=$?
eq "dry resume exits ok" 0 "$rc"
eq "dry resume exports once" 1 "$(grep -c 'snapshots export --height 2371489' "$t/dry/plan")"
check "dry resume continues"       grep -q 'passthrough v5 snapshots export --height 8662012' "$t/dry/plan"
# A checkpoint that fails verification is kept as failed.json, never as
# checkpoint.json, so a resumed run does not skip it and move on.
rc=0; CKPT_DRY_VERIFY_RC=1 dry "$t/dry-fail" 2371489,6748821 >/dev/null 2>&1 || rc=$?
check "failed verify fails the run" test "$rc" -ne 0
check "failed verify is not a checkpoint" test ! -e "$t/dry-fail/ckpt/2371489/checkpoint.json"
check "failed verify keeps evidence" jq -e '.verified == "false" and .height == 2371489' "$t/dry-fail/ckpt/2371489/failed.json"
check "failed verify stops the plan" bash -c "! grep -q 'snapshots export --height 6748821' '$t/dry-fail/plan'"
rc=0; CKPT_DRY_VERIFY_RC=1 dry "$t/dry-fail" 2371489,6748821 >/dev/null 2>&1 || rc=$?
check "resume retries the failed height" test "$rc" -ne 0
check "resume still does not pass it" bash -c "! grep -q 'snapshots export --height 6748821' '$t/dry-fail/plan'"
rc=0; dry "$t/dry-fail" 2371489,6748821 >/dev/null 2>&1 || rc=$?
eq "resume after a good verify exits ok" 0 "$rc"
check "good verify replaces the failure" bash -c "test -s '$t/dry-fail/ckpt/2371489/checkpoint.json' && test ! -e '$t/dry-fail/ckpt/2371489/failed.json'"
rc=0
env CKPT_DRY_RUN="$t/dry-bad" CKPT_META_name='x;reboot' bash "$here/checkpoints-vm.sh" >/dev/null 2>&1 || rc=$?
check "dry run rejects bad metadata" test "$rc" -ne 0

[[ $fails == 0 ]] || { echo "$fails failed"; exit 1; }
echo "all passed"
