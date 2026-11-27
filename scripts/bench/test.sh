#!/usr/bin/env bash
# Tests the pure helpers in scripts/bench/lib.sh and a dry run of vm.sh.
# Needs bash, jq, gzip, base64 and awk; no cloud access. CI runs it (ci.yml).
# shellcheck disable=SC2031 # fixtures source vm.sh in subshells; it sets the same values
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=scripts/bench/lib.sh
. "$here/lib.sh"
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
check  "campaign ok"           bench_valid campaign state-sync
refute "campaign unknown"      bench_valid campaign state
refute "campaign injection"    bench_valid campaign 'bridge;id'
check  "height ok"             bench_valid height 10
refute "height zero"           bench_valid height 0
refute "height too long"       bench_valid height 123456789
refute "height newline"        bench_valid height $'10\n11'
check  "pairs ok"              bench_valid pairs 3
refute "pairs four"            bench_valid pairs 4
check  "commit ok"             bench_valid commit 58be8ae90d00518800df5506b3a1e988f009c1c6
refute "commit short"          bench_valid commit 58be8ae
refute "commit upper"          bench_valid commit 58BE8AE90D00518800DF5506B3A1E988F009C1C6
check  "run id ok"             bench_valid run_id 12345678901
refute "run id alpha"          bench_valid run_id 12a
check  "trust hash ok"         bench_valid trust_hash "$(printf 'A%.0s' {1..64})"
refute "trust hash short"      bench_valid trust_hash ABCD
check  "role ok"               bench_valid role go
refute "role other"            bench_valid role java
check  "minutes ok"            bench_valid minutes 60
refute "minutes big"           bench_valid minutes 1000
peer=0cd6b8188957d18e8326559ee59e86fad4bbc2c0@170.101.101.70:26656
check  "peers one"             bench_valid peers "$peer"
check  "peers hostname"        bench_valid peers "$peer,076c9cc9de3039819a98522aeea39e8c72b49682@peer.celestia.mainnet.dteam.tech:28656"
check  "peers empty"           bench_valid peers ""
refute "peers trailing comma"  bench_valid peers "$peer,"
refute "peers quote"           bench_valid peers "$peer\""
refute "peers space"           bench_valid peers "$peer x"
refute "peers too many"        bench_valid peers "$(printf "$peer,%.0s" {1..40})$peer"
refute "unknown kind"          bench_valid colour red

# --- tip rule: catching_up false, within one block, two polls, advancing ------
tip() { printf '%s\n' "$@" | bench_tip; }
refute "tip never"             tip "5 10 100 true" "10 50 100 true"
eq     "tip first qualifying"  "15 100" "$(tip '5 90 101 true' '10 99 101 false' '15 100 101 false' '20 101 101 false')"
eq     "tip waits for advance" "15 100" "$(tip '10 100 101 false' '15 100 101 false' '20 101 101 false' '25 102 102 false')"
refute "tip single poll"       tip "10 100 100 false" "15 101 103 false"
eq     "tip ahead by one"      "5 101" "$(tip '5 101 100 false' '10 102 101 false')"
refute "tip ahead by two"      tip "5 102 100 false" "10 103 101 false"
refute "tip catching up"       tip "5 100 100 true" "10 101 101 true"
refute "tip malformed breaks"  tip "5 100 100 false" "10 x 101 false" "15 101 101 false"
refute "tip empty reference"   tip "5 100 - false" "10 101 - false"

# --- pair validity ------------------------------------------------------------
eq     "pair same snapshot"    valid "$(bench_pair_check 16200000 16200000)"
eq     "pair different"        discard:different_snapshots "$(bench_pair_check 16200000 16198500 || true)"
eq     "pair missing"          discard:missing_snapshot "$(bench_pair_check '' 16200000 || true)"
refute "pair exits nonzero"    bench_pair_check 1 2

# Exercise collection against the summary.json format emitted by vm.sh.
rs="$t/rust-summary.json"; go="$t/go-summary.json"
jq -n --arg hash "$(printf 'A%.0s' {1..64})" '{
  role:"rs", campaign:"state-sync", outcome:"ok", snapshot:"16200000",
  verify:[{height:"16200099", match:true, chain_id:"celestia",
           local_block_id:$hash, reference_block_id:$hash}]
}' >"$rs"
jq '.role = "go"' "$rs" >"$go"
pair() { bench_pair_verdict "$1" "$rs" "$go"; }
eq "verified pair" valid "$(pair state-sync)"
jq '.outcome = "failed"' "$go" >"$t/failed"
eq "failed run with same snapshot" discard:incomplete_run "$(bench_pair_verdict state-sync "$rs" "$t/failed" || true)"
for invalid in '.verify = []' '.verify[0].match = false' '.verify[0].chain_id = "other"' \
  '.verify[0].local_block_id = ""' '.verify[0].reference_block_id = "different"'; do
  jq "$invalid" "$go" >"$t/unverified"
  eq "unverified tip ($invalid)" discard:unverified_tip "$(bench_pair_verdict state-sync "$rs" "$t/unverified" || true)"
done
jq '.snapshot = "16198500"' "$go" >"$t/different"
eq "verified different snapshots" discard:different_snapshots "$(bench_pair_verdict state-sync "$rs" "$t/different" || true)"
eq "pending collection" discard:missing_or_invalid_summary "$(bench_pair_verdict state-sync "$rs" "$t/absent" || true)"
printf '{' >"$t/partial"
refute "partial summary" bench_pair_verdict state-sync "$rs" "$t/partial"
refute "wrong role" bench_pair_verdict state-sync "$rs" "$rs"

jq --arg digest "$(printf 'a%.0s' {1..64})" '
  .campaign = "bridge" | .bridge = {tail:16200001, start:16200100, end:16200700, requests:32, digest:$digest}
' "$rs" >"$t/bridge"
cp "$t/bridge" "$rs"
jq '.role = "go"' "$rs" >"$go"
eq "same verified bridge work" valid "$(pair bridge)"
for invalid in 'del(.bridge)' '.bridge.requests = 31' '.bridge.end -= 1' '.bridge.digest = ""' \
  '.bridge.tail = (.bridge.start + 1)' '.bridge.start |= tostring'; do
  jq "$invalid" "$go" >"$t/incomplete"
  eq "incomplete bridge ($invalid)" discard:incomplete_bridge_receipt "$(bench_pair_verdict bridge "$rs" "$t/incomplete" || true)"
done
jq '.bridge.tail -= 1' "$go" >"$t/different"
eq "unequal DA range" discard:different_da_ranges "$(bench_pair_verdict bridge "$rs" "$t/different" || true)"
jq '.bridge.digest |= gsub("a"; "b")' "$go" >"$t/different"
eq "unequal share responses" discard:different_workloads "$(bench_pair_verdict bridge "$rs" "$t/different" || true)"
refute "bridge discard exits nonzero" bench_pair_verdict bridge "$rs" "$t/different"

# --- chunking into guest-attribute-sized values -------------------------------
head -c 300000 /dev/urandom >"$t/raw"
n=$(bench_chunk "$t/raw" "$t/c" 100000 8)
eq     "chunk count"           5 "$n"   # 300000 bytes gzip-incompressible, base64 ~400 KB
check  "chunk size bound"      test "$(wc -c <"$t/c.0")" -le 100000
bench_unchunk "$n" "$t/c" >"$t/back"
check  "chunk round trip"      cmp "$t/raw" "$t/back"

# --- log excerpt: what explains a slow or failed sync, small enough to publish
{
  echo "t0 INFO starting node"
  for i in $(seq 1 9000); do echo "t$i INFO p2p::switch: Added peer $i"; done
  echo "t9001 INFO statesync::syncer: Discovering snapshots discovery_time=15s"
  echo "t9002 ERROR light: verification failed"
  echo "t9003 INFO Snapshot restored height=16212000"
  for i in $(seq 1 9000); do echo "t$i INFO consensus: committed block"; done
} >"$t/node.log"
bench_log_excerpt <"$t/node.log" >"$t/excerpt"
check  "excerpt keeps start"     grep -q 'starting node' "$t/excerpt"
check  "excerpt keeps statesync" grep -q 'Discovering snapshots' "$t/excerpt"
check  "excerpt keeps errors"    grep -q 'verification failed' "$t/excerpt"
check  "excerpt keeps restore"   grep -q 'Snapshot restored height=16212000' "$t/excerpt"
check  "excerpt is small"        test "$(wc -l <"$t/excerpt")" -le 4000
# Many late matches must not evict the early ones that explain a long wait.
{
  for i in $(seq 1 300); do echo "t$i INFO p2p::switch: Added peer $i"; done
  echo "t301 ERROR p2p::pex::reactor: Couldn't connect to any seeds"
  for i in $(seq 1 9000); do echo "t$i INFO blocksync: applied block $i"; done
  echo "t9999 ERROR last problem"
} >"$t/node2.log"
bench_log_excerpt <"$t/node2.log" >"$t/excerpt2"
check  "excerpt keeps early match" grep -q "Couldn't connect to any seeds" "$t/excerpt2"
check  "excerpt keeps last match"  grep -q 'last problem' "$t/excerpt2"
check  "excerpt marks the gap"     grep -q 'matching lines omitted' "$t/excerpt2"
check  "excerpt 2 is small"        test "$(wc -l <"$t/excerpt2")" -le 4000
refute "chunk over limit"      bench_chunk "$t/raw" "$t/d" 100000 4
check  "chunk alphabet"        grep -qvE '[^A-Za-z0-9+/=]' "$t/c.0"

# --- TOML edits ---------------------------------------------------------------
printf 'pruning = "default"\n\n[p2p]\nseeds = ""\n\n[statesync]\nenable = false\n' >"$t/c.toml"
bench_toml_set "$t/c.toml" statesync enable true
bench_toml_set "$t/c.toml" p2p persistent_peers '"a@b:1"'
bench_toml_set "$t/c.toml" "" pruning '"custom"'
bench_toml_set "$t/c.toml" tx_index indexer '"null"'
eq "toml edit" $'pruning = "custom"\n\n[p2p]\nseeds = ""\npersistent_peers = "a@b:1"\n\n[statesync]\nenable = true\n\n[tx_index]\nindexer = "null"' "$(cat "$t/c.toml")"

# --- peers from /net_info -----------------------------------------------------
cat >"$t/net.json" <<EOF
{"result":{"peers":[
 {"node_info":{"id":"0cd6b8188957d18e8326559ee59e86fad4bbc2c0","listen_addr":"tcp://0.0.0.0:26656"},"remote_ip":"170.101.101.70"},
 {"node_info":{"id":"bad","listen_addr":"tcp://0.0.0.0:26656"},"remote_ip":"1.2.3.4"},
 {"node_info":{"id":"54280fb97e7dca08380106c8a3f4fb66ebb58b77","listen_addr":"92.42.111.49:36656"},"remote_ip":"92.42.111.49"},
 {"node_info":{"id":"6d661e1af1f53e7cd34664313df055dc40399795","listen_addr":"tcp://0.0.0.0:26656"},"remote_ip":"2001:db8::1"}]}}
EOF
eq "peers from net_info" "0cd6b8188957d18e8326559ee59e86fad4bbc2c0@170.101.101.70:26656,54280fb97e7dca08380106c8a3f4fb66ebb58b77@92.42.111.49:36656" "$(bench_peers "$t/net.json" 20)"
eq "peers limit" "0cd6b8188957d18e8326559ee59e86fad4bbc2c0@170.101.101.70:26656" "$(bench_peers "$t/net.json" 1)"

# --- column maximum -----------------------------------------------------------
printf '1 2 3\n4 9 1\n7 - 2\n' >"$t/p"
eq "max column"     9 "$(bench_max "$t/p" 2)"
eq "max empty"      "" "$(bench_max /dev/null 2)"

# --- vm.sh dry run: validates metadata and writes the plan, no cloud calls ----
for c in genesis-prefix state-sync bridge; do
  for r in rs go; do
    out=$(env BENCH_DRY_RUN="$t/dry-$c-$r" BENCH_META_campaign=$c BENCH_META_role=$r \
      BENCH_META_height=10 BENCH_META_rust_commit=58be8ae90d00518800df5506b3a1e988f009c1c6 \
      BENCH_META_bridge_start=16205000 BENCH_META_trust_height=16200000 BENCH_META_trust_hash="$(printf 'A%.0s' {1..64})" \
      BENCH_META_peers="$peer" BENCH_META_deadline=4102444800 \
      bash "$here/vm.sh" 2>&1) && rc=0 || rc=$?
    eq "dry run $c $r" 0 "$rc"
    [[ $rc == 0 ]] || printf '%s\n' "$out" | sed 's/^/     | /'
  done
done
check "dry run published"       test -s "$t/dry-state-sync-go/attrs/summary"
check "dry run go multiplexer"  grep -q 'make -C .*celestia-app build' "$t/dry-genesis-prefix-go/plan"
check "dry run rs state sync"   grep -q -- '--statesync-trust-height 16200000' "$t/dry-state-sync-rs/plan"
check "dry run same config"     cmp "$t/dry-state-sync-rs/data/home/config/config.toml" "$t/dry-state-sync-go/data/home/config/config.toml"
check "dry run cgroup slice"    grep -q -- '--slice=benchnode.slice' "$t/dry-state-sync-go/plan"
check "dry run summary outcome" jq -e '.outcome == "dry_run" and .campaign == "state-sync"' "$t/dry-state-sync-go/attrs/summary"
check "dry run rs bridge flags" grep -q -- '--bridge-rpc-listen 127.0.0.1:26658 --bridge-p2p-listen' "$t/dry-bridge-rs/plan"
check "dry run node pinned"     grep -q 'clone -q --depth 1 --branch v0.36.0 .*celestia-node' "$t/dry-bridge-go/plan"
check "dry run go bridge"       grep -q 'celestia bridge start .*--core.port 9090' "$t/dry-bridge-go/plan"
check "dry run DA floor"        grep -q 'SyncFromHeight = 16200001' "$t/dry-bridge-go/bridge-config.toml"
check "dry run bridge workload" jq -e '.workload == "share.GetShare(row=0,col=0) at tail and every 20 blocks" and .bridge == null' "$t/dry-bridge-go/attrs/summary"
check "dry run no bridge on consensus runs" bash -c "! grep -q -- '--bridge-' '$t/dry-state-sync-rs/plan'"
rc=0
env BENCH_DRY_RUN="$t/dry-bad" BENCH_META_campaign='state-sync;reboot' BENCH_META_role=go \
  bash "$here/vm.sh" >/dev/null 2>&1 || rc=$?
check "dry run rejects bad metadata" test "$rc" -ne 0

check "dry run enables bbr"     grep -q 'sysctl -w net.core.default_qdisc=fq net.ipv4.tcp_congestion_control=bbr' "$t/dry-state-sync-go/plan"
check "dry run rs enables bbr"  grep -q 'tcp_congestion_control=bbr' "$t/dry-state-sync-rs/plan"

# The ERR trap must not run in subshells: a failing command substitution or
# process substitution (a poll before the RPC is up, a tip not yet reached)
# once ran finish there and stopped the node.
mkdir -p "$t/trap"
rc=0
# shellcheck disable=SC2016
env BENCH_DRY_RUN="$t/trap" bash -c '
  . "$1/vm.sh"
  finish() { touch "$STATE/finish-ran"; }
  units=() height="" phase=sync reason="" outcome=""
  arm_err_trap
  printf "1.0 - 100 -\n" >"$STATE/polls.tsv"
  at_tip && exit 3
  if st=$(rpc http://127.0.0.1:1/status); then exit 4; fi
  exit 0' _ "$here" >/dev/null 2>&1 || rc=$?
check "trap quiet in subshells" test "$rc" -eq 0
check "trap did not run finish" test ! -e "$t/trap/finish-ran"
rc=0
# shellcheck disable=SC2016
env BENCH_DRY_RUN="$t/trap" bash -c '
  . "$1/vm.sh"
  finish() { touch "$STATE/finish-main"; }
  units=() height="" phase=sync reason="" outcome=""
  arm_err_trap
  false
  exit 0' _ "$here" >/dev/null 2>&1 || rc=$?
check "trap fires in main shell" test "$rc" -eq 1 -a -e "$t/trap/finish-main"

# GCE may start the metadata script concurrently. Exactly one process must
# claim the detached driver; a check followed by touch is racy here.
driver_claim_fixture() (
  STATE=$t/driver-claim
  mkdir -p "$STATE"
  for _ in {1..20}; do (bench_claim_driver "$STATE" && echo won) & done
  wait
)
driver_claim_fixture >"$t/claims"
eq "one startup driver wins" 1 "$(wc -l <"$t/claims")"

# Exercise the fixed-height runner, including JSON-RPC share validation and
# the head/tail checks. The mock advances on WaitForHeight like a live bridge.
bridge_fixture() (
  # shellcheck source=scripts/bench/vm.sh
  . "$here/vm.sh"
  STATE=$t/fixture-$1
  mkdir -p "$STATE"
  local fault=$1
  role=go bridge_tail=900 bridge_start=1000 t0=1 deadline=4102444800 da_token=fixture
  local fixture_share
  fixture_share=$(head -c 512 /dev/zero | base64 -w0)
  echo 999 >"$STATE/head"
  alive() { return 0; }
  poll() { return 0; }
  da_sample() { echo '1 100 20 30 40 50 60' >>"$STATE/da-memory.tsv"; }
  da_rpc() {
    local h
    case $1 in
      header.WaitForHeight)
        h=${2:1:-1}
        echo "$h" >"$STATE/head"
        printf '{"header":{"height":"%d"}}\n' "$h" ;;
      das.SamplingStats)
        read -r h <"$STATE/head"
        printf '{"is_running":true,"head_of_sampled_chain":%d}' "$h" ;;
      header.LocalHead)
        read -r h <"$STATE/head"
        if [[ $fault == late && $h == "$bridge_start" ]]; then h=$((h + 1)); fi
        printf '{"header":{"height":"%d"}}\n' "$h" ;;
      header.Tail)
        read -r h <"$STATE/head"
        local tail=$bridge_tail
        if [[ $fault == pruned && $h -gt $bridge_start ]]; then tail=$((tail + 1)); fi
        printf '{"header":{"height":"%d"}}\n' "$tail" ;;
      share.GetShare)
        if [[ $fault == malformed && $2 == '[1000,0,0]' ]]; then echo null; else jq -n --arg s "$fixture_share" '$s'; fi ;;
      *) return 1 ;;
    esac
  }
  da_window || return 1
  jq -e '.tail==900 and .start==1000 and .end==1600 and .requests==32' "$STATE/bridge.json" &&
    [[ $(awk '{print $1}' "$STATE/da-reads.tsv") == "$(printf '%s\n' 900; seq 1000 20 1600)" ]]
)
check "bridge serves same 32 reads across 600 heights" bridge_fixture matched
refute "bridge rejects missed start" bridge_fixture late
refute "bridge rejects tail moving in window" bridge_fixture pruned
refute "bridge rejects malformed share" bridge_fixture malformed

# --- block-range campaign: a fixed mainnet block window on both sides ---------
check  "campaign block-range"   bench_valid campaign block-range
check  "blocks ok"              bench_valid blocks 1000
refute "blocks zero"            bench_valid blocks 0
refute "blocks too many"        bench_valid blocks 100000
check  "disk pd"                bench_valid disk pd
check  "disk nvme"              bench_valid disk nvme
refute "disk other"             bench_valid disk 'nvme;id'

# range-memory.tsv: uptime_s local_h mem_current_B rss_KiB cpu_usec anon_B file_B disk_used_B
printf '%s\n' '# uptime_s local_h mem_current_B rss_KiB cpu_usec anon_B file_B disk_used_B' \
  '100.0 1000 400 40 5000000 1 2 900' '105.0 1002 600 60 6000000 1 2 950' \
  '110.0 - - - - - - -' '115.0 1006 500 50 7000000 1 2 980' '120.5 1010 300 30 8500000 1 2 990' >"$t/range.tsv"
eq "range stats" '{"entry_h":1000,"exit_h":1010,"elapsed_s":20.5,"cpu_usec":3500000,"samples":4,"mem_peak_B":600,"mem_median_B":450,"rss_peak_KiB":60,"rss_median_KiB":45,"disk_used_peak_B":990}' \
  "$(bench_range_stats "$t/range.tsv")"
refute "range stats empty" bench_range_stats /dev/null

# Pair verdicts for the block-range campaign.
jq -n --arg hash "$(printf 'A%.0s' {1..64})" '{
  role:"rs", campaign:"block-range", outcome:"ok", snapshot:"16200000",
  verify:[{height:"16200099", match:true, chain_id:"celestia", local_block_id:$hash, reference_block_id:$hash}],
  range:{scheduled_start:16201000, blocks:1000, entry_h:16201000, exit_h:16202000, final_app_height:16202000}
}' >"$rs"
jq '.role = "go"' "$rs" >"$go"
eq "block range same height" valid "$(pair block-range)"
jq '.range.final_app_height = 16202003' "$go" >"$t/higher"
eq "block range unequal final heights" "discard:different_final_heights:rs=16202000,go=16202003,diff=-3" \
  "$(bench_pair_verdict block-range "$rs" "$t/higher" || true)"
refute "block range unequal exits nonzero" bench_pair_verdict block-range "$rs" "$t/higher"
jq '.range.scheduled_start = 16200500' "$go" >"$t/other"
eq "block range different schedule" discard:different_range "$(bench_pair_verdict block-range "$rs" "$t/other" || true)"
for invalid in 'del(.range)' '.range.exit_h = 16201999' '.range.final_app_height = null' '.range.entry_h = 16200999'; do
  jq "$invalid" "$go" >"$t/incomplete"
  eq "incomplete range ($invalid)" discard:incomplete_range "$(bench_pair_verdict block-range "$rs" "$t/incomplete" || true)"
done
jq '.outcome = "failed" | .reason = "missed_range_start"' "$go" >"$t/failed"
eq "block range failed side" discard:incomplete_run "$(bench_pair_verdict block-range "$rs" "$t/failed" || true)"

block_range_dry() { # block_range_dry ROLE DIR [ENV...]
  local role=$1 dir=$2; shift 2
  env BENCH_DRY_RUN="$dir" BENCH_META_campaign=block-range BENCH_META_role="$role" \
    BENCH_META_height=10 BENCH_META_rust_commit=58be8ae90d00518800df5506b3a1e988f009c1c6 \
    BENCH_META_range_start=16205000 BENCH_META_range_blocks=1000 \
    BENCH_META_trust_height=16200000 BENCH_META_trust_hash="$(printf 'A%.0s' {1..64})" \
    BENCH_META_peers="$peer" BENCH_META_deadline=4102444800 "$@" bash "$here/vm.sh"
}
for r in rs go; do
  out=$(block_range_dry "$r" "$t/dry-block-range-$r" 2>&1) && rc=0 || rc=$?
  eq "dry run block-range $r" 0 "$rc"
  [[ $rc == 0 ]] || printf '%s\n' "$out" | sed 's/^/     | /'
  check "dry run block-range $r halt height" grep -qx 'halt-height = 16206001' "$t/dry-block-range-$r/data/home/config/app.toml"
  check "dry run block-range $r summary" jq -e '.campaign == "block-range" and .outcome == "dry_run" and
    .range.scheduled_start == 16205000 and .range.blocks == 1000 and .range.halt_height == 16206001 and
    .host.disk_layout == "pd" and (.host | has("cpu_model") and has("gfni") and has("sha_ni") and has("vcpus") and has("mem_KiB"))' \
    "$t/dry-block-range-$r/attrs/summary"
  check "dry run block-range $r keeps unit after halt" grep -q -- '-p RemainAfterExit=yes' "$t/dry-block-range-$r/plan"
done
check "dry run block-range same app config" cmp "$t/dry-block-range-rs/data/home/config/app.toml" "$t/dry-block-range-go/data/home/config/app.toml"
check "dry run block-range rs halt flag" grep -q -- '--halt-height 16206001' "$t/dry-block-range-rs/plan"
check "dry run block-range rs state sync" grep -q -- '--statesync-trust-height 16200000' "$t/dry-block-range-rs/plan"
check "dry run other campaigns keep units default" bash -c "! grep -q RemainAfterExit '$t/dry-state-sync-rs/plan' '$t/dry-bridge-go/plan'"
check "dry run other campaigns no halt height" bash -c "! grep -q halt-height '$t/dry-state-sync-go/data/home/config/app.toml'"
check "dry run default data disk" grep -qx 'mkfs.ext4 -q -m 0 /dev/disk/by-id/google-data' "$t/dry-state-sync-go/plan"
rc=0
block_range_dry go "$t/dry-block-range-bad" BENCH_META_range_start=16199000 >/dev/null 2>&1 || rc=$?
check "dry run rejects range before trust height" test "$rc" -ne 0

# Local NVMe SSDs: one is formatted directly, several are striped.
mkdir -p "$t/nvme4" "$t/nvme1"
touch "$t/nvme4/google-local-nvme-ssd-"{0,1,2,3} "$t/nvme1/google-local-nvme-ssd-0"
block_range_dry rs "$t/dry-nvme4" BENCH_META_disk=nvme BENCH_NVME_DIR="$t/nvme4" >/dev/null 2>&1 && rc=0 || rc=$?
eq "dry run nvme raid" 0 "$rc"
check "nvme raid0 of four" grep -q "mdadm --create /dev/md0 --level=0 --raid-devices=4 --run $t/nvme4/google-local-nvme-ssd-0 " "$t/dry-nvme4/plan"
check "nvme raid formatted" grep -q 'mkfs.ext4 -q -m 0 -E lazy_itable_init=0,lazy_journal_init=0 /dev/md0' "$t/dry-nvme4/plan"
check "nvme layout recorded" jq -e '.host.disk_layout == "nvme-raid0:4"' "$t/dry-nvme4/attrs/summary"
block_range_dry go "$t/dry-nvme1" BENCH_META_disk=nvme BENCH_NVME_DIR="$t/nvme1" >/dev/null 2>&1 && rc=0 || rc=$?
eq "dry run one nvme" 0 "$rc"
check "one nvme formatted directly" grep -q "mkfs.ext4 -q -m 0 -E lazy_itable_init=0,lazy_journal_init=0 $t/nvme1/google-local-nvme-ssd-0" "$t/dry-nvme1/plan"
check "one nvme no raid" bash -c "! grep -q mdadm '$t/dry-nvme1/plan'"
mkdir -p "$t/nvme0"
rc=0
block_range_dry go "$t/dry-nvme0" BENCH_META_disk=nvme BENCH_NVME_DIR="$t/nvme0" >/dev/null 2>&1 || rc=$?
check "no nvme device fails" test "$rc" -ne 0

# The window runner: a scripted chain advancing one block per poll.
range_fixture() (
  # shellcheck source=scripts/bench/vm.sh
  . "$here/vm.sh"
  STATE=$t/range-$1
  mkdir -p "$STATE"
  local fault=$1
  range_start=1000 range_blocks=20 t0=0 deadline=4102444800 units=(bench-node) role=go
  echo 990 >"$STATE/h"
  [[ $fault != late ]] || echo 1001 >"$STATE/h"
  sleep() { :; }
  mono() { cat "$STATE/h"; }
  alive() { return 0; }
  status() { :; }
  poll() {
    local h; read -r h <"$STATE/h"
    height=$h
    printf '%s %s %s false 1 1 1 1 900 1\n' "$h" "$h" "$h" >>"$STATE/polls.tsv"
    # The node halts before executing block 1021 (halt-height 1021).
    ((h >= 1020)) || echo $((h + 1)) >"$STATE/h"
  }
  range_sample() { echo "$(mono) $height 100 10 $((height * 1000)) 1 1 500" >>"$STATE/range-memory.tsv"; }
  node_app_height() { # "exited": the node left at its halt height before its RPC could be read
    case $fault in runaway) echo 1025 ;; exited) return 1 ;; *) cat "$STATE/h" ;; esac
  }
  node_halt_log_height() { [[ $fault == exited ]] && echo 1021; }
  node_halted() { [[ $fault != runaway ]]; }
  run() { printf '%s\n' "$*" >>"$STATE/plan"; }
  height=$(cat "$STATE/h")
  range_run || return 1
  jq -e '.scheduled_start == 1000 and .blocks == 20 and .entry_h == 1000 and .exit_h == 1020 and
    .final_app_height == (if $f == "runaway" then 1025 else 1020 end) and .halted == ($f != "runaway") and
    .retained_earliest == 900 and .retained_latest == 1020' --arg f "$fault" "$STATE/range.json" &&
    [[ $(awk '!/^#/ {print $2}' "$STATE/range-memory.tsv" | head -1) == 1000 ]] &&
    grep -q 'systemctl stop bench-node' "$STATE/plan"
)
check  "range window from H to H+N" range_fixture matched
check  "range records a missing halt" range_fixture runaway
check  "range recovers the height of an exited node" range_fixture exited
refute "range rejects a late node" range_fixture late

# The local dispatcher and the report have their own tests.
bash "$here/cloud-test.sh" || fails=$((fails + 1))
python3 -m unittest discover -s "$here" -p 'test_*.py' || fails=$((fails + 1))

[[ $fails == 0 ]] || { echo "$fails failed"; exit 1; }
echo "all passed"
