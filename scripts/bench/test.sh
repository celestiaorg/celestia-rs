#!/usr/bin/env bash
# Tests the pure helpers in scripts/bench/lib.sh and a dry run of vm.sh.
# Needs bash, jq, gzip, base64 and awk; no cloud access. CI runs it (ci.yml).
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

# --- chunking into guest-attribute-sized values -------------------------------
head -c 300000 /dev/urandom >"$t/raw"
n=$(bench_chunk "$t/raw" "$t/c" 100000 8)
eq     "chunk count"           5 "$n"   # 300000 bytes gzip-incompressible, base64 ~400 KB
check  "chunk size bound"      test "$(wc -c <"$t/c.0")" -le 100000
bench_unchunk "$n" "$t/c" >"$t/back"
check  "chunk round trip"      cmp "$t/raw" "$t/back"
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
      BENCH_META_trust_height=16200000 BENCH_META_trust_hash="$(printf 'A%.0s' {1..64})" \
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
check "dry run bridge workload" jq -e '.workload == "live mainnet traffic only" and .window == "60"' "$t/dry-bridge-go/attrs/summary"
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

[[ $fails == 0 ]] || { echo "$fails failed"; exit 1; }
echo "all passed"
