#!/usr/bin/env bash
# Unattended VM-side run of one side of the operator sync comparison
# (docs/benchmarks/operator-sync.md). The operator-benchmark workflow passes
# lib.sh + this file as the startup script and the run inputs as bench-*
# instance metadata. The instance name ends in -rs or -go. Progress, the
# summary and compressed raw data go to guest attributes in namespace
# "bench", which the workflow's collect mode reads.
#
#   BENCH_DRY_RUN=DIR bash scripts/bench/vm.sh   # inputs from BENCH_META_<key>;
#   commands are written to DIR/plan and attributes to DIR/attrs instead.
set -Eeuo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/bench/lib.sh
declare -F bench_valid >/dev/null || . "$here/lib.sh"

DRY=${BENCH_DRY_RUN:-}
STATE=${DRY:-/var/lib/bench}
DATA=${DRY:+$DRY/data}
DATA=${DATA:-/data}
HOMEDIR=$DATA/home
MD=http://metadata.google.internal/computeMetadata/v1/instance
LOCAL=http://127.0.0.1:26657
SLICE=/sys/fs/cgroup/benchnode.slice
EGRESS_CAP=4mbit # bounds egress charges; see the price table in the workflow

meta() { # meta KEY: instance attribute bench-KEY (or BENCH_META_KEY in a dry run)
  if [[ -n $DRY ]]; then local v="BENCH_META_${1//-/_}"; printf '%s' "${!v:-}"; return; fi
  curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/attributes/bench-$1" 2>/dev/null || true
}
run() { if [[ -n $DRY ]]; then printf '%s\n' "$*" >>"$STATE/plan"; else "$@"; fi; }
as_bench() { run runuser -u bench -- env HOME="$DATA/userhome" "$@"; }
now() { date +%s; }
mono() { awk '{print $1}' /proc/uptime; }
put() { # put KEY FILE: one guest attribute; the API allows ~10 writes a minute
  if [[ -n $DRY ]]; then mkdir -p "$STATE/attrs" && cp "$2" "$STATE/attrs/$1"; return; fi
  curl -fsS -m 30 -X PUT --data-binary "@$2" -H Metadata-Flavor:Google \
    "$MD/guest-attributes/bench/$1" >/dev/null || echo "guest attribute $1 not written" >&2
  sleep 7
}
last_status=0
status() { # status PHASE [force]: progress line, published at most every 5 minutes
  phase=$1
  printf '%s %s height=%s\n' "$(date -u +%FT%TZ)" "$phase" "${height:-}" | tee -a "$STATE/status.log" >"$STATE/status"
  if [[ ${2:-} == force ]] || (($(now) - last_status >= 300)); then last_status=$(now); put status "$STATE/status"; fi
}

# --- inputs -----------------------------------------------------------------
read_inputs() {
  campaign=$(meta campaign); height_target=$(meta height); rust_commit=$(meta rust-commit)
  trust_height=$(meta trust-height); trust_hash=$(meta trust-hash); peers=$(meta peers); deadline=$(meta deadline)
  if [[ -n $DRY ]]; then role=$(meta role); name=bench-dry-$role; else
    name=$(curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/name"); role=${name##*-}
  fi
  if ! { bench_valid campaign "$campaign" && bench_valid role "$role" && bench_valid height "$height_target" &&
    bench_valid commit "$rust_commit" && bench_valid peers "$peers" && bench_valid deadline "$deadline"; }; then
    echo "invalid bench-* metadata" >&2; return 1
  fi
  if [[ $campaign != genesis-prefix ]] && ! { bench_valid trust_height "$trust_height" && bench_valid trust_hash "$trust_hash"; }; then
    echo "invalid trust inputs" >&2; return 1
  fi
  window_min=30
  if [[ $campaign == bridge ]]; then
    bridge_start=$(meta bridge-start)
    if ! bench_valid height "$bridge_start" || ((bridge_start <= trust_height)); then echo "invalid DA window start" >&2; return 1; fi
    window_min='' # bridge duration is defined in blocks
  fi
  if [[ $campaign == block-range ]]; then
    range_start=$(meta range-start); range_blocks=$(meta range-blocks)
    if ! { bench_valid height "$range_start" && bench_valid blocks "$range_blocks"; } || ((range_start <= trust_height)); then
      echo "invalid block range" >&2; return 1
    fi
    range_end_h=$((range_start + range_blocks))
    # Both implementations stop before executing this block: Go checkHalt and
    # celestia-rs --halt-height. The last committed height is range_end_h.
    halt_height=$((range_end_h + 1))
    window_min='' # defined in blocks
  fi
  disk_layout=$(meta disk)
  disk_layout=${disk_layout:-pd}
  bench_valid disk "$disk_layout" || { echo "invalid data disk layout" >&2; return 1; }
  outcome=running reason='' height='' t0='' tip_t='' tip_h=''
}

# --- setup and build (not timed) --------------------------------------------
setup() {
  status setup force
  run apt-get update -q
  run env DEBIAN_FRONTEND=noninteractive apt-get install -yq build-essential clang libclang-dev cmake \
    pkg-config git jq python3 curl wget ca-certificates
  local dev=/dev/disk/by-id/google-data
  if [[ $disk_layout == pd ]]; then
    run mkfs.ext4 -q -m 0 "$dev"
    disk_desc=pd
  else
    nvme_device || return 1
    dev=$nvme_dev
  fi
  run mkdir -p "$DATA" && run mount -o noatime "$dev" "$DATA"
  run useradd --system --home-dir "$DATA/userhome" bench
  mkdir -p "$HOMEDIR" "$DATA/userhome"
  run chown -R bench:bench "$DATA"
  local dev_if
  dev_if=$(ip route show default 2>/dev/null | awk '{print $5; exit}' || true)
  run tc qdisc replace dev "${dev_if:-ens4}" root tbf rate "$EGRESS_CAP" burst 64kb latency 400ms
  # celestia-app refuses to start without BBR. Both sides get it.
  run modprobe tcp_bbr
  run sysctl -w net.core.default_qdisc=fq net.ipv4.tcp_congestion_control=bbr
  host_info
}

# nvme_device: format the local NVMe SSDs as the data filesystem, striped
# (RAID 0) when there are several. Sets nvme_dev and disk_desc.
nvme_device() {
  local -a devs=()
  local d
  for d in "${BENCH_NVME_DIR:-/dev/disk/by-id}"/google-local-nvme-ssd-*; do [[ -e $d ]] && devs+=("$d"); done
  if ((${#devs[@]} == 0)); then reason=no_local_ssd; return 1; fi
  if ((${#devs[@]} == 1)); then
    nvme_dev=${devs[0]} disk_desc=nvme:1
  else
    run env DEBIAN_FRONTEND=noninteractive apt-get install -yq mdadm
    run mdadm --create /dev/md0 --level=0 --raid-devices="${#devs[@]}" --run "${devs[@]}"
    nvme_dev=/dev/md0 disk_desc=nvme-raid0:${#devs[@]}
  fi
  # Initialise inode tables now: lazy init would write in the background during the run.
  run mkfs.ext4 -q -m 0 -E lazy_itable_init=0,lazy_journal_init=0 "$nvme_dev"
}

# host_info: machine, CPU features and data disk layout for the summary.
host_info() {
  local flags model
  flags=$(grep -m1 '^flags' /proc/cpuinfo 2>/dev/null || true)
  model=$(grep -m1 '^model name' /proc/cpuinfo 2>/dev/null | sed 's/^[^:]*: *//' || true)
  jq -n --arg cpu_model "$model" --arg vcpus "$(nproc 2>/dev/null || true)" \
    --arg mem_KiB "$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null || true)" \
    --arg machine_type "$(curl -fsS -m 5 -H Metadata-Flavor:Google "$MD/machine-type" 2>/dev/null | sed 's,.*/,,' || true)" \
    --arg disk_layout "$disk_desc" \
    --arg data_fs_B "$(df -B1 --output=size "$DATA" 2>/dev/null | tail -1 | tr -d ' ' || true)" \
    --argjson gfni "$([[ $flags == *" gfni"* ]] && echo true || echo false)" \
    --argjson sha_ni "$([[ $flags == *" sha_ni"* ]] && echo true || echo false)" \
    --argjson avx512f "$([[ $flags == *" avx512f"* ]] && echo true || echo false)" \
    --argjson vaes "$([[ $flags == *" vaes"* ]] && echo true || echo false)" \
    '$ARGS.named' >"$STATE/host.json"
}

fetch_commit() { # fetch_commit URL COMMIT DIR
  run git init -q "$3" && run git -C "$3" fetch -q --depth 1 "$1" "$2" && run git -C "$3" checkout -q FETCH_HEAD
  [[ -n $DRY || $(git -C "$3" rev-parse HEAD) == "$2" ]]
}
fetch_tag() { # fetch_tag URL TAG COMMIT DIR: the tag must still name COMMIT
  run git clone -q --depth 1 --branch "$2" "$1" "$4"
  [[ -n $DRY || $(git -C "$4" rev-parse HEAD) == "$3" ]]
}

build() {
  status build force
  export HOME=/root PATH=/root/.cargo/bin:/usr/local/go/bin:$PATH
  local src=/opt/src
  fetch_commit https://github.com/celestiaorg/celestia-rs "$rust_commit" "$src/celestia-rs"
  if [[ $role == rs ]]; then
    run curl -fsSL --proto =https https://sh.rustup.rs -o /tmp/rustup-init.sh
    run sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain none
    # rust-toolchain.toml applies to the working directory.
    run env -C "$src/celestia-rs" cargo build -q --release --locked -p celestia-rs --features da-light
    run install -m 755 "$src/celestia-rs/target/release/celestia-rs" /usr/local/bin/celestia-rs
    bin=/usr/local/bin/celestia-rs
    toolchain=$(env -C "$src/celestia-rs" rustc -V 2>/dev/null || echo dry)
  else
    run curl -fsSLo /tmp/go.tgz "https://go.dev/dl/$BENCH_GOLANG.linux-amd64.tar.gz"
    [[ -n $DRY ]] || echo "$BENCH_GOLANG_SHA256  /tmp/go.tgz" | sha256sum -c --quiet
    run tar -C /usr/local -xzf /tmp/go.tgz
    fetch_tag https://github.com/celestiaorg/celestia-app "$BENCH_GO_TAG" "$BENCH_GO_COMMIT" "$src/celestia-app"
    run make -C "$src/celestia-app" build # multiplexer: embeds the v3-v10 binaries
    run install -m 755 "$src/celestia-app/build/celestia-appd" /usr/local/bin/celestia-appd
    bin=/usr/local/bin/celestia-appd
    toolchain=$BENCH_GOLANG
    if [[ $campaign == bridge ]]; then
      fetch_tag https://github.com/celestiaorg/celestia-node "$BENCH_NODE_TAG" "$BENCH_NODE_COMMIT" "$src/celestia-node"
      run make -C "$src/celestia-node" build
      run install -m 755 "$src/celestia-node/build/celestia" /usr/local/bin/celestia
      node_commit=$BENCH_NODE_COMMIT
      node_sha=$(sha256sum /usr/local/bin/celestia 2>/dev/null | cut -d' ' -f1 || echo dry)
    fi
  fi
  bin_sha=$(sha256sum "$bin" 2>/dev/null | cut -d' ' -f1 || echo dry)
  check_apphash=$src/celestia-rs/packaging/check-apphash.sh
  (($(now) < deadline)) || { reason=deadline_before_start; return 1; }
}

# --- node configuration (identical on both sides) --------------------------
configure() {
  status configure force
  run curl -fsSLo "$DATA/genesis.json" "$BENCH_GENESIS_URL"
  [[ -n $DRY ]] || echo "$BENCH_GENESIS_SHA256  $DATA/genesis.json" | sha256sum -c --quiet
  if [[ $role == rs ]]; then
    as_bench celestia-rs init bench --network mainnet --no-download --home "$HOMEDIR"
  else
    as_bench celestia-appd init bench --chain-id celestia --home "$HOMEDIR"
  fi
  mkdir -p "$HOMEDIR/config"
  [[ -z $DRY ]] || touch "$HOMEDIR/config/config.toml" "$HOMEDIR/config/app.toml"
  run install -o bench -g bench -m 644 "$DATA/genesis.json" "$HOMEDIR/config/genesis.json"
  local c=$HOMEDIR/config/config.toml a=$HOMEDIR/config/app.toml all_peers=$BENCH_PEERS${peers:+,$peers}
  bench_toml_set "$c" p2p seeds "\"$BENCH_SEEDS\""
  bench_toml_set "$c" p2p persistent_peers "\"$all_peers\""
  bench_toml_set "$c" tx_index indexer "\"${indexer:-null}\""
  bench_toml_set "$a" "" pruning '"custom"'
  bench_toml_set "$a" "" pruning-keep-recent '"100"'
  bench_toml_set "$a" "" pruning-interval '"10"'
  bench_toml_set "$a" "" min-retain-blocks 3000
  bench_toml_set "$a" state-sync snapshot-interval 0
  bench_toml_set "$a" grpc enable true # celestia-node reads blocks over gRPC
  bench_toml_set "$a" grpc address '"127.0.0.1:9090"'
  [[ $campaign != block-range ]] || bench_toml_set "$a" "" halt-height "$halt_height"
  rpcs="${BENCH_REF_RPCS[0]}:443,${BENCH_REF_RPCS[1]}:443"
  if [[ $campaign == genesis-prefix ]]; then
    bench_toml_set "$c" statesync enable false
  else
    bench_toml_set "$c" statesync enable true
    bench_toml_set "$c" statesync rpc_servers "\"$rpcs\""
    bench_toml_set "$c" statesync trust_height "$trust_height"
    bench_toml_set "$c" statesync trust_hash "\"$trust_hash\""
    bench_toml_set "$c" statesync trust_period '"168h0m0s"'
  fi
  if [[ $role == rs ]]; then
    node_cmd=(/usr/local/bin/celestia-rs start --home "$HOMEDIR")
    [[ $campaign == genesis-prefix ]] || node_cmd+=(--statesync-rpc-servers "$rpcs"
      --statesync-trust-height "$trust_height" --statesync-trust-hash "$trust_hash")
    [[ $campaign != block-range ]] || node_cmd+=(--halt-height "$halt_height")
  else
    node_cmd=(/usr/local/bin/celestia-appd start --home "$HOMEDIR")
  fi
  run cp "$c" "$a" "$STATE/"
}

start_unit() { # start_unit NAME CMD...: a dedicated service in benchnode.slice
  local unit=bench-$1; shift
  units+=("$unit")
  run systemd-run --unit="$unit" --slice=benchnode.slice --uid=bench --gid=bench \
    -p LimitNOFILE=65536 -p MemoryAccounting=yes -p CPUAccounting=yes -p WorkingDirectory="$DATA" \
    ${unit_props[@]+"${unit_props[@]}"} --setenv=HOME="$DATA/userhome" "$@"
}

# --- measurement -------------------------------------------------------------
rpc() { curl -fsS -m 4 "$1" 2>/dev/null; }
ref_height() {
  local r
  [[ $campaign != genesis-prefix ]] || { echo -; return; } # 1 s polls; the reference is not needed
  for r in "${BENCH_REF_RPCS[@]}"; do
    rpc "$r/status" | jq -er '.result.sync_info.latest_block_height' 2>/dev/null && return
  done
  echo -
}
rss_kib() {
  local f
  find "$SLICE" -name cgroup.procs -exec cat {} + 2>/dev/null | while read -r p; do
    awk '/^VmRSS:/ {print $2}' "/proc/$p/status" 2>/dev/null
  done | awk '{s += $1} END {print s + 0}'
}
POLL_COLUMNS="elapsed_s local_h ref_h catching_up mem_current_B rss_KiB disk_used_B cpu_usec earliest_h mem_peak_B"
poll() { # appends one line of POLL_COLUMNS; the peak survives a crashed unit
  [[ -s $STATE/polls.tsv ]] || echo "# $POLL_COLUMNS" >"$STATE/polls.tsv"
  local st lh=- cu=- er=-
  if st=$(rpc "$LOCAL/status"); then
    lh=$(jq -r '.result.sync_info.latest_block_height' <<<"$st"); cu=$(jq -r '.result.sync_info.catching_up' <<<"$st")
    er=$(jq -r '.result.sync_info.earliest_block_height' <<<"$st")
  fi
  [[ $lh =~ ^[0-9]+$ ]] || lh=-
  height=$lh
  printf '%.1f %s %s %s %s %s %s %s %s %s\n' "$(awk -v a="$(mono)" -v b="$t0" 'BEGIN{print a - b}')" "$lh" "$(ref_height)" "$cu" \
    "$(cat "$SLICE/memory.current" 2>/dev/null || echo -)" "$(rss_kib)" \
    "$(df -B1 --output=used "$DATA" | tail -1 | tr -d ' ')" \
    "$(awk '/^usage_usec/ {print $2}' "$SLICE/cpu.stat" 2>/dev/null || echo -)" "$er" \
    "$(cat "$SLICE/memory.peak" 2>/dev/null || echo -)" >>"$STATE/polls.tsv"
}
alive() { local u; for u in "${units[@]}"; do systemctl is-active --quiet "$u" || return 1; done; }
# measure INTERVAL STALL_S UNTIL_FN: poll until UNTIL_FN succeeds; fail on a
# stopped service, the deadline or no height change for STALL_S seconds.
measure() {
  local best=0 since=$SECONDS
  while :; do
    poll
    status "$phase"
    if [[ $height =~ ^[0-9]+$ ]] && ((height > best)); then best=$height since=$SECONDS; fi
    "$3" && return 0
    alive || { reason="service_exited"; return 1; }
    (($(now) < deadline)) || { reason="deadline"; return 1; }
    ((SECONDS - since < $2)) || { reason="stalled"; return 1; }
    sleep "$1"
  done
}
reached_h() { [[ $height =~ ^[0-9]+$ ]] && ((height >= height_target)); }
at_tip() { read -r tip_t tip_h < <(bench_tip <"$STATE/polls.tsv") && [[ -n $tip_t ]]; }
window_done() { (($(now) >= window_end)); }

run_genesis_prefix() {
  phase=sync; t0=$(mono)
  start_unit node "${node_cmd[@]}" || { reason=start_failed; return 1; }
  [[ -z $DRY ]] || return 0
  poll_s=1; measure 1 1800 reached_h || return 1
  # Comet commits H+1 only after checking header H+1's app hash against its
  # own state after H; wait for that, then compare H+1 with the reference.
  phase=verify; local _
  for _ in {1..60}; do poll; [[ $height =~ ^[0-9]+$ ]] && ((height > height_target)) && break; sleep 2; done
  local ours theirs
  ours=$(rpc "$LOCAL/block?height=$((height_target + 1))" | jq -r '.result.block.header.app_hash // empty')
  theirs=$(rpc "${BENCH_REF_RPCS[0]}/block?height=$((height_target + 1))" | jq -r '.result.block.header.app_hash // empty')
  jq -nc --arg o "$ours" --arg t "$theirs" '{header_height_checked: $ARGS.named.h, local_app_hash: $o, reference_app_hash: $t, match: ($o != "" and $o == $t)}' \
    --argjson h "$((height_target + 1))" >"$STATE/verify-h.json"
  bash "$check_apphash" "$LOCAL" "${BENCH_REF_RPCS[0]}" --json >"$STATE/verify-latest.json" || true
  jq -e .match "$STATE/verify-h.json" >/dev/null || { reason=app_hash_mismatch_or_unavailable; return 1; }
}

run_state_sync() {
  phase=sync; t0=$(mono)
  start_unit node "${node_cmd[@]}" || { reason=start_failed; return 1; }
  [[ -z $DRY ]] || return 0
  poll_s=5; measure 5 3600 at_tip || return 1
  phase=at-tip; verify_block_id || { reason=tip_block_id_mismatch_or_unavailable; return 1; }
  window_end=$(($(now) + window_min * 60))
  measure 5 600 window_done || return 1
}

# Both consensus nodes reach verified tip with DA disabled. Each bridge then
# indexes from the first retained block. A pair is comparable only if its
# snapshots, actual tails, window endpoints and read receipts all agree.
run_bridge() {
  phase=sync; t0=$(mono)
  start_unit node "${node_cmd[@]}" || { reason=start_failed; return 1; }
  if [[ -z $DRY ]]; then
    poll_s=5; measure 5 3600 at_tip || return 1
    verify_block_id || { reason=tip_block_id_mismatch_or_unavailable; return 1; }
    if ! bridge_tail=$(rpc "$LOCAL/status" | jq -er '.result.sync_info.earliest_block_height | tonumber') ||
      ! bench_valid height "$bridge_tail" || ((bridge_tail >= bridge_start)); then reason=invalid_bridge_base; return 1; fi
    # Same endpoint for the consensus-only state-sync memory comparison, before any DA service starts.
    if ! sync_peak=$(cat "$SLICE/memory.peak") || ! sync_rss=$(rss_kib); then reason=missing_sync_memory; return 1; fi
  else
    bridge_tail=$((trust_height + 1))
  fi
  if [[ $role == go ]]; then
    local net=(--p2p.network celestia --node.store "$DATA/bridge")
    as_bench celestia bridge init "${net[@]}" --core.ip 127.0.0.1 --core.port 9090 || { reason=bridge_init_failed; return 1; }
    local cfg=$DATA/bridge/config.toml
    if [[ -n $DRY ]]; then mkdir -p "$DATA/bridge"; printf '  [Header.Syncer]\n    SyncFromHeight = 0\n' >"$cfg"; fi
    sed -i -E "s/^([[:space:]]*)SyncFromHeight = [0-9]+$/\1SyncFromHeight = $bridge_tail/" "$cfg"
    # Reject unexpected encoder layout or an overriding hash in the pinned Go config.
    python3 -c 'import sys,tomllib; s=tomllib.load(open(sys.argv[1], "rb"))["Header"]["Syncer"]; sys.exit(s.get("SyncFromHeight") != int(sys.argv[2]) or bool(s.get("SyncFromHash")))' \
      "$cfg" "$bridge_tail" || { reason=bridge_config_failed; return 1; }
    cp "$cfg" "$STATE/bridge-config.toml"
    start_unit bridge /usr/local/bin/celestia bridge start "${net[@]}" --core.ip 127.0.0.1 --core.port 9090 \
      --rpc.addr 127.0.0.1 --rpc.port 26658 || { reason=bridge_start_failed; return 1; }
  else
    # Preserve the consensus-only measurements across restarting with DA enabled.
    if [[ -z $DRY ]]; then
      sync_unit_peak=$(cat "$SLICE/bench-node.service/memory.peak")
      sync_cpu=$(awk '/^usage_usec/ {print $2}' "$SLICE/cpu.stat")
      sync_slice=$(stat -c %i "$SLICE")
    fi
    run systemctl stop bench-node || { reason=stop_failed; return 1; }
    units=()
    start_unit stack "${node_cmd[@]}" --bridge-rpc-listen 127.0.0.1:26658 --bridge-p2p-listen /ip4/0.0.0.0/tcp/2121 || { reason=bridge_start_failed; return 1; }
    if [[ -z $DRY && $(stat -c %i "$SLICE") != "$sync_slice" ]]; then cpu_offset=$sync_cpu; fi
  fi
  [[ -z $DRY ]] || return 0
  phase=da-catchup; da_token=""
  if [[ $role == rs ]]; then
    token_cmd=(runuser -u bench -- celestia-rs da-auth read --home "$HOMEDIR")
  else
    token_cmd=(runuser -u bench -- env HOME="$DATA/userhome" celestia bridge auth read "${net[@]}")
  fi
  measure 5 600 da_ready || return 1
  da_window || return 1
}

da_rpc() { # METHOD JSON_PARAMS [TIMEOUT]; output must be a successful result
  curl -fsS -m "${3:-10}" http://127.0.0.1:26658 -H "Authorization: Bearer $da_token" -H 'Content-Type: application/json' \
    --data "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}" |
    jq -ce 'select(.id == 1 and (.error == null)) | .result'
}
da_height() { da_rpc "$@" | jq -er '.header.height | tonumber'; }
da_ready() {
  # Warm up before the common start. WaitForHeight then wakes at the exact block.
  [[ $height =~ ^[0-9]+$ ]] && ((height >= bridge_start - 30)) || return 1
  # Stop waiting if the consensus chain passes the scheduled window before DA catches up.
  if ((height > bridge_start + 5)); then return 0; fi
  da_token=$("${token_cmd[@]}") && [[ -n $da_token ]] || return 1
  local tail head
  tail=$(da_height header.Tail '[]') && head=$(da_height header.LocalHead '[]') || return 1
  echo "$height $tail $head" >>"$STATE/da-ready.tsv"
  [[ $tail == "$bridge_tail" ]] && ((head >= bridge_start - 30))
}
da_read() { # One request, no retries. Commit the canonical 512-byte share to the receipt.
  local h=$1 share
  share=$(da_rpc share.GetShare "[$h,0,0]" | jq -er 'select(type=="string" and length==684 and test("^[A-Za-z0-9+/]{682}[AEIMQUYcgkosw048]=$"))') || return 1
  printf '%s 0 0 %s\n' "$h" "$share" >>"$STATE/da-reads.tsv"
}
da_sample() {
  local current rss cpu anon file disk
  current=$(cat "$SLICE/memory.current") && rss=$(rss_kib) &&
    cpu=$(awk '/^usage_usec/ {print $2}' "$SLICE/cpu.stat") &&
    read -r anon file < <(awk '$1=="anon"{a=$2} $1=="file"{f=$2} END{print a,f}' "$SLICE/memory.stat") || return 1
  [[ $current =~ ^[0-9]+$ && $rss =~ ^[1-9][0-9]*$ && $cpu =~ ^[0-9]+$ && $anon =~ ^[0-9]+$ && $file =~ ^[0-9]+$ ]] || return 1
  disk=$(df -B1 --output=used "$DATA" | tail -1 | tr -d ' ') || return 1
  echo "$(mono) $current $rss $cpu $anon $file $disk" >>"$STATE/da-memory.tsv"
}
da_boundary() {
  local h=$1 waited head tail sampled=-
  waited=$(da_height header.WaitForHeight "[$h]" 180) && [[ $waited == "$h" ]] || return 1
  if [[ $role == go ]]; then
    # The header store can be at tip while the separate DASer is still filling
    # earlier EDSs. Require its contiguous sampled chain to cover the boundary.
    local until_s=$((SECONDS + 10))
    until sampled=$(da_rpc das.SamplingStats '[]' | jq -er --argjson h "$h" \
      'select(.is_running == true and (.failed // {} | length == 0)) | .head_of_sampled_chain | numbers | select(. >= $h)'); do
      ((SECONDS < until_s)) || return 1
      sleep 0.2
    done
  fi
  head=$(da_height header.LocalHead '[]') && tail=$(da_height header.Tail '[]') || return 1
  echo "$h $head $tail $sampled" >>"$STATE/da-boundaries.tsv"
  [[ $head == "$h" && $tail == "$bridge_tail" ]]
}
da_workload() {
  local h waited stop=$((bridge_start + BENCH_BRIDGE_HEIGHTS))
  da_read "$bridge_tail" && da_read "$bridge_start" || return 1
  for ((h = bridge_start + BENCH_BRIDGE_STRIDE; h < stop; h += BENCH_BRIDGE_STRIDE)); do
    (($(now) < deadline)) && alive || return 1
    waited=$(da_height header.WaitForHeight "[$h]" 120) && [[ $waited == "$h" ]] || return 1
    da_read "$h" || return 1
  done
  da_boundary "$stop" && da_read "$stop"
}
da_window() {
  local sampler rc=0 count digest stop=$((bridge_start + BENCH_BRIDGE_HEIGHTS))
  if [[ -z $da_token ]] || ! da_boundary "$bridge_start"; then reason=missed_da_window_start; return 1; fi

  phase=bridge-window; window_start_s=$(awk -v a="$(mono)" -v b="$t0" 'BEGIN{print a-b}')
  echo '# uptime_s mem_current_B rss_KiB cpu_usec anon_B file_B disk_used_B' >"$STATE/da-memory.tsv"
  da_sample || { reason=missing_da_memory; return 1; }
  (while sleep 5; do da_sample || exit 1; done) & sampler=$!
  da_workload || rc=1
  kill "$sampler" 2>/dev/null || rc=1
  wait "$sampler" 2>/dev/null || true
  da_sample || rc=1
  ((rc == 0)) || { reason=incomplete_da_window; return 1; }
  count=$(wc -l <"$STATE/da-reads.tsv")
  digest=$(sha256sum "$STATE/da-reads.tsv" | cut -d' ' -f1)
  jq -n --argjson tail "$bridge_tail" --argjson start "$bridge_start" --argjson end "$stop" \
    --argjson requests "$count" --arg digest "$digest" '$ARGS.named' >"$STATE/bridge.json"
  # Preserve the endpoint poll separately from the exact DA boundary receipt.
  poll
}

# Both consensus nodes reach verified tip as in state-sync, then wait for a
# common scheduled height H, measure the window H..H+N and halt before
# executing H+N+1. Disk is measured after a graceful stop at that height.
run_block_range() {
  phase=sync; t0=$(mono)
  unit_props=(-p RemainAfterExit=yes) # a clean halt leaves the unit active (exited), not gone
  start_unit node "${node_cmd[@]}" || { reason=start_failed; return 1; }
  if [[ -n $DRY ]]; then range_json; return 0; fi
  poll_s=5; measure 5 3600 at_tip || return 1
  phase=at-tip; verify_block_id || { reason=tip_block_id_mismatch_or_unavailable; return 1; }
  range_run
}

range_run() {
  range_end_h=$((range_start + range_blocks)) halt_height=$((range_start + range_blocks + 1))
  # A node not at tip before H is a recorded failure, not an extrapolation.
  if [[ ! $height =~ ^[0-9]+$ ]] || ((height >= range_start)); then reason=missed_range_start; return 1; fi
  phase=pre-range; range_loop range_entered || return 1
  phase=range; range_begin; range_loop range_finished || return 1
  range_exit_t=$(mono) range_exit_h=$height
  local last u
  last=$(awk '$2 ~ /^[0-9]+$/ {l = $0} END {print l}' "$STATE/polls.tsv")
  retained_latest=$(awk '{print $2}' <<<"$last") retained_earliest=$(awk '{print $9}' <<<"$last")
  range_slice_peak=$(cat "$SLICE/memory.peak" 2>/dev/null || true)
  range_halt
  phase=stopped
  for u in "${units[@]}"; do run systemctl stop "$u" || true; done
  sync || true
  du -sB1 "$HOMEDIR" "$HOMEDIR"/data/* 2>/dev/null >"$STATE/range-du.txt" || true
  home_bytes=$(awk 'NR == 1 {print $1}' "$STATE/range-du.txt")
  fs_used=$(df -B1 --output=used "$DATA" 2>/dev/null | tail -1 | tr -d ' ' || true)
  range_json
}
range_entered() { [[ $height =~ ^[0-9]+$ ]] && ((height >= range_start)); }
range_finished() { [[ $height =~ ^[0-9]+$ ]] && ((height >= range_end_h)); }
range_interval() { # 1 s within three blocks of either boundary, so entry and exit land on H and H+N
  if [[ $height =~ ^[0-9]+$ ]] && { ((height >= range_start - 3 && height <= range_start)) || ((height >= range_end_h - 3)); }; then
    echo 1
  else echo 5; fi
}
# range_loop UNTIL_FN: like measure, with the boundary poll interval and a
# memory/CPU/disk sample on every poll inside the window.
range_loop() {
  local best=0 since=$SECONDS
  while :; do
    poll
    status "$phase"
    [[ $phase != range ]] || range_sample || true
    if [[ $height =~ ^[0-9]+$ ]] && ((height > best)); then best=$height since=$SECONDS; fi
    "$1" && return 0
    alive || { reason=service_exited; return 1; }
    (($(now) < deadline)) || { reason=deadline; return 1; }
    ((SECONDS - since < 600)) || { reason=stalled; return 1; }
    sleep "$(range_interval)"
  done
}
range_begin() {
  range_t0=$(mono) range_entry_h=$height
  echo '# uptime_s local_h mem_current_B rss_KiB cpu_usec anon_B file_B disk_used_B' >"$STATE/range-memory.tsv"
  range_sample || true
}
range_sample() {
  local current rss cpu anon file disk
  current=$(cat "$SLICE/memory.current" 2>/dev/null) || current=-
  rss=$(rss_kib) || rss=-
  cpu=$(awk '/^usage_usec/ {print $2}' "$SLICE/cpu.stat" 2>/dev/null) || cpu=-
  read -r anon file < <(awk '$1=="anon"{a=$2} $1=="file"{f=$2} END{print a+0,f+0}' "$SLICE/memory.stat" 2>/dev/null) || anon=- file=-
  disk=$(df -B1 --output=used "$DATA" 2>/dev/null | tail -1 | tr -d ' ') || disk=-
  echo "$(mono) ${height:--} ${current:--} ${rss:--} ${cpu:--} ${anon:--} ${file:--} ${disk:--}" >>"$STATE/range-memory.tsv"
}
node_app_height() { rpc "$LOCAL/abci_info" | jq -er '.result.response.last_block_height | tonumber' 2>/dev/null; }
node_halted() { # celestia-rs exits cleanly at its halt height; Go stays up after "CONSENSUS FAILURE"
  local u=${units[0]}
  [[ $(systemctl show -p SubState --value "$u" 2>/dev/null) != running ]] && return 0
  journalctl -u "$u" --no-pager -n 3000 2>/dev/null | grep -q 'halt per configuration'
}
# range_halt: wait up to three minutes for the configured halt, recording the
# last committed app height. Without a halt the node is stopped anyway and
# its actual final height is reported.
range_halt() {
  phase=halt; final_app_h='' halted=false
  local h until_s=$((SECONDS + 180))
  while ((SECONDS < until_s)); do
    poll
    if h=$(node_app_height) && [[ $h =~ ^[0-9]+$ ]]; then final_app_h=$h; fi
    if node_halted; then halted=true; break; fi
    if [[ $final_app_h =~ ^[0-9]+$ ]] && ((final_app_h > halt_height)); then break; fi
    sleep 1
  done
  final_status_h=$(awk '$2 ~ /^[0-9]+$/ {h = $2} END {print h}' "$STATE/polls.tsv")
  echo "$final_app_h $final_status_h $halted" >"$STATE/range-halt.txt"
}
range_json() { # range.json: the scheduled window, its statistics and the end state
  local stats='{}'
  [[ ! -s $STATE/range-memory.tsv ]] || stats=$(bench_range_stats "$STATE/range-memory.tsv") || stats='{}'
  jq -n --argjson stats "$stats" --argjson scheduled_start "$range_start" --argjson blocks "$range_blocks" \
    --argjson halt_height "$halt_height" --arg entry_h "${range_entry_h:-}" --arg exit_h "${range_exit_h:-}" \
    --arg elapsed_s "$(awk -v a="${range_exit_t:-}" -v b="${range_t0:-}" 'BEGIN { if (a != "" && b != "") printf "%.1f", a - b }')" \
    --arg final_app "${final_app_h:-}" --arg final_status "${final_status_h:-}" \
    --arg earliest "${retained_earliest:-}" --arg latest "${retained_latest:-}" --arg home "${home_bytes:-}" \
    --arg fs_used "${fs_used:-}" --arg slice_peak "${range_slice_peak:-}" --arg halted "${halted:-}" \
    --arg du "$(cat "$STATE/range-du.txt" 2>/dev/null)" '
    def num: if test("^[0-9]+(\\.[0-9]+)?$") then tonumber else null end;
    $stats + {scheduled_start: $scheduled_start, blocks: $blocks, scheduled_end: ($scheduled_start + $blocks), halt_height: $halt_height,
      entry_h: ($entry_h | num), exit_h: ($exit_h | num), elapsed_s: ($elapsed_s | num),
      final_app_height: ($final_app | num), final_status_height: ($final_status | num),
      halted: (if $halted == "" then null else $halted == "true" end),
      retained_earliest: ($earliest | num), retained_latest: ($latest | num),
      home_bytes: ($home | num), fs_used_after_stop_B: ($fs_used | num), slice_peak_at_exit_B: ($slice_peak | num), du: $du}' \
    >"$STATE/range.json"
}

verify_block_id() { # chain ID and block ID at the local height against the reference
  local h ours theirs chain
  h=$(rpc "$LOCAL/status" | jq -r .result.sync_info.latest_block_height)
  chain=$(rpc "$LOCAL/status" | jq -r .result.node_info.network)
  ours=$(rpc "$LOCAL/block?height=$h" | jq -r '.result.block_id.hash // empty')
  theirs=$(rpc "${BENCH_REF_RPCS[0]}/block?height=$h" | jq -r '.result.block_id.hash // empty')
  jq -nc --arg c "$chain" --arg o "$ours" --arg t "$theirs" --arg h "$h" \
    '{height: $h, chain_id: $c, local_block_id: $o, reference_block_id: $t, match: ($c == "celestia" and $o != "" and $o == $t)}' >"$STATE/verify-tip.json"
  jq -e .match "$STATE/verify-tip.json" >/dev/null
}

# --- results ----------------------------------------------------------------
# Poll-derived milestones (resolution: the poll interval). CometBFT serves RPC
# only after its ABCI handshake, so rpc_up_s includes InitChain on Go.
timings() {
  awk -v h="$height_target" '$2 !~ /^[0-9]+$/ { next }
    !u { printf "rpc_up_s=%s ", $1; u = 1 }
    $2 >= h && !t { printf "target_s=%s target_seen=%s ", $1, $2; t = 1 }
    f && $2 > f && !a { printf "first_advance_s=%s ", $1; a = 1 }
    $2 >= 1 && !f { printf "first_height_s=%s first_height=%s ", $1, $2; f = $2 }' "$STATE/polls.tsv" 2>/dev/null || true
}

finish() {
  trap - ERR
  local peak="{}" u
  for u in "${units[@]}"; do
    peak=$(jq -c --arg u "$u" --arg v "$(cat "$SLICE/$u.service/memory.peak" 2>/dev/null)" '. + {($u): $v}' <<<"$peak")
  done
  local slice_peak cpu
  slice_peak=$(cat "$SLICE/memory.peak" 2>/dev/null || true)
  cpu=$(awk '/^usage_usec/ {print $2}' "$SLICE/cpu.stat" 2>/dev/null || true)
  local -a log_units=("${units[@]}")
  if [[ -n ${sync_unit_peak:-} ]]; then
    log_units+=(bench-node)
    peak=$(jq -c --arg v "$sync_unit_peak" '. + {"bench-node":$v}' <<<"$peak")
    ((sync_peak <= ${slice_peak:-0})) || slice_peak=$sync_peak
    cpu=$((${cpu:-0} + ${cpu_offset:-0}))
  fi
  for u in "${units[@]}"; do run systemctl stop "$u" || true; done
  [[ -n $DRY ]] || for u in "${log_units[@]}"; do journalctl -u "$u" -o short-iso-precise --no-pager >"$STATE/$u.log" || true; done
  local snap
  snap=$(grep -hoE 'Snapshot restored.*height[=: "]+[0-9]+' "$STATE"/bench-*.log 2>/dev/null | grep -oE '[0-9]+$' | head -1 || true)
  du -sB1 "$HOMEDIR" "$HOMEDIR"/data/* "$DATA"/bridge 2>/dev/null >"$STATE/du.txt" || true
  local rss_max disk_max mem_max
  rss_max=$(bench_max "$STATE/polls.tsv" 6 2>/dev/null || true)
  disk_max=$(bench_max "$STATE/polls.tsv" 7 2>/dev/null || true)
  mem_max=$(bench_max "$STATE/polls.tsv" 5 2>/dev/null || true)
  if [[ -s $STATE/da-memory.tsv ]]; then
    rss_max=$(printf '%s\n' "$rss_max" "$(bench_max "$STATE/da-memory.tsv" 3)" | sort -nr | head -1)
    disk_max=$(printf '%s\n' "$disk_max" "$(bench_max "$STATE/da-memory.tsv" 7)" | sort -nr | head -1)
    mem_max=$(printf '%s\n' "$mem_max" "$(bench_max "$STATE/da-memory.tsv" 2)" | sort -nr | head -1)
  fi
  jq -n --arg campaign "$campaign" --arg role "$role" --arg name "$name" --arg outcome "$outcome" --arg reason "$reason" \
    --arg last_height "$(awk '$2 ~ /^[0-9]+$/ {h = $2} END {print h}' "$STATE/polls.tsv" 2>/dev/null)" \
    --arg target "$height_target" --arg tip_t "$tip_t" --arg tip_h "$tip_h" --arg snapshot "$snap" \
    --arg rust_commit "$rust_commit" --arg go_commit "$BENCH_GO_COMMIT" --arg node_commit "${node_commit:-}" \
    --arg bin_sha "${bin_sha:-}" --arg node_sha "${node_sha:-}" --arg toolchain "${toolchain:-}" \
    --arg trust_height "$trust_height" --arg trust_hash "$trust_hash" --arg egress "$EGRESS_CAP" \
    --arg sync_peak_B "${sync_peak:-}" --arg sync_tip_rss_KiB "${sync_rss:-}" \
    --arg slice_peak "$slice_peak" --argjson unit_peaks "$peak" --arg cpu_usec "$cpu" \
    --arg rss_peak "$rss_max" --arg mem_max "$mem_max" \
    --arg disk_hwm "$disk_max" --arg end "$(tail -1 "$STATE/polls.tsv" 2>/dev/null)" \
    --arg du "$(cat "$STATE/du.txt")" --arg window "${window_min:-}" --arg window_start_s "${window_start_s:-}" \
    --slurpfile bridge <(cat "$STATE/bridge.json" 2>/dev/null) \
    --arg workload "$([[ $campaign == bridge ]] && echo "share.GetShare(row=0,col=0) at tail and every $BENCH_BRIDGE_STRIDE blocks")" --arg node_tag "$BENCH_NODE_TAG" \
    --arg machine "$(curl -fsS -m 5 -H Metadata-Flavor:Google "$MD/machine-type" 2>/dev/null | sed 's,.*/,,')" \
    --arg kernel "$(uname -r)" --arg cpus "$(nproc)" --arg mem "$(awk '/MemTotal/ {print $2}' /proc/meminfo)" \
    --slurpfile verify <(cat "$STATE"/verify-*.json 2>/dev/null) \
    --arg poll_columns "$POLL_COLUMNS" --arg poll_interval_s "$poll_s" --arg timings "$(timings)" \
    --slurpfile range <(cat "$STATE/range.json" 2>/dev/null) --slurpfile host <(cat "$STATE/host.json" 2>/dev/null) \
    --arg mem_peak_polled "$(bench_max "$STATE/polls.tsv" 10 2>/dev/null)" \
    '$ARGS.named + {verify: $verify, bridge: $bridge[0], range: $range[0], host: $host[0]}' \
    >"$STATE/summary.json"
  # Guest attributes kept only about 500 KB for each VM in the 2026-11-25
  # runs: chunks after the second were dropped. Two chunks is the budget.
  local RAW_MAX_CHUNKS=2
  # Publish everything if it fits. If not, publish log excerpts in place of
  # the full node logs, and as the last resort the polls alone.
  local n i f
  for f in "$STATE"/bench-*.log; do
    if [[ -e $f ]]; then bench_log_excerpt <"$f" >"${f%.log}.excerpt" || true; fi
  done
  pack() { tar -C "$STATE" -cf "$STATE/raw.tar" --exclude=raw.tar --exclude=attrs --exclude='raw.*' "$@" . 2>/dev/null || true; }
  pack --exclude='*.excerpt'
  n=$(bench_chunk "$STATE/raw.tar" "$STATE/raw" 196608 "$RAW_MAX_CHUNKS") || {
    pack --exclude='bench-*.log'
    n=$(bench_chunk "$STATE/raw.tar" "$STATE/raw" 196608 "$RAW_MAX_CHUNKS")
  } || {
    tail -c 1000000 "$STATE/polls.tsv" >"$STATE/polls.tail.tsv"
    local evidence=(polls.tail.tsv summary.json)
    for f in da-reads.tsv da-memory.tsv da-boundaries.tsv bridge-config.toml range-memory.tsv range.json range-du.txt; do [[ ! -f $STATE/$f ]] || evidence+=("$f"); done
    tar -C "$STATE" -cf "$STATE/raw.tar" "${evidence[@]}"
    n=$(bench_chunk "$STATE/raw.tar" "$STATE/raw" 196608 "$RAW_MAX_CHUNKS")
  }
  put summary "$STATE/summary.json"
  for ((i = 0; i < n; i++)); do put "raw-$i" "$STATE/raw.$i"; done
  printf '%s' "$n" >"$STATE/raw-count" && put raw-count "$STATE/raw-count"
  status "done-$outcome" force
}

# With set -E, subshells inherit the ERR trap. Only the main shell may act
# on it: a failing command or process substitution (a poll before the RPC
# is up, a tip not yet reached) must not run finish and stop the node.
arm_err_trap() {
  main_pid=$BASHPID
  trap '[[ $BASHPID != "$main_pid" ]] || { reason=${reason:-"error in $phase at line $LINENO"}; outcome=failed; finish; exit 1; }' ERR
}

main() {
  mkdir -p "$STATE"
  if [[ -z $DRY && ${BENCH_DETACHED:-} != 1 ]]; then # the startup script re-runs on boot
    bench_claim_driver "$STATE" || exit 0
    curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/attributes/startup-script" >"$STATE/run.sh"
    exec systemd-run --unit=bench-driver --setenv=BENCH_DETACHED=1 bash "$STATE/run.sh"
  fi
  read_inputs
  units=() height='' phase=start poll_s=''
  arm_err_trap
  setup; build; configure
  if "run_${campaign//-/_}"; then outcome=${DRY:+dry_run}; outcome=${outcome:-ok}; else outcome=failed; fi
  finish
  [[ $outcome != failed ]]
}
# Run unless sourced (the guest agent may also run this through bash -c).
if [[ -z ${BASH_SOURCE[0]:-} || ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
