#!/usr/bin/env bash
# Local dispatcher for the operator comparison (docs/benchmarks/operator-sync.md):
# the same VMs as .github/workflows/operator-benchmark.yml, created with your
# own gcloud credentials instead of GitHub Actions. Runs on the macOS system
# bash (3.2) and on Linux.
#
#   cloud.sh provision [--dry-run] [flags]   create the pairs (prints the price first)
#   cloud.sh status  --run-id ID              progress of this run's VMs
#   cloud.sh collect --run-id ID [--out DIR]  decode results and judge each pair
#   cloud.sh delete  --run-id ID              delete this run's VMs and leftover disks
#
# Every VM and disk is labelled owner=claude-blog-bench,bench-run=ID,campaign=C.
# status and delete match only instances that carry both owner and bench-run.
# Each VM deletes itself, with its disks, at --max-hours.
#
# Price per VM-hour (us-central1 on-demand list prices, read 2026-11-26 from
# cloud.google.com/products/compute/pricing/general-purpose,
# cloud.google.com/compute/disks-image-pricing and cloud.google.com/vpc/network-pricing):
# n2-highcpu-32 $1.147136/h (n2-standard-8 $0.388472/h); Local SSD
# $0.000109589/GiB-h (375 GiB each, counted as GiB); pd-balanced
# $0.000136986/GiB-h, pd-ssd $0.000232877/GiB-h, pd-standard $0.000054795/GiB-h;
# ephemeral IPv4 on a standard VM $0.005/h; Standard Tier internet egress
# $0.085/GiB (first paid tier), with egress capped on the VM at 4 Mbit/s
# (1.8 GB/h, counted as GiB): $0.153/h. Default VM (n2-highcpu-32, 4 local
# SSDs, 80 GB boot pd-balanced): $1.4805/h, so one pair for 4 h <= $11.84.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/bench/lib.sh
. "$here/lib.sh"

OWNER=claude-blog-bench
USD_LOCAL_SSD_GIB_H=0.000109589
USD_IPV4_H=0.005
USD_EGRESS_H=0.153 # 1.8 GiB/h at $0.085/GiB
BOOT_GB=80

die() { echo "cloud.sh: $*" >&2; exit 1; }
usd_machine_h() {
  case $1 in
    n2-standard-8) echo 0.388472 ;;
    n2-highcpu-16) echo 0.573568 ;;
    n2-highcpu-32) echo 1.147136 ;;
    n2-standard-32) echo 1.553888 ;;
    *) echo "${machine_usd_h:-}" ;;
  esac
}
usd_pd_gib_h() {
  case $1 in
    pd-balanced) echo 0.000136986 ;;
    pd-ssd) echo 0.000232877 ;;
    pd-standard) echo 0.000054795 ;;
  esac
}

# --- arguments ----------------------------------------------------------------
cmd=${1:-}
[[ $cmd =~ ^(provision|status|collect|delete)$ ]] || die "usage: cloud.sh provision|status|collect|delete [flags] (see the header)"
shift
project=fibreda zone=us-central1-a campaign=block-range pairs=1 machine=n2-highcpu-32
cpu_platform="Intel Ice Lake" local_ssds=4 data_gb='' data_type=pd-balanced rust_commit=''
blocks=1000 lead_min=60 lead_set='' max_hours=4 run_id='' max_usd=40 out='' dry=0 machine_usd_h=''
while (($#)); do
  case $1 in
    --dry-run) dry=1; shift; continue ;;
    --project | --zone | --campaign | --pairs | --machine-type | --min-cpu-platform | --local-ssds | --data-disk-gb | \
      --data-disk-type | --rust-commit | --blocks | --lead-min | --max-hours | --run-id | --max-usd | --out | --machine-usd-hour)
      (($# >= 2)) || die "$1 needs a value" ;;
    *) die "unknown flag $1" ;;
  esac
  case $1 in
    --project) project=$2 ;; --zone) zone=$2 ;; --campaign) campaign=$2 ;; --pairs) pairs=$2 ;;
    --machine-type) machine=$2 ;; --min-cpu-platform) cpu_platform=$2 ;; --local-ssds) local_ssds=$2 ;;
    --data-disk-gb) data_gb=$2 ;; --data-disk-type) data_type=$2 ;; --rust-commit) rust_commit=$2 ;;
    --blocks) blocks=$2 ;; --lead-min) lead_min=$2 lead_set=1 ;; --max-hours) max_hours=$2 ;;
    --run-id) run_id=$2 ;; --max-usd) max_usd=$2 ;; --out) out=$2 ;; --machine-usd-hour) machine_usd_h=$2 ;;
  esac
  shift 2
done

[[ $project =~ ^[a-z][a-z0-9-]{4,29}$ ]] || die "invalid --project"
[[ $zone =~ ^[a-z]+-[a-z]+[0-9]-[a-z]$ ]] || die "invalid --zone"
[[ $campaign =~ ^(state-sync|bridge|block-range)$ ]] || die "--campaign must be state-sync, bridge or block-range"
bench_valid pairs "$pairs" || die "--pairs must be 1-3"
if [[ $cmd == provision ]]; then
  run_id=${run_id:-$(date -u +%Y%m%d%H%M%S)}
else
  [[ -n $run_id ]] || die "$cmd needs --run-id"
fi
bench_valid run_id "$run_id" || die "--run-id must be digits"
filter="labels.bench-run=$run_id AND labels.owner=$OWNER"
labels="owner=$OWNER,bench-run=$run_id,campaign=$campaign"
names=()
for ((p = 1; p <= pairs; p++)); do names+=("bench-$campaign-$run_id-p$p-rs" "bench-$campaign-$run_id-p$p-go"); done
own_vm="^bench-(state-sync|bridge|block-range)-$run_id-p[1-3]-(rs|go)$"
own_disk="^bench-(state-sync|bridge|block-range)-$run_id-p[1-3]-(rs|go)(-data)?$"

shell_quote() { # one argument, single-quoted unless it is plainly safe (same on bash 3.2 and 5)
  if [[ $1 =~ ^[A-Za-z0-9_=,./:@+-]+$ ]]; then printf '%s' "$1"; else printf "'%s'" "$(printf '%s' "$1" | sed "s/'/'\\\\''/g")"; fi
}
gc() { # every cloud call; --dry-run prints it instead
  if ((dry)); then
    local a line=gcloud
    for a in "$@"; do line+=" $(shell_quote "$a")"; done
    echo "$line"
  else gcloud "$@"; fi
}

# --- provision ------------------------------------------------------------------
ref_get() { # ref_get INDEX PATH: a reference RPC response (CLOUD_FIXTURES=DIR replays recorded ones)
  if [[ -n ${CLOUD_FIXTURES:-} ]]; then
    local f=${2#/}
    f=${f/\?height=/-}
    cat "$CLOUD_FIXTURES/ref$1-$f"
  else
    curl -fsS -m 20 "${BENCH_REF_RPCS[$1]}$2"
  fi
}
block_time() { ref_get 0 "/block?height=$1" | jq -er '.result.block.header.time | sub("\\.[0-9]+"; "") | fromdateiso8601'; }

provision() {
  [[ -n $rust_commit ]] || rust_commit=$(git -C "$here" rev-parse HEAD)
  bench_valid commit "$rust_commit" || die "--rust-commit must be a full lowercase commit hash"
  bench_valid blocks "$blocks" || die "--blocks must be 1-9999"
  [[ $lead_min =~ ^[1-9][0-9]{0,2}$ ]] || die "invalid --lead-min"
  if [[ ! $max_hours =~ ^[1-9][0-9]?$ ]] || ((max_hours > 24)); then die "--max-hours must be 1-24"; fi
  [[ $max_usd =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "invalid --max-usd"
  [[ $machine =~ ^[a-z0-9]+-[a-z0-9-]+$ ]] || die "invalid --machine-type"
  [[ $cpu_platform =~ ^[A-Za-z0-9\ ]{0,40}$ ]] || die "invalid --min-cpu-platform"
  local disk
  if [[ -n $data_gb ]]; then
    [[ $data_gb =~ ^[1-9][0-9]{1,4}$ ]] || die "invalid --data-disk-gb"
    [[ $data_type =~ ^pd-(balanced|ssd|standard)$ ]] || die "--data-disk-type must be pd-balanced, pd-ssd or pd-standard"
    disk=pd local_ssds=0
  else
    [[ $local_ssds =~ ^(1|2|4|8|16|24)$ ]] || die "--local-ssds must be 1, 2, 4, 8, 16 or 24 (or use --data-disk-gb)"
    disk=nvme
  fi
  [[ $campaign != bridge || -n $lead_set ]] || lead_min=90 # the workflow's bridge lead time

  # Upper-bound price, before anything is created.
  local mh vm_h total n=${#names[@]}
  mh=$(usd_machine_h "$machine")
  [[ $mh =~ ^[0-9]+(\.[0-9]+)?$ ]] || die "no list price for $machine: pass --machine-usd-hour"
  vm_h=$(awk -v m="$mh" -v l="$local_ssds" -v ls="$USD_LOCAL_SSD_GIB_H" -v b="$BOOT_GB" -v pb="$(usd_pd_gib_h pd-balanced)" \
    -v d="${data_gb:-0}" -v pd="$(usd_pd_gib_h "$data_type")" -v ip="$USD_IPV4_H" -v e="$USD_EGRESS_H" \
    'BEGIN { printf "%.4f", m + l * 375 * ls + b * pb + d * pd + ip + e }')
  total=$(awk -v v="$vm_h" -v h="$max_hours" -v n="$n" 'BEGIN { printf "%.2f", v * h * n }')
  echo "upper-bound cost: \$$total for $n VM(s) x ${max_hours} h (\$$vm_h per VM-hour: $machine, $([[ $disk == pd ]] && echo "${data_gb} GB $data_type" || echo "$local_ssds local SSD"), ${BOOT_GB} GB boot, IPv4, egress cap)"
  awk -v t="$total" -v m="$max_usd" 'BEGIN { exit !(t <= m) }' || die "refusing: \$$total exceeds --max-usd $max_usd"

  # Identical trust inputs, scheduled window and peers for every VM.
  local latest trust_height trust_hash other span range_start='' bridge_start='' deadline now
  latest=$(ref_get 0 /status | jq -er '.result.sync_info.latest_block_height | tonumber')
  trust_height=$(((latest - 3000) / 1000 * 1000))
  trust_hash=$(ref_get 0 "/block?height=$trust_height" | jq -er .result.block_id.hash)
  other=$(ref_get 1 "/block?height=$trust_height" | jq -er .result.block_id.hash)
  if ! { bench_valid trust_height "$trust_height" && bench_valid trust_hash "$trust_hash"; }; then die "invalid trust inputs"; fi
  [[ $trust_hash == "$other" ]] || die "reference RPCs disagree on block $trust_height"
  if [[ $campaign != state-sync ]]; then
    span=$(($(block_time "$latest") - $(block_time "$((latest - 100))")))
    ((span > 0)) || die "could not estimate the block time"
    local ahead=$((latest + lead_min * 6000 / span))
    bench_valid height "$ahead" || die "invalid scheduled height"
    if [[ $campaign == bridge ]]; then bridge_start=$ahead; else range_start=$ahead; fi
    if [[ $campaign == block-range ]]; then
      # Lead time, the window and 15 minutes to halt, measure and publish
      # must end before the VM's publishing deadline.
      local need=$((lead_min * 60 + blocks * span / 100 + 900)) have=$((max_hours * 3600 - 2400))
      ((need <= have)) || die "window needs ~$((need / 60)) min but VMs publish after $((have / 60)) min: raise --max-hours or lower --blocks/--lead-min"
      echo "range: start $range_start (~$lead_min min ahead), $blocks blocks (~$((blocks * span / 6000)) min at $(awk -v s="$span" 'BEGIN { printf "%.2f", s / 100 }') s/block)"
    fi
  fi
  tmp=$(mktemp -d)
  trap 'rm -rf "$tmp"' EXIT
  ref_get 0 /net_info >"$tmp/net_info.json"
  local peers
  peers=$(bench_peers "$tmp/net_info.json" 20)
  bench_valid peers "$peers" || die "invalid peers"
  printf '%s' "$peers" >"$tmp/peers"
  now=${CLOUD_NOW:-$(date +%s)}
  deadline=$((now + max_hours * 3600 - 2400)) # publishing starts 40 minutes before deletion
  { echo '#!/usr/bin/env bash'; cat "$here/lib.sh" "$here/vm.sh"; } >"$tmp/startup.sh"
  (($(wc -c <"$tmp/startup.sh") < 262144)) || die "startup script exceeds the 256 KiB metadata limit"

  local md="block-project-ssh-keys=TRUE,enable-guest-attributes=TRUE,bench-campaign=$campaign,bench-height=10"
  md+=",bench-rust-commit=$rust_commit,bench-trust-height=$trust_height,bench-trust-hash=$trust_hash"
  [[ -z $bridge_start ]] || md+=",bench-bridge-start=$bridge_start"
  [[ -z $range_start ]] || md+=",bench-range-start=$range_start,bench-range-blocks=$blocks"
  md+=",bench-disk=$disk,bench-deadline=$deadline"
  local -a common=(--project="$project" --zone="$zone" --machine-type="$machine" --network-tier=STANDARD
    --image-family=ubuntu-2404-lts-amd64 --image-project=ubuntu-os-cloud
    --boot-disk-type=pd-balanced --boot-disk-size="${BOOT_GB}GB" --boot-disk-auto-delete
    --no-service-account --no-scopes --max-run-duration="${max_hours}h"
    --instance-termination-action=DELETE --no-restart-on-failure
    --metadata="$md" --metadata-from-file="startup-script=$tmp/startup.sh,bench-peers=$tmp/peers" --labels="$labels")
  [[ -z $cpu_platform ]] || common+=(--min-cpu-platform="$cpu_platform")
  local vm i disks=()
  if [[ $disk == nvme ]]; then
    local -a ssd=()
    for ((i = 0; i < local_ssds; i++)); do ssd+=(--local-ssd=interface=NVME); done
    gc compute instances create "${names[@]}" "${common[@]}" "${ssd[@]}"
    disks=("${names[@]}")
  else
    # A named data disk allows one instance per call.
    for vm in "${names[@]}"; do
      gc compute instances create "$vm" "${common[@]}" \
        --create-disk="device-name=data,name=$vm-data,size=${data_gb}GB,type=$data_type,auto-delete=yes"
      disks+=("$vm" "$vm-data")
    done
  fi
  for vm in "${disks[@]}"; do
    gc compute disks add-labels "$vm" --project="$project" --zone="$zone" --labels="$labels" ||
      echo "warning: could not label disk $vm (it is deleted with its VM)" >&2
  done
  local args="--run-id $run_id --campaign $campaign --pairs $pairs"
  [[ $project == fibreda && $zone == us-central1-a ]] || args+=" --project $project --zone $zone"
  cat <<EOF
Run $run_id: $campaign, $pairs pair(s), celestia-rs $rust_commit.
The VMs publish results from $(jq -nr --argjson t "$deadline" '$t | todate') and delete themselves at $(jq -nr --argjson t "$((deadline + 2400))" '$t | todate').
  bash scripts/bench/cloud.sh status  $args
  bash scripts/bench/cloud.sh collect $args
  python3 scripts/bench/report.py <collect output directory>
  bash scripts/bench/cloud.sh delete  $args
EOF
}

# --- status, collect, delete ------------------------------------------------------
status_cmd() {
  local listing name z state progress
  listing=$(gc compute instances list --project="$project" --filter="$filter" --format='value(name,zone.basename(),status)')
  ((dry == 0)) || { echo "$listing"; return 0; }
  [[ -n $listing ]] || { echo "no instances labelled bench-run=$run_id owner=$OWNER"; return 0; }
  while read -r name z state; do
    [[ $name =~ $own_vm ]] || continue
    progress=$(gc compute instances get-guest-attributes "$name" --project="$project" --zone="$z" \
      --query-path=bench/status --format='value(value)' 2>/dev/null || echo "(no status yet)")
    echo "$name $state $progress"
  done <<<"$listing"
}

collect_cmd() {
  out=${out:-${XDG_CACHE_HOME:-$HOME/.cache}/celestia-rs-bench/$run_id}
  mkdir -p "$out"
  local name d n i p verdict reasons rs go
  for name in "${names[@]}"; do
    d=$out/$name
    mkdir -p "$d"
    if ! gc compute instances get-guest-attributes "$name" --project="$project" --zone="$zone" \
      --query-path=bench/ --format=json >"$d/attributes.json"; then
      echo "$name: no guest attributes (VM gone or not booted)"
      continue
    fi
    ((dry == 0)) || continue
    attr() { jq -r --arg k "$1" '.[] | select(.key == $k) | .value' "$d/attributes.json"; }
    attr status >"$d/status"
    attr summary >"$d/summary.json"
    n=$(attr raw-count)
    if [[ $n =~ ^[1-8]$ ]]; then
      for ((i = 0; i < n; i++)); do attr "raw-$i" >"$d/raw.$i"; done
      bench_unchunk "$n" "$d/raw" >"$d/raw.tar" && rm -f "$d"/raw.[0-9]*
    fi
    echo "$name: $(cat "$d/status")"
  done
  ((dry == 0)) || return 0
  : >"$out/pairs.txt"
  for ((p = 1; p <= pairs; p++)); do
    rs=$out/bench-$campaign-$run_id-p$p-rs/summary.json go=$out/bench-$campaign-$run_id-p$p-go/summary.json
    verdict=$(bench_pair_verdict "$campaign" "$rs" "$go" || true)
    if [[ $verdict == discard:incomplete_run ]]; then
      reasons=$(jq -sr 'map("\(.role): \(.outcome) \(.reason // "")") | join("; ")' "$rs" "$go" 2>/dev/null || true)
      verdict+=" ($reasons)"
    fi
    echo "pair p$p: $verdict" | tee -a "$out/pairs.txt"
  done
  echo "results in $out"
}

delete_cmd() {
  local listing name vms=() disks=()
  listing=$(gc compute instances list --project="$project" --filter="$filter" --format='value(name)')
  if ((dry)); then
    echo "$listing"
    gc compute instances delete '<instances listed above matching '"$own_vm"'>' --project="$project" --zone="$zone" --quiet
    return 0
  fi
  while read -r name; do if [[ $name =~ $own_vm ]]; then vms+=("$name"); fi; done <<<"$listing"
  if ((${#vms[@]})); then
    gc compute instances delete "${vms[@]}" --project="$project" --zone="$zone" --quiet
  else
    echo "no instances labelled bench-run=$run_id owner=$OWNER"
  fi
  listing=$(gc compute disks list --project="$project" --filter="$filter" --format='value(name)')
  while read -r name; do if [[ $name =~ $own_disk ]]; then disks+=("$name"); fi; done <<<"$listing"
  ((${#disks[@]} == 0)) || gc compute disks delete "${disks[@]}" --project="$project" --zone="$zone" --quiet
  echo "deleted ${#vms[@]} instance(s) and ${#disks[@]} leftover disk(s) of run $run_id"
}

case $cmd in
  provision) provision ;;
  status) status_cmd ;;
  collect) collect_cmd ;;
  delete) delete_cmd ;;
esac
