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
  [[ $campaign != bridge ]] || window_min=60
  outcome=running reason='' height='' t0='' tip_t='' tip_h=''
}

# --- setup and build (not timed) --------------------------------------------
setup() {
  status setup force
  run apt-get update -q
  run env DEBIAN_FRONTEND=noninteractive apt-get install -yq build-essential clang libclang-dev cmake \
    pkg-config git jq curl wget ca-certificates
  local dev=/dev/disk/by-id/google-data
  run mkfs.ext4 -q -m 0 "$dev"
  run mkdir -p "$DATA" && run mount -o noatime "$dev" "$DATA"
  run useradd --system --home-dir "$DATA/userhome" bench
  mkdir -p "$HOMEDIR" "$DATA/userhome"
  run chown -R bench:bench "$DATA"
  local dev_if
  dev_if=$(ip route show default 2>/dev/null | awk '{print $5; exit}' || true)
  run tc qdisc replace dev "${dev_if:-ens4}" root tbf rate "$EGRESS_CAP" burst 64kb latency 400ms
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
    # The integrated bridge with its default 169 h retention, as celestia-node's.
    [[ $campaign != bridge ]] || node_cmd+=(--bridge-rpc-listen 127.0.0.1:26658 --bridge-p2p-listen /ip4/0.0.0.0/tcp/2121)
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
    --setenv=HOME="$DATA/userhome" "$@"
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

# Bridge: state sync to tip, then (Go) start celestia-node; measure the stack
# for the window under live mainnet traffic, probing the local DA head each
# minute. No client workload is generated.
run_bridge() {
  phase=sync; t0=$(mono)
  start_unit node "${node_cmd[@]}" || { reason=start_failed; return 1; }
  local net=(--p2p.network celestia --node.store "$DATA/bridge")
  if [[ -z $DRY ]]; then poll_s=5; measure 5 3600 at_tip || return 1; verify_block_id || { reason=tip_block_id_mismatch_or_unavailable; return 1; }; fi
  if [[ $role == go ]]; then
    as_bench celestia bridge init "${net[@]}" --core.ip 127.0.0.1 --core.port 9090 || { reason=bridge_init_failed; return 1; }
    start_unit bridge /usr/local/bin/celestia bridge start "${net[@]}" --core.ip 127.0.0.1 --core.port 9090 \
      --rpc.addr 127.0.0.1 --rpc.port 26658 || { reason=bridge_start_failed; return 1; }
  fi
  [[ -z $DRY ]] || return 0
  phase=bridge-window; window_start_s=$(awk -v a="$(mono)" -v b="$t0" 'BEGIN{print a - b}')
  window_end=$(($(now) + window_min * 60)) next_probe=0
  measure 5 600 da_window_done || return 1
}
da_window_done() {
  if (($(now) >= next_probe)); then next_probe=$(($(now) + 60)); da_probe; fi
  window_done
}
da_probe() { # elapsed da_local_head consensus_height
  local tok h
  if [[ $role == rs ]]; then tok=$(runuser -u bench -- celestia-rs da-auth read --home "$HOMEDIR" 2>/dev/null)
  else tok=$(runuser -u bench -- env HOME="$DATA/userhome" celestia bridge auth read --p2p.network celestia --node.store "$DATA/bridge" 2>/dev/null); fi
  h=$(curl -fsS -m 10 http://127.0.0.1:26658 -H "Authorization: Bearer $tok" -H 'Content-Type: application/json' \
    -d '{"jsonrpc":"2.0","id":1,"method":"header.LocalHead","params":[]}' 2>/dev/null | jq -r '.result.header.height // "-"' 2>/dev/null)
  echo "$(awk -v a="$(mono)" -v b="$t0" 'BEGIN{print a - b}') ${h:--} $height" >>"$STATE/da.tsv"
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
  for u in "${units[@]}"; do run systemctl stop "$u" || true; done
  [[ -n $DRY ]] || for u in "${units[@]}"; do journalctl -u "$u" -o short-iso-precise --no-pager >"$STATE/$u.log" || true; done
  local snap
  snap=$(grep -hoE 'Snapshot restored.*height[=: "]+[0-9]+' "$STATE"/bench-*.log 2>/dev/null | grep -oE '[0-9]+$' | head -1 || true)
  du -sB1 "$HOMEDIR" "$HOMEDIR"/data/* "$DATA"/bridge 2>/dev/null >"$STATE/du.txt" || true
  jq -n --arg campaign "$campaign" --arg role "$role" --arg name "$name" --arg outcome "$outcome" --arg reason "$reason" \
    --arg last_height "$(awk '$2 ~ /^[0-9]+$/ {h = $2} END {print h}' "$STATE/polls.tsv" 2>/dev/null)" \
    --arg target "$height_target" --arg tip_t "$tip_t" --arg tip_h "$tip_h" --arg snapshot "$snap" \
    --arg rust_commit "$rust_commit" --arg go_commit "$BENCH_GO_COMMIT" --arg node_commit "${node_commit:-}" \
    --arg bin_sha "${bin_sha:-}" --arg node_sha "${node_sha:-}" --arg toolchain "${toolchain:-}" \
    --arg trust_height "$trust_height" --arg trust_hash "$trust_hash" --arg egress "$EGRESS_CAP" \
    --arg slice_peak "$slice_peak" --argjson unit_peaks "$peak" --arg cpu_usec "$cpu" \
    --arg rss_peak "$(bench_max "$STATE/polls.tsv" 6 2>/dev/null)" --arg mem_max "$(bench_max "$STATE/polls.tsv" 5 2>/dev/null)" \
    --arg disk_hwm "$(bench_max "$STATE/polls.tsv" 7 2>/dev/null)" --arg end "$(tail -1 "$STATE/polls.tsv" 2>/dev/null)" \
    --arg du "$(cat "$STATE/du.txt")" --arg window "${window_min:-}" --arg window_start_s "${window_start_s:-}" \
    --arg workload "$([[ $campaign == bridge ]] && echo 'live mainnet traffic only')" --arg node_tag "$BENCH_NODE_TAG" \
    --arg machine "$(curl -fsS -m 5 -H Metadata-Flavor:Google "$MD/machine-type" 2>/dev/null | sed 's,.*/,,')" \
    --arg kernel "$(uname -r)" --arg cpus "$(nproc)" --arg mem "$(awk '/MemTotal/ {print $2}' /proc/meminfo)" \
    --slurpfile verify <(cat "$STATE"/verify-*.json 2>/dev/null) \
    --arg poll_columns "$POLL_COLUMNS" --arg poll_interval_s "$poll_s" --arg timings "$(timings)" \
    --arg mem_peak_polled "$(bench_max "$STATE/polls.tsv" 10 2>/dev/null)" '$ARGS.named + {verify: $verify}' \
    >"$STATE/summary.json"
  tar -C "$STATE" -cf "$STATE/raw.tar" --exclude=raw.tar --exclude=attrs --exclude='raw.*' . 2>/dev/null || true
  local n i
  n=$(bench_chunk "$STATE/raw.tar" "$STATE/raw" 196608 8) || { tail -c 1000000 "$STATE/polls.tsv" >"$STATE/raw.tar"; n=$(bench_chunk "$STATE/raw.tar" "$STATE/raw" 196608 8); }
  put summary "$STATE/summary.json"
  for ((i = 0; i < n; i++)); do put "raw-$i" "$STATE/raw.$i"; done
  printf '%s' "$n" >"$STATE/raw-count" && put raw-count "$STATE/raw-count"
  status "done-$outcome" force
}

main() {
  mkdir -p "$STATE"
  if [[ -z $DRY && ${BENCH_DETACHED:-} != 1 ]]; then # the startup script re-runs on boot
    [[ ! -e $STATE/started ]] || exit 0
    touch "$STATE/started"
    curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/attributes/startup-script" >"$STATE/run.sh"
    exec systemd-run --unit=bench-driver --setenv=BENCH_DETACHED=1 bash "$STATE/run.sh"
  fi
  read_inputs
  units=() height='' phase=start poll_s=''
  trap 'reason=${reason:-"error in $phase at line $LINENO"}; outcome=failed; finish; exit 1' ERR
  setup; build; configure
  if "run_${campaign//-/_}"; then outcome=${DRY:+dry_run}; outcome=${outcome:-ok}; else outcome=failed; fi
  finish
  [[ $outcome != failed ]]
}
# Run unless sourced (the guest agent may also run this through bash -c).
if [[ -z ${BASH_SOURCE[0]:-} || ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
