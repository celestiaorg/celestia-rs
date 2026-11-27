#!/usr/bin/env bash
# Unattended VM-side run of the Go mainnet checkpoint campaign
# (conformance/README.md, "Where checkpoints come from"). The
# genesis-checkpoints workflow passes lib.sh + checkpoints-lib.sh + this file
# as the startup script and the inputs as ckpt-* instance metadata.
#
# It block-syncs a celestia-app multiplexer node from the mainnet genesis on
# the retained data disk. For each planned height H it sets halt-height to
# H+1, waits until the node has committed H and refused H+1, exports a state
# sync snapshot of H with the embedded binary of H's app version, verifies it
# against the signed header H+1 and copies it with its metadata to the
# retained checkpoint disk. A rerun with the same disks resumes. Progress and
# results go to guest attributes in namespace "ckpt".
#
#   CKPT_DRY_RUN=DIR bash scripts/bench/checkpoints-vm.sh  # inputs from CKPT_META_<key>;
#   commands are written to DIR/plan and attributes to DIR/attrs instead.
set -Eeuo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/bench/lib.sh
declare -F bench_valid >/dev/null || . "$here/lib.sh"
# shellcheck source=scripts/bench/checkpoints-lib.sh
declare -F ckpt_valid >/dev/null || . "$here/checkpoints-lib.sh"

DRY=${CKPT_DRY_RUN:-}
RUN=${DRY:-/run/ckpt} # tmpfs: a reboot reruns the campaign, which resumes
DATA=${DRY:+$DRY/data}
DATA=${DATA:-/data} # retained data disk: node home, binaries, polls
CKPT=${DRY:+$DRY/ckpt}
CKPT=${CKPT:-/ckpt} # retained checkpoint disk: one directory per height
SRC=${DRY:+$DRY/src}
SRC=${SRC:-/opt/src}
# The replay fixture and verify-replay-checkpoint.py: the dispatching
# commit's on the VM, this checkout's in a dry run.
RS=${DRY:+$here/../..}
RS=${RS:-$SRC/celestia-rs}
STATE=$DATA/ckpt-state
HOMEDIR=$DATA/home
USERHOME=$DATA/userhome
MD=http://metadata.google.internal/computeMetadata/v1/instance
LOCAL=http://127.0.0.1:26657
UNIT=ckpt-node
POLL_S=30
STALL_S=7200 # no new height for two hours: no peer serves the next block

meta() { # meta KEY: instance attribute ckpt-KEY (or CKPT_META_KEY in a dry run)
  if [[ -n $DRY ]]; then local v="CKPT_META_${1//-/_}"; printf '%s' "${!v:-}"; return; fi
  curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/attributes/ckpt-$1" 2>/dev/null || true
}
run() { if [[ -n $DRY ]]; then printf '%s\n' "$*" >>"$RUN/plan"; else "$@"; fi; }
as_bench() { run runuser -u bench -- env HOME="$USERHOME" "$@"; }
now() { date +%s; }
put() { # put KEY FILE: one guest attribute; the API allows ~10 writes a minute
  if [[ -n $DRY ]]; then mkdir -p "$RUN/attrs" && cp "$2" "$RUN/attrs/$1"; return; fi
  curl -fsS -m 30 -X PUT --data-binary "@$2" -H Metadata-Flavor:Google \
    "$MD/guest-attributes/ckpt/$1" >/dev/null || echo "guest attribute $1 not written" >&2
  sleep 7
}
last_status=0
status() { # status PHASE [force]: progress line, published at most every 5 minutes
  phase=$1
  local next rate eta=-
  next=$(ckpt_next "$CKPT" "$heights" || echo none)
  rate=$(ckpt_rate "$STATE/polls.tsv" 3600 2>/dev/null || echo -)
  if [[ $next != none && $height =~ ^[0-9]+$ ]]; then eta=$(ckpt_eta "$rate" "$height" "$next"); fi
  printf '%s %s height=%s next=%s rate_1h=%s eta_s=%s done=%s/%s\n' "$(date -u +%FT%TZ)" "$phase" "${height:--}" \
    "$next" "$rate" "$eta" \
    "$(find "$CKPT" -mindepth 2 -maxdepth 2 -name checkpoint.json 2>/dev/null | wc -l)" "$(ckpt_plan "$heights" | wc -l)" |
    tee -a "$STATE/status.log" >"$RUN/status"
  if [[ ${2:-} == force ]] || (($(now) - last_status >= 300)); then last_status=$(now); put status "$RUN/status"; fi
}

# --- inputs -----------------------------------------------------------------
read_inputs() {
  name=$(meta name); heights=$(meta heights); rs_commit=$(meta rs-commit); run_id=$(meta run-id)
  peers=$(meta peers); deadline=$(meta deadline)
  if ! { ckpt_valid name "$name" && ckpt_valid heights "$heights" && bench_valid commit "$rs_commit" &&
    bench_valid run_id "$run_id" && bench_valid peers "$peers" && bench_valid deadline "$deadline"; }; then
    echo "invalid ckpt-* metadata" >&2; return 1
  fi
  outcome=running reason='' height=''
}

# --- disks, tools and build ---------------------------------------------------
mount_disk() { # mount_disk DEVICE_NAME DIR: format only a disk with no filesystem
  local dev=/dev/disk/by-id/google-$1
  mkdir -p "$2"
  [[ -z $DRY ]] || { run mount -o noatime "$dev" "$2"; return; }
  blkid -p "$dev" >/dev/null 2>&1 || mkfs.ext4 -q -m 0 "$dev"
  mountpoint -q "$2" || mount -o noatime "$dev" "$2" || mountpoint -q "$2" # another boot script may mount it first
}

setup() {
  mkdir -p "$RUN"
  # The first campaign failed when unattended-upgrades' needrestart restarted
  # this driver: the new driver failed on the apt lock and stopped the node.
  run systemctl disable --now unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer
  [[ -n $DRY ]] || { mkdir -p /etc/needrestart/conf.d && echo "\$nrconf{restart} = 'l';" >/etc/needrestart/conf.d/ckpt.conf; }
  run apt-get -o DPkg::Lock::Timeout=900 update -q
  run env DEBIAN_FRONTEND=noninteractive apt-get -o DPkg::Lock::Timeout=900 install -yq build-essential git jq curl wget ca-certificates python3
  mount_disk data "$DATA"
  mount_disk ckpt "$CKPT"
  mkdir -p "$STATE" "$HOMEDIR" "$USERHOME" "$DATA/bin"
  status setup force
  id bench >/dev/null 2>&1 || run useradd --system --home-dir "$USERHOME" bench
  run chown bench:bench "$DATA" "$HOMEDIR" "$USERHOME"
  local dev_if
  dev_if=$(ip route show default 2>/dev/null | awk '{print $5; exit}' || true)
  # Internet egress is capped; Cloud Storage (through the private.googleapis.com
  # VIP, free in region), the metadata server and the us-central1 VPC subnet
  # (peers that sync from this node) are not.
  dev_if=${dev_if:-ens4}
  run tc qdisc replace dev "$dev_if" root handle 1: htb default 20
  run tc class replace dev "$dev_if" parent 1: classid 1:10 htb rate 10gbit
  run tc class replace dev "$dev_if" parent 1: classid 1:20 htb rate "$CKPT_EGRESS_CAP" ceil "$CKPT_EGRESS_CAP"
  run tc filter replace dev "$dev_if" parent 1: protocol ip prio 1 handle 800::800 u32 match ip dst 199.36.153.8/30 flowid 1:10
  run tc filter replace dev "$dev_if" parent 1: protocol ip prio 1 handle 800::801 u32 match ip dst 169.254.169.254/32 flowid 1:10
  run tc filter replace dev "$dev_if" parent 1: protocol ip prio 1 handle 800::802 u32 match ip dst 10.128.0.0/20 flowid 1:10
  local hosts=${DRY:+$DRY/hosts}
  hosts=${hosts:-/etc/hosts}
  grep -q ' storage.googleapis.com$' "$hosts" 2>/dev/null || echo '199.36.153.8 storage.googleapis.com' >>"$hosts"
  run modprobe tcp_bbr # celestia-app refuses to start without BBR
  run sysctl -w net.core.default_qdisc=fq net.ipv4.tcp_congestion_control=bbr
}

build() {
  status build force
  export HOME=/root PATH=/usr/local/go/bin:$PATH
  # The replay fixture and verify-replay-checkpoint.py at the dispatching commit.
  rm -rf "$SRC/celestia-rs"
  run git init -q "$SRC/celestia-rs"
  run git -C "$SRC/celestia-rs" fetch -q --depth 1 https://github.com/celestiaorg/celestia-rs "$rs_commit"
  run git -C "$SRC/celestia-rs" checkout -q FETCH_HEAD
  [[ -n $DRY || $(git -C "$SRC/celestia-rs" rev-parse HEAD) == "$rs_commit" ]]
  # Every run of a campaign uses the binary its first run built.
  bin=$DATA/bin/celestia-appd-$BENCH_GO_COMMIT
  if [[ ! -x $bin ]]; then
    run curl -fsSLo /tmp/go.tgz "https://go.dev/dl/$BENCH_GOLANG.linux-amd64.tar.gz"
    [[ -n $DRY ]] || echo "$BENCH_GOLANG_SHA256  /tmp/go.tgz" | sha256sum -c --quiet
    run tar -C /usr/local -xzf /tmp/go.tgz
    rm -rf "$SRC/celestia-app"
    run git clone -q --depth 1 --branch "$BENCH_GO_TAG" https://github.com/celestiaorg/celestia-app "$SRC/celestia-app"
    [[ -n $DRY || $(git -C "$SRC/celestia-app" rev-parse HEAD) == "$BENCH_GO_COMMIT" ]]
    run make -C "$SRC/celestia-app" build # multiplexer: embeds the v3 to v10 binaries
    run install -m 755 "$SRC/celestia-app/build/celestia-appd" "$bin"
  fi
  bin_sha=$(sha256sum "$bin" 2>/dev/null | cut -d' ' -f1 || echo dry)
}

# --- node configuration -------------------------------------------------------
configure() {
  status configure force
  local c=$HOMEDIR/config/config.toml a=$HOMEDIR/config/app.toml
  if [[ ! -s $HOMEDIR/config/genesis.json ]]; then
    run curl -fsSLo "$DATA/genesis.json" "$BENCH_GENESIS_URL"
    [[ -n $DRY ]] || echo "$BENCH_GENESIS_SHA256  $DATA/genesis.json" | sha256sum -c --quiet
    as_bench "$bin" init checkpoints --chain-id celestia --home "$HOMEDIR"
    mkdir -p "$HOMEDIR/config"
    [[ -z $DRY ]] || touch "$c" "$a" "$HOMEDIR/config/genesis.json"
    run install -o bench -g bench -m 644 "$DATA/genesis.json" "$HOMEDIR/config/genesis.json"
  fi
  # Applied on every run, so a resumed run picks up changed peers.
  bench_toml_set "$c" p2p seeds "\"$BENCH_SEEDS\""
  bench_toml_set "$c" p2p persistent_peers "\"$CKPT_ARCHIVE_PEERS${peers:+,$peers}\""
  bench_toml_set "$c" p2p max_num_outbound_peers 20
  bench_toml_set "$c" p2p recv_rate 20971520 # do not cap downloads from archive peers
  bench_toml_set "$c" statesync enable false
  bench_toml_set "$c" tx_index indexer '"null"'
  bench_toml_set "$c" storage discard_abci_responses true
  # Pruned state, as the operator benchmark. Comet keeps the larger of
  # min-retain-blocks and the evidence age (120,961 to 559,940 blocks).
  bench_toml_set "$a" "" pruning '"custom"'
  bench_toml_set "$a" "" pruning-keep-recent '"100"'
  bench_toml_set "$a" "" pruning-interval '"10"'
  bench_toml_set "$a" "" min-retain-blocks 3000
  # The fast-node index only speeds up queries of the latest state; keeping it
  # costs a write per changed key. Snapshot export reads the trees.
  bench_toml_set "$a" "" iavl-disable-fastnode true
  bench_toml_set "$a" state-sync snapshot-interval 0
}

# --- sync to a checkpoint -----------------------------------------------------
rpc() { curl -fsS -m 4 "$1" 2>/dev/null; }
poll() { # appends "epoch_s height catching_up rx_B tx_B data_used_B" to the retained polls
  local st h=- cu=- dev_if
  if st=$(rpc "$LOCAL/status"); then
    h=$(jq -r '.result.sync_info.latest_block_height' <<<"$st" 2>/dev/null || echo -)
    cu=$(jq -r '.result.sync_info.catching_up' <<<"$st" 2>/dev/null || echo -)
  fi
  [[ $h =~ ^[0-9]+$ ]] || h=-
  [[ $h == - ]] || { height=$h; echo "$h" >"$STATE/last_height"; }
  dev_if=$(ip route show default 2>/dev/null | awk '{print $5; exit}' || true)
  printf '%s %s %s %s %s %s\n' "$(now)" "$h" "$cu" \
    "$(cat "/sys/class/net/${dev_if:-ens4}/statistics/rx_bytes" 2>/dev/null || echo -)" \
    "$(cat "/sys/class/net/${dev_if:-ens4}/statistics/tx_bytes" 2>/dev/null || echo -)" \
    "$(df -B1 --output=used "$DATA" | tail -1 | tr -d ' ')" >>"$STATE/polls.tsv"
}
node_log() { # the end of the node journal, where a halt or a crash is
  journalctl -u "$UNIT" -n 5000 -o short-iso-precise --no-pager >"$STATE/node.tail.log" 2>/dev/null || true
}
stop_node() {
  [[ -z $DRY ]] || return 0
  systemctl stop "$UNIT" 2>/dev/null || true
  node_log
}

# sync_to H: run the node until it has committed H and refused H+1.
sync_to() {
  local h=$1 best=0 since=$SECONDS
  [[ ! -e $STATE/halted-$h ]] || return 0 # a previous run halted here
  bench_toml_set "$HOMEDIR/config/app.toml" "" halt-height $((h + 1))
  if [[ -n $DRY ]]; then cp "$HOMEDIR/config/app.toml" "$RUN/plan.app.toml.$h"; return 0; fi
  systemctl reset-failed "$UNIT" 2>/dev/null || true
  systemd-run --unit="$UNIT" --uid=bench --gid=bench -p LimitNOFILE=65536 -p WorkingDirectory="$DATA" \
    --setenv=HOME="$USERHOME" "$bin" start --home "$HOMEDIR"
  while :; do
    sleep "$POLL_S"
    poll
    status "sync"
    if [[ $height =~ ^[0-9]+$ ]] && ((height > best)); then best=$height since=$SECONDS; fi
    # The multiplexer fails block H+1; block sync then panics and the
    # service exits. Look for the reason once the service is gone or at H.
    if ! systemctl is-active --quiet "$UNIT" || [[ $height == "$h" ]]; then
      node_log
      if ckpt_halted "$STATE/node.tail.log" "$h"; then
        stop_node; echo "$h" >"$STATE/last_height"; touch "$STATE/halted-$h"; height=$h; return 0
      fi
      systemctl is-active --quiet "$UNIT" || { reason=service_exited; return 1; }
    fi
    (($(now) < deadline)) || { reason=deadline; return 1; }
    ((SECONDS - since < STALL_S)) || { reason=stalled; return 1; }
  done
}

# --- export, verify and keep one checkpoint ----------------------------------
ref_hash() { # ref_hash PATH JQ: the value both reference archive RPCs agree on
  local r v first=''
  [[ -z $DRY ]] || { printf 'A%.0s' {1..64}; return; }
  for r in "${CKPT_REF_RPCS[@]}"; do
    v=$(rpc "$r/$1" | jq -r "$2") || return 1
    ckpt_valid hash "$v" || return 1
    [[ -z $first || $v == "$first" ]] || return 1
    first=$v
  done
  echo "$first"
}

drop_snapshots() { # drop_snapshots H: remove H from the node's own snapshot store
  local f
  for f in "$HOMEDIR/data/snapshots/$1"/*; do
    [[ -d $f ]] || continue
    as_bench "$bin" "${cmd[@]}" snapshots delete "$1" "${f##*/}" --home "$HOMEDIR" >/dev/null 2>&1 || true
    [[ -z $DRY ]] || rm -rf "$f"
  done
}

checkpoint() { # checkpoint H V
  local h=$1 v=$2 cmd fmt chunks out src dst tag emb_sha block_hash app_hash window verify_cmd vrc=0
  status export force
  read -r -a cmd <<<"$(ckpt_export_cmd "$h")"
  drop_snapshots "$h" # the SDK refuses to export a height its store already holds
  if [[ -n $DRY ]]; then
    run "$bin" "${cmd[@]}" snapshots export --height "$h" --home "$HOMEDIR"
    fmt=$(ckpt_format "$v")
    mkdir -p "$HOMEDIR/data/snapshots/$h/$fmt" && printf 'chunk' >"$HOMEDIR/data/snapshots/$h/$fmt/0"
    out="Snapshot created at height $h, format $fmt, chunks 1"
  else
    out=$(runuser -u bench -- env HOME="$USERHOME" "$bin" "${cmd[@]}" snapshots export --height "$h" --home "$HOMEDIR" 2>&1) ||
      { printf '%s\n' "$out" >"$STATE/export-$h.log"; reason=export_failed; return 1; }
  fi
  printf '%s\n' "$out" >"$STATE/export-$h.log"
  [[ $out =~ Snapshot\ created\ at\ height\ $h,\ format\ ([0-9]+),\ chunks\ ([0-9]+) ]] ||
    { reason=export_output_unrecognized; return 1; }
  fmt=${BASH_REMATCH[1]} chunks=${BASH_REMATCH[2]}
  src=$HOMEDIR/data/snapshots/$h/$fmt dst=$CKPT/$h/$fmt
  rm -rf "${CKPT:?}/$h" && mkdir -p "$CKPT/$h" && cp -a "$src" "$dst"
  drop_snapshots "$h"

  # The multiplexer extracts each embedded release under $HOME/.celestia-app/bin/<tag>.
  tag=$(ckpt_embedded_tag "$v")
  emb_sha=dry
  if [[ -z $DRY ]]; then
    emb_sha=$(cd "$USERHOME/.celestia-app/bin/$tag" 2>/dev/null && find . -type f -perm -u+x -exec sha256sum {} + |
      sort -k2 | awk '{printf "%s%s:%s", sep, $2, $1; sep = " "}' || true)
    emb_sha=${emb_sha:-not found}
  fi
  block_hash=$(ref_hash "block?height=$h" .result.block_id.hash) || { reason=reference_block_hash_unavailable; return 1; }
  app_hash=$(ref_hash "block?height=$((h + 1))" .result.block.header.app_hash) || { reason=reference_app_hash_unavailable; return 1; }

  # A pinned window starting at H+1 also names each store whose root differs.
  window=$(jq -r --argjson s $((h + 1)) '.windows[] | select(.start_height == $s) | .app_version' \
    "$RS/conformance/fixtures/mainnet/replay-windows.json" 2>/dev/null || true)
  verify_cmd=(python3 "$RS/conformance/mainnet-rpc/verify-replay-checkpoint.py" "$dst")
  [[ -n $window ]] || verify_cmd+=(--app-hash "$app_hash")
  status verify force
  if [[ -n $DRY ]]; then run "${verify_cmd[@]}"; echo dry >"$CKPT/$h/verify.log"; vrc=${CKPT_DRY_VERIFY_RC:-0}
  else "${verify_cmd[@]}" >"$CKPT/$h/verify.log" 2>&1 || vrc=$?; fi

  ckpt_metadata "$dst" app_version="$v" exporter="${cmd[*]}" embedded_tag="$tag" embedded_sha256="$emb_sha" \
    multiplexer_tag="$BENCH_GO_TAG" multiplexer_commit="$BENCH_GO_COMMIT" multiplexer_sha256="$bin_sha" \
    golang="$BENCH_GOLANG" block_hash="$block_hash" next_header_app_hash="$app_hash" \
    reference_rpcs="${CKPT_REF_RPCS[*]}" verified_against="$([[ -n $window ]] && echo "replay window $window" || echo "header $((h + 1))")" \
    verified="$([[ $vrc == 0 ]] && echo true || echo false)" verify_output="$(tail -c 2000 "$CKPT/$h/verify.log")" \
    export_output="$out" campaign="$name" run_id="$run_id" exported_at="$(date -u +%FT%TZ)" >"$CKPT/$h/checkpoint.tmp"
  jq -e --argjson n "$chunks" '.chunk_count == $n' "$CKPT/$h/checkpoint.tmp" >/dev/null || { reason=chunk_count_mismatch; return 1; }
  # A checkpoint that does not reproduce the signed app hash means the Go
  # node diverged; later heights from the same node would be wrong too. Only
  # a verified export becomes checkpoint.json, which a resumed run skips. A
  # failed one stays as failed.json, so a resume exports this height again
  # and cannot pass it.
  if [[ $vrc != 0 ]]; then
    mv "$CKPT/$h/checkpoint.tmp" "$CKPT/$h/failed.json"
    sync
    put "ckpt-$h" "$CKPT/$h/failed.json"
    reason=checkpoint_verification_failed
    return 1
  fi
  mv "$CKPT/$h/checkpoint.tmp" "$CKPT/$h/checkpoint.json"
  sync
  put "ckpt-$h" "$CKPT/$h/checkpoint.json"
  upload "$h" || echo "checkpoint $h not uploaded; it stays on the checkpoint disk" >&2
}

# upload H: copy checkpoint H to Cloud Storage under checkpoints/<campaign>/
# when the VM runs the vmexec agent with a bucket (vmexec create --gcs).
has_bucket() { [[ -n $DRY ]] || { command -v vmx-gcs >/dev/null && vmx-gcs url probe >/dev/null 2>&1; }; }
upload() {
  has_bucket || return 0
  local f
  while IFS= read -r f; do
    run vmx-gcs put "$f" "checkpoints/$name/${f#"$CKPT"/}" || return 1
  done < <(find "$CKPT/$1" -type f | sort)
}

run_campaign() {
  local h v last
  while read -r h v <&3; do
    [[ ! -s $CKPT/$h/checkpoint.json && ! -s $CKPT/$h/skipped.json ]] || continue
    last=$(cat "$STATE/last_height" 2>/dev/null || echo 0)
    if ((last > h)); then # the plan gained a height this disk has passed
      jq -nc --argjson h "$h" --argjson at "$last" '{height: $h, skipped: "node already past this height", node_height: $at}' >"$CKPT/$h/skipped.json"
      continue
    fi
    sync_to "$h" || return 1
    checkpoint "$h" "$v" || return 1
  done 3< <(ckpt_plan "$heights")
  reason=all_checkpoints
}

# --- results ----------------------------------------------------------------
finish() {
  trap - ERR
  stop_node
  local f
  du -sB1 "$HOMEDIR"/data/* "$CKPT"/* 2>/dev/null >"$STATE/du.txt" || true
  df -B1 "$DATA" "$CKPT" >"$STATE/df.txt" 2>/dev/null || true
  [[ ! -s $STATE/node.tail.log ]] || bench_log_excerpt <"$STATE/node.tail.log" >"$STATE/node.excerpt"
  tail -n 2000 "$STATE/polls.tsv" >"$STATE/polls.tail.tsv" 2>/dev/null || true
  jq -n --arg campaign "$name" --arg run_id "$run_id" --arg outcome "$outcome" --arg reason "$reason" \
    --arg heights "$heights" --arg last_height "$(cat "$STATE/last_height" 2>/dev/null)" \
    --arg next "$(ckpt_next "$CKPT" "$heights" || echo none)" \
    --arg rate_1h "$(ckpt_rate "$STATE/polls.tsv" 3600 2>/dev/null)" --arg rate_24h "$(ckpt_rate "$STATE/polls.tsv" 86400 2>/dev/null)" \
    --arg multiplexer_tag "$BENCH_GO_TAG" --arg multiplexer_sha256 "${bin_sha:-}" --arg rs_commit "$rs_commit" \
    --arg egress "$CKPT_EGRESS_CAP" --arg du "$(cat "$STATE/du.txt")" --arg df "$(cat "$STATE/df.txt" 2>/dev/null)" \
    --arg machine "$(curl -fsS -m 5 -H Metadata-Flavor:Google "$MD/machine-type" 2>/dev/null | sed 's,.*/,,')" \
    --slurpfile checkpoints <(for f in "$CKPT"/*/checkpoint.json "$CKPT"/*/skipped.json; do
      [[ -e $f ]] && jq -c 'del(.chunks)' "$f"; done) \
    '$ARGS.named + {checkpoints: $checkpoints}' >"$RUN/summary.json"
  local n files=()
  for f in polls.tail.tsv node.excerpt status.log du.txt df.txt export-*.log; do [[ -e $STATE/$f ]] && files+=("$f"); done
  tar -C "$STATE" -cf "$RUN/raw.tar" "${files[@]}" 2>/dev/null || true
  # With a bucket the raw results go there: guest attributes share ~512 KiB
  # per VM with the vmexec agent's results.
  if has_bucket && run vmx-gcs put "$RUN/raw.tar" "checkpoints/$name/raw/$(date -u +%Y%m%dT%H%M%SZ).tar"; then
    n=0
  else n=$(bench_chunk "$RUN/raw.tar" "$RUN/raw" 196608 2); fi || {
    tar -C "$STATE" -cf "$RUN/raw.tar" polls.tail.tsv status.log 2>/dev/null || true
    n=$(bench_chunk "$RUN/raw.tar" "$RUN/raw" 196608 2) || n=0
  }
  put summary "$RUN/summary.json"
  for ((i = 0; i < n; i++)); do put "raw-$i" "$RUN/raw.$i"; done
  printf '%s' "$n" >"$RUN/raw-count" && put raw-count "$RUN/raw-count"
  sync
  status "done-$outcome" force
}

# With set -E, subshells inherit the ERR trap. Only the main shell may act on it.
arm_err_trap() {
  main_pid=$BASHPID
  trap '[[ $BASHPID != "$main_pid" ]] || { reason=${reason:-"error in $phase at line $LINENO"}; outcome=failed; finish; exit 1; }' ERR
}

main() {
  if [[ -z $DRY && ${CKPT_DETACHED:-} != 1 ]]; then # the startup script runs on every boot
    mkdir -p "$RUN"
    [[ ! -e $RUN/started ]] || exit 0
    touch "$RUN/started"
    curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/attributes/startup-script" >"$RUN/run.sh"
    exec systemd-run --unit=ckpt-driver --setenv=CKPT_DETACHED=1 bash "$RUN/run.sh"
  fi
  mkdir -p "$RUN" "$STATE"
  read_inputs
  phase=start bin='' bin_sha=''
  arm_err_trap
  setup; build; configure
  if run_campaign; then outcome=${DRY:+dry_run}; outcome=${outcome:-ok}; else outcome=failed; fi
  finish
  [[ $outcome != failed ]]
}
# Run unless sourced (the guest agent may also run this through bash -c).
if [[ -z ${BASH_SOURCE[0]:-} || ${BASH_SOURCE[0]} == "$0" ]]; then main "$@"; fi
