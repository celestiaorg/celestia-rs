#!/usr/bin/env bash
# Tests scripts/bench/cloud.sh against recorded RPC responses and a fake
# gcloud on PATH. No cloud or network access. Runs from test.sh and on the
# macOS system bash (3.2), where operators run cloud.sh.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=scripts/bench/lib.sh
. "$here/lib.sh"
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0
check() { local name=$1; shift; if "$@" >/dev/null 2>&1; then echo "ok   $name"; else echo "FAIL $name"; fails=$((fails + 1)); fi; }
refute() { local name=$1; shift; if "$@" >/dev/null 2>&1; then echo "FAIL $name"; fails=$((fails + 1)); else echo "ok   $name"; fi; }

# Recorded reference RPC responses: tip 16205000, 100 blocks in 600 s.
fx=$t/fixtures
mkdir -p "$fx" "$t/bin"
hash=$(printf 'B%.0s' {1..64})
for i in 0 1; do
  echo '{"result":{"sync_info":{"latest_block_height":"16205000"}}}' >"$fx/ref$i-status"
  printf '{"result":{"block_id":{"hash":"%s"},"block":{"header":{"time":"2026-11-26T10:10:00.123456789Z"}}}}\n' "$hash" >"$fx/ref$i-block-16205000"
  printf '{"result":{"block_id":{"hash":"%s"},"block":{"header":{"time":"2026-11-26T10:00:00.5Z"}}}}\n' "$hash" >"$fx/ref$i-block-16204900"
  printf '{"result":{"block_id":{"hash":"%s"}}}\n' "$hash" >"$fx/ref$i-block-16202000"
done
cat >"$fx/ref0-net_info" <<'EOF'
{"result":{"peers":[{"node_info":{"id":"0cd6b8188957d18e8326559ee59e86fad4bbc2c0","listen_addr":"tcp://0.0.0.0:26656"},"remote_ip":"170.101.101.70"}]}}
EOF

# Fake gcloud: logs every call; answers list and guest attribute reads.
cat >"$t/bin/gcloud" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$FAKE_GCLOUD_LOG"
case "$*" in
  *"instances list"*) cat "$FAKE_GCLOUD_DIR/list" 2>/dev/null ;;
  *"disks list"*) cat "$FAKE_GCLOUD_DIR/disks" 2>/dev/null ;;
  *"get-guest-attributes"*"--query-path=bench/status"*) echo "2026-11-26T12:00:00Z range height=16201500" ;;
  *"get-guest-attributes"*)
    for a in "$@"; do case $a in bench-*) vm=$a ;; esac; done
    cat "$FAKE_GCLOUD_DIR/$vm.json" 2>/dev/null || { echo "not found" >&2; exit 1; } ;;
esac
EOF
chmod +x "$t/bin/gcloud"
export PATH="$t/bin:$PATH" FAKE_GCLOUD_LOG=$t/gcloud.log FAKE_GCLOUD_DIR=$t/gc
mkdir -p "$t/gc"
commit=58be8ae90d00518800df5506b3a1e988f009c1c6
cloud() { env CLOUD_FIXTURES="$fx" CLOUD_NOW=1795000000 bash "$here/cloud.sh" "$@"; }

# --- provision --dry-run --------------------------------------------------------
: >"$t/gcloud.log"
cloud provision --dry-run --run-id 7 --rust-commit "$commit" >"$t/p.out" 2>&1 && rc=0 || rc=$?
check  "provision dry run succeeds" test "$rc" -eq 0
[[ $rc == 0 ]] || sed 's/^/     | /' "$t/p.out"
check  "dry run calls no gcloud"   test ! -s "$t/gcloud.log"
p() { grep -q -- "$1" "$t/p.out"; }
check  "creates both VMs of pair 1" p 'gcloud compute instances create bench-block-range-7-p1-rs bench-block-range-7-p1-go '
check  "machine type default"       p '--machine-type=n2-highcpu-32'
check  "cpu platform default"       p "'--min-cpu-platform=Intel Ice Lake'"
check  "four local nvme ssds"       test "$(grep -o -- '--local-ssd=interface=NVME' "$t/p.out" | wc -l | tr -d ' ')" -eq 4
check  "no service account"         p '--no-service-account --no-scopes'
check  "lifetime cap deletes"       p '--max-run-duration=4h --instance-termination-action=DELETE'
check  "labels"                     p '--labels=owner=claude-blog-bench,bench-run=7,campaign=block-range'
check  "disks labelled"             p 'gcloud compute disks add-labels bench-block-range-7-p1-go .*--labels=owner=claude-blog-bench,bench-run=7,campaign=block-range'
check  "guest attributes"           p 'enable-guest-attributes=TRUE'
check  "standard tier"              p '--network-tier=STANDARD'
check  "trust inputs"               p "bench-trust-height=16202000,bench-trust-hash=$hash"
# 60 minutes ahead at 6 s per block is 600 blocks.
check  "range start 60 min ahead"   p 'bench-range-start=16205600,bench-range-blocks=1000'
check  "nvme layout metadata"       p 'bench-disk=nvme'
check  "deadline 40 min before cap" p "bench-deadline=$((1795000000 + 4 * 3600 - 2400))"
check  "rust commit"                p "bench-rust-commit=$commit"
check  "startup script and peers"   p 'startup-script=.*,bench-peers='
check  "cost estimate printed"      p 'upper-bound cost: [$]11[.][0-9][0-9] for 2 VM'

cloud provision --dry-run --run-id 7 --rust-commit "$commit" --data-disk-gb 300 --data-disk-type pd-ssd >"$t/pd.out" 2>&1 && rc=0 || rc=$?
check  "persistent disk dry run"    test "$rc" -eq 0
check  "persistent data disk"       grep -q -- '--create-disk=device-name=data,name=bench-block-range-7-p1-rs-data,size=300GB,type=pd-ssd,auto-delete=yes' "$t/pd.out"
check  "pd layout metadata"         grep -q 'bench-disk=pd' "$t/pd.out"
check  "pd one VM per call"         grep -q 'instances create bench-block-range-7-p1-go --project' "$t/pd.out"
check  "pd data disk labelled"      grep -q 'disks add-labels bench-block-range-7-p1-rs-data ' "$t/pd.out"
refute "pd has no local ssd"        grep -q 'local-ssd' "$t/pd.out"

cloud provision --dry-run --run-id 7 --rust-commit "$commit" --pairs 3 >"$t/p3.out" 2>&1 && rc=0 || rc=$?
check  "three pairs fit default budget" test "$rc" -eq 0
check  "three pairs six VMs"        grep -q 'bench-block-range-7-p3-go ' "$t/p3.out"
refute "over budget refused"        cloud provision --dry-run --run-id 7 --rust-commit "$commit" --pairs 3 --max-usd 20
cloud provision --dry-run --run-id 7 --rust-commit "$commit" --pairs 3 --max-usd 20 >"$t/over.out" 2>&1 || true
refute "over budget creates nothing" grep -q 'instances create' "$t/over.out"
refute "window longer than lifetime" cloud provision --dry-run --run-id 7 --rust-commit "$commit" --blocks 3000 --max-hours 4
refute "bad commit"                 cloud provision --dry-run --run-id 7 --rust-commit abc
refute "bad run id"                 cloud provision --dry-run --run-id 'x;y' --rust-commit "$commit"
refute "bad pairs"                  cloud provision --dry-run --run-id 7 --rust-commit "$commit" --pairs 4
refute "unknown flag"               cloud provision --dry-run --bogus 1

cloud provision --dry-run --run-id 7 --rust-commit "$commit" --campaign state-sync --local-ssds 0 --data-disk-gb 200 >"$t/ss.out" 2>&1 && rc=0 || rc=$?
check  "state-sync dry run"         test "$rc" -eq 0
refute "state-sync no range"        grep -q 'bench-range-start=1' "$t/ss.out"

# --- status, delete: only this run's labelled instances -------------------------
printf '%s\n' 'bench-block-range-7-p1-rs us-central1-a RUNNING' 'bench-block-range-7-p1-go us-central1-a RUNNING' >"$t/gc/list"
: >"$t/gcloud.log"
cloud status --run-id 7 >"$t/s.out" 2>&1 && rc=0 || rc=$?
check  "status succeeds"            test "$rc" -eq 0
check  "status filters on run"      grep -q 'instances list .*--filter=labels.bench-run=7 AND labels.owner=claude-blog-bench' "$t/gcloud.log"
check  "status shows progress"      grep -q 'bench-block-range-7-p1-rs .*range height=16201500' "$t/s.out"
refute "status needs run id"        cloud status

printf '%s\n' bench-block-range-7-p1-rs bench-block-range-7-p1-go claude-genesis-1 bench-block-range-77-p1-rs >"$t/gc/list"
printf '%s\n' bench-block-range-7-p1-rs-data >"$t/gc/disks"
: >"$t/gcloud.log"
cloud delete --run-id 7 >"$t/d.out" 2>&1 && rc=0 || rc=$?
check  "delete succeeds"            test "$rc" -eq 0
check  "delete filters on run"      grep -q 'instances list .*--filter=labels.bench-run=7 AND labels.owner=claude-blog-bench' "$t/gcloud.log"
check  "delete this run's VMs"      grep -q 'instances delete bench-block-range-7-p1-rs bench-block-range-7-p1-go ' "$t/gcloud.log"
refute "delete never foreign VMs"   grep -qE 'delete .*(claude-genesis|-77-)' "$t/gcloud.log"
check  "delete leftover disks"      grep -q 'disks delete bench-block-range-7-p1-rs-data ' "$t/gcloud.log"
: >"$t/gcloud.log"
cloud delete --run-id 7 --dry-run >/dev/null 2>&1
refute "delete dry run deletes nothing" grep -q 'delete' "$t/gcloud.log"
refute "delete needs run id"        cloud delete

# --- collect: decode guest attributes and judge the pair ------------------------
mk_attrs() { # mk_attrs VM ROLE FINAL
  local d=$t/src-$1
  mkdir -p "$d/state"
  jq -n --arg role "$2" --argjson final "$3" --arg hash "$hash" '{role:$role, campaign:"block-range", outcome:"ok",
    snapshot:"16202000", tip_t:"300.0", verify:[{height:"16202100", match:true, chain_id:"celestia", local_block_id:$hash, reference_block_id:$hash}],
    range:{scheduled_start:16205600, blocks:1000, entry_h:16205600, exit_h:16206600, final_app_height:$final}}' >"$d/summary.json"
  echo polls >"$d/state/polls.tsv"
  tar -C "$d/state" -cf "$d/raw.tar" .
  bench_chunk "$d/raw.tar" "$d/raw" 196608 2 >/dev/null
  jq -n --rawfile s "$d/summary.json" --rawfile r "$d/raw.0" \
    '[{key:"status", value:"x done-ok height=1"}, {key:"summary", value:$s}, {key:"raw-count", value:"1"}, {key:"raw-0", value:$r}]' >"$t/gc/$1.json"
}
mk_attrs bench-block-range-7-p1-rs rs 16206600
mk_attrs bench-block-range-7-p1-go go 16206600
cloud collect --run-id 7 --out "$t/out" >"$t/c.out" 2>&1 && rc=0 || rc=$?
check  "collect succeeds"           test "$rc" -eq 0
check  "collect summary"            jq -e '.role == "go"' "$t/out/bench-block-range-7-p1-go/summary.json"
check  "collect raw decoded"        tar -tf "$t/out/bench-block-range-7-p1-rs/raw.tar"
check  "collect status"             grep -q done-ok "$t/out/bench-block-range-7-p1-rs/status"
check  "collect pair valid"         grep -qx 'pair p1: valid' "$t/out/pairs.txt"
mk_attrs bench-block-range-7-p1-go go 16206601
cloud collect --run-id 7 --out "$t/out2" >/dev/null 2>&1 || true
check  "collect reports height gap" grep -qx 'pair p1: discard:different_final_heights:rs=16206600,go=16206601,diff=-1' "$t/out2/pairs.txt"
rm "$t/gc/bench-block-range-7-p1-go.json"
cloud collect --run-id 7 --out "$t/out3" >/dev/null 2>&1 || true
check  "collect missing VM"         grep -q 'pair p1: discard:missing_or_invalid_summary' "$t/out3/pairs.txt"

[[ $fails == 0 ]] || { echo "cloud: $fails failed"; exit 1; }
echo "cloud: all passed"
