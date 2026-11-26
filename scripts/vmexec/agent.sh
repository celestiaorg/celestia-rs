#!/usr/bin/env bash
# vmexec VM agent. The client (scripts/vmexec/vmexec) passes lib.sh + this
# file as the VM's startup script. On every boot it installs itself as the
# systemd service vmexec-agent, which long-polls the instance metadata for
# "vmx-req-<id>" keys, runs each request as a transient systemd unit and
# publishes the exit status and the end of the output as the guest attribute
# vmx/<id>. See scripts/vmexec/README.md.
#
#   agent.sh                 install and start the service (startup script)
#   agent.sh daemon          the service's main loop
#   agent.sh runone ID       run one decoded request (started by the daemon)
#   agent.sh put KEY FILE|-  publish guest attribute vmx/u-KEY (vmx-put)
#   agent.sh job ...         long-running jobs (vmx-job; see usage below)
#
# VMX_FAKE=DIR replaces the metadata server, guest attributes and systemd
# with files under DIR and background processes (scripts/vmexec/test.sh).
set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=scripts/vmexec/lib.sh
declare -F vmx_valid >/dev/null || . "$here/lib.sh"

FAKE=${VMX_FAKE:-}
ROOT=${FAKE:+$FAKE/state}
ROOT=${ROOT:-/var/lib/vmexec}
SELF=${FAKE:+${BASH_SOURCE[0]}}
SELF=${SELF:-/usr/local/lib/vmexec/agent.sh}
MD=http://metadata.google.internal/computeMetadata/v1/instance
POLL_S=${VMX_POLL_S:-30}       # long-poll timeout: heartbeats and job status run between polls
HEARTBEAT_S=${VMX_HEARTBEAT_S:-120}
RESULT_MAX=${VMX_RESULT_MAX:-120000} # encoded bytes; all guest attributes of a VM share ~512 KiB

log() { printf '%s vmexec: %s\n' "$(date -u +%FT%TZ)" "$*" >&2; }

# --- metadata and guest attributes ---------------------------------------------
md_list() { # attribute keys, one per line; waits for a change when given an etag
  if [[ -n $FAKE ]]; then
    local before
    before=$(ls "$FAKE/md" 2>/dev/null | md5sum)
    if [[ -n ${1:-} ]]; then
      local i
      for ((i = 0; i < 10; i++)); do [[ $(ls "$FAKE/md" 2>/dev/null | md5sum) != "$before" ]] && break; sleep 0.2; done
    fi
    ls "$FAKE/md" 2>/dev/null; echo "etag=fake" >"$ROOT/etag"; return 0
  fi
  local url="$MD/attributes/" hdr=$ROOT/md.hdr
  [[ -z ${1:-} ]] || url+="?wait_for_change=true&timeout_sec=$POLL_S&last_etag=$1"
  curl -fsS -m $((POLL_S + 15)) -D "$hdr" -H Metadata-Flavor:Google "$url" || return 1
  awk 'tolower($1) == "etag:" { gsub(/\r/, "", $2); print "etag=" $2 }' "$hdr" >"$ROOT/etag"
}
md_get() { # md_get KEY OUT
  if [[ -n $FAKE ]]; then cp "$FAKE/md/$1" "$2"; return; fi
  curl -fsS -m 30 -H Metadata-Flavor:Google -o "$2" "$MD/attributes/$1"
}
ga_put() { # ga_put KEY FILE: guest attribute vmx/KEY, retried
  local i
  if [[ -n $FAKE ]]; then mkdir -p "$FAKE/ga/vmx" && cp "$2" "$FAKE/ga/vmx/$1.tmp" && mv "$FAKE/ga/vmx/$1.tmp" "$FAKE/ga/vmx/$1"; return; fi
  for i in 1 2 3 4 5 6; do
    curl -fsS -m 30 -X PUT --data-binary "@$2" -H Metadata-Flavor:Google "$MD/guest-attributes/vmx/$1" >/dev/null && return 0
    sleep $((i * 5))
  done
  log "guest attribute vmx/$1 not written"
  return 1
}
ga_del() { # ga_del KEY
  if [[ -n $FAKE ]]; then rm -f "$FAKE/ga/vmx/$1"; return; fi
  curl -fsS -m 30 -X DELETE -H Metadata-Flavor:Google "$MD/guest-attributes/vmx/$1" >/dev/null 2>&1 || true
}

# --- Cloud Storage (VMs created with --gcs) -----------------------------------------
# The bucket is the vmx-bucket metadata value; objects go under
# vmexec/<instance name>/. The VM's service account token (storage scope) is
# read from the metadata server; nothing secret is stored.
gcs_bucket() {
  if [[ -n $FAKE ]]; then cat "$FAKE/md/vmx-bucket" 2>/dev/null; return; fi
  curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/attributes/vmx-bucket" 2>/dev/null
}
gcs_prefix() {
  local n
  if [[ -n $FAKE ]]; then n=fake; else n=$(curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/name"); fi
  echo "vmexec/$n"
}
gcs_token() {
  curl -fsS -m 10 -H Metadata-Flavor:Google "$MD/service-accounts/default/token" | sed -n 's/.*"access_token" *: *"\([^"]*\)".*/\1/p'
}
gcs_obj_ok() { [[ $1 =~ ^[A-Za-z0-9._/-]{1,900}$ && $1 != *..* ]]; }
gcs() { # gcs put FILE OBJECT | get OBJECT FILE | url OBJECT
  local b op=${1:-} obj tok
  b=$(gcs_bucket) && [[ -n $b ]] || { echo "vmx-gcs: this VM has no bucket (create it with --gcs)" >&2; return 1; }
  case $op in put) obj=$3 ;; get | url) obj=$2 ;; *) echo "usage: vmx-gcs put FILE OBJECT | get OBJECT FILE | url OBJECT" >&2; return 2 ;; esac
  gcs_obj_ok "$obj" || { echo "vmx-gcs: invalid object name: $obj" >&2; return 2; }
  if [[ -n $FAKE ]]; then
    case $op in
      put) mkdir -p "$(dirname "$FAKE/gcs/$obj")" && cp "$2" "$FAKE/gcs/$obj" ;;
      get) cp "$FAKE/gcs/$obj" "$3" ;;
      url) echo "gs://$b/$obj" ;;
    esac
    return
  fi
  [[ $op == url ]] && { echo "gs://$b/$obj"; return; }
  tok=$(gcs_token) && [[ -n $tok ]] || { echo "vmx-gcs: no service account token" >&2; return 1; }
  case $op in
    # XML API PUT streams the file (-T), so large files need no memory.
    put) curl -fsS --retry 5 -T "$2" -H @<(printf 'Authorization: Bearer %s\n' "$tok") -H 'Content-Type: application/octet-stream' \
      "https://storage.googleapis.com/$b/$obj" >/dev/null ;;
    get) curl -fsS --retry 5 -H @<(printf 'Authorization: Bearer %s\n' "$tok") -o "$3" "https://storage.googleapis.com/storage/v1/b/$b/o/${obj//\//%2F}?alt=media" ;;
  esac
}

# --- install (startup script) ----------------------------------------------------
install_agent() {
  mkdir -p /usr/local/lib/vmexec "$ROOT"/{runs,jobs}
  curl -fsS -m 30 -H Metadata-Flavor:Google "$MD/attributes/startup-script" >/usr/local/lib/vmexec/agent.sh.new &&
    mv /usr/local/lib/vmexec/agent.sh.new "$SELF" && chmod 755 "$SELF"
  ln -sf "$SELF" /usr/local/bin/vmx-agent
  printf '#!/bin/sh\nexec %s put "$@"\n' "$SELF" >/usr/local/bin/vmx-put
  printf '#!/bin/sh\nexec %s job "$@"\n' "$SELF" >/usr/local/bin/vmx-job
  printf '#!/bin/sh\nexec %s gcs "$@"\n' "$SELF" >/usr/local/bin/vmx-gcs
  chmod 755 /usr/local/bin/vmx-put /usr/local/bin/vmx-job /usr/local/bin/vmx-gcs
  # Unattended upgrades take the apt lock and needrestart restarts services
  # whose libraries changed, including transient units: keep both away from
  # long jobs (this killed the first Go checkpoint campaign's driver).
  systemctl disable --now unattended-upgrades.service apt-daily.timer apt-daily-upgrade.timer >/dev/null 2>&1 || true
  mkdir -p /etc/needrestart/conf.d && echo "\$nrconf{restart} = 'l';" >/etc/needrestart/conf.d/vmexec.conf
  # A disk attached with device name "data" is mounted at /data (formatted only if blank).
  local dev=/dev/disk/by-id/google-data
  if [[ -e $dev ]]; then
    mkdir -p /data
    blkid -p "$dev" >/dev/null 2>&1 || mkfs.ext4 -q -m 0 -E lazy_itable_init=1 "$dev"
    mountpoint -q /data || mount -o noatime,discard "$dev" /data
  fi
  cat >/etc/systemd/system/vmexec-agent.service <<EOF
[Unit]
Description=vmexec agent
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=$SELF daemon
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
EOF
  systemctl daemon-reload
  systemctl enable vmexec-agent.service >/dev/null 2>&1
  systemctl restart vmexec-agent.service
}

# --- running requests -----------------------------------------------------------
publish_result() { # publish_result ID STATE: guest attribute vmx/ID
  local id=$1 d=$ROOT/runs/$1 rc=- bytes=0 tail=$VMX_TAIL_BYTES enc
  [[ ! -s $d/rc ]] || rc=$(cat "$d/rc")
  [[ ! -e $d/out ]] || bytes=$(stat -c %s "$d/out")
  enc=$d/result
  while :; do
    VMX_TAIL_BYTES=$tail vmx_encode_result "$2" "$rc" "$bytes" "$d/out" "started=$(cat "$d/started" 2>/dev/null)" \
      "ended=$(cat "$d/ended" 2>/dev/null)" "output=$d/out" "output_gs=$(cat "$d/output_gs" 2>/dev/null)" >"$enc"
    (($(stat -c %s "$enc") <= RESULT_MAX || tail < 1024)) && break
    tail=$((tail / 2))
  done
  ga_put "$1" "$enc"
}

runone() { # runone ID: the request's script was decoded by the daemon into runs/ID
  local id=$1 d=$ROOT/runs/$1 rc
  vmx_valid id "$id" || return 1
  date -u +%FT%TZ >"$d/started"
  publish_result "$id" running
  (
    cd "$d" || exit 1
    export HOME=${HOME:-/root} VMX_ID=$id VMX_RUN_DIR=$d
    export PATH=/root/.cargo/bin:/usr/local/go/bin:$PATH
    exec timeout -k 30 "$(cat "$d/timeout")" bash "$d/script"
  ) >"$d/out" 2>&1 </dev/null
  rc=$?
  echo "$rc" >"$d/rc"
  date -u +%FT%TZ >"$d/ended"
  # Output beyond the result's tail goes to the bucket when the VM has one.
  if (($(stat -c %s "$d/out") > VMX_TAIL_BYTES)) && [[ -n $(gcs_bucket) ]]; then
    local obj
    obj=$(gcs_prefix)/runs/$id.out
    gcs put "$d/out" "$obj" && gcs url "$obj" >"$d/output_gs"
  fi
  publish_result "$id" "done"
}

start_request() { # start_request ID: fetch, decode and launch one request
  local id=$1 d=$ROOT/runs/$1 timeout
  mkdir "$d" 2>/dev/null || return 0 # claimed already
  if ! md_get "vmx-req-$id" "$d/req" || ! timeout=$(vmx_decode_request "$d/req" "$d/script"); then
    echo "undecodable request" >"$d/out"; echo 125 >"$d/rc"; publish_result "$id" "done"; return 0
  fi
  echo "$timeout" >"$d/timeout"
  log "request $id (timeout ${timeout}s)"
  if [[ -n $FAKE ]]; then
    setsid "$SELF" runone "$id" >>"$ROOT/agent.log" 2>&1 </dev/null &
  else
    systemd-run --quiet --collect --unit="vmx-run-$id" --setenv=HOME=/root "$SELF" runone "$id" ||
      { echo "systemd-run failed" >"$d/out"; echo 126 >"$d/rc"; publish_result "$id" "done"; }
  fi
}

# Once the client removed vmx-req-ID (it has read the result), drop the
# result attribute: all guest attributes of a VM share about 512 KiB.
reap() { # reap LISTING
  local d id
  for d in "$ROOT"/runs/*/; do
    [[ -d $d ]] || continue
    id=$(basename "$d")
    [[ -s $d/rc && ! -e $d/reaped ]] || continue
    grep -qx "vmx-req-$id" <<<"$1" && continue
    ga_del "$id"
    touch "$d/reaped"
  done
}

heartbeat() {
  local f=$ROOT/heartbeat
  {
    echo "version=$VMX_VERSION"
    echo "time=$(date -u +%FT%TZ)"
    echo "uptime_s=$(cut -d. -f1 /proc/uptime)"
    echo "load=$(cut -d' ' -f1-3 /proc/loadavg)"
    echo "mem_avail_mb=$(awk '/MemAvailable/ { printf "%d", $2 / 1024 }' /proc/meminfo)"
    df -BG --output=target,used,avail / /data 2>/dev/null | awk 'NR > 1 { printf "disk_%s=%s/%s\n", ($1 == "/" ? "root" : "data"), $2, $3 }'
    echo "jobs=$(ls "$ROOT/jobs" 2>/dev/null | tr '\n' ' ')"
  } >"$f"
  ga_put agent "$f"
}

daemon() {
  mkdir -p "$ROOT"/{runs,jobs}
  local etag='' listing last_hb=0 id
  log "agent v$VMX_VERSION started"
  while :; do
    if listing=$(md_list "$etag"); then
      etag=$(sed -n 's/^etag=//p' "$ROOT/etag" 2>/dev/null)
      while read -r id; do [[ -z $id ]] || start_request "$id"; done < <(vmx_new_keys "$listing" "$ROOT/runs")
      reap "$listing"
    else
      etag=''; sleep 2
    fi
    if (($(date +%s) - last_hb >= HEARTBEAT_S)); then heartbeat; last_hb=$(date +%s); fi
    job_publish_due
    [[ ! -e $ROOT/stop ]] || { log "stop file"; return 0; }
  done
}

# --- jobs -------------------------------------------------------------------------
# vmx-job start NAME [--restart] [--boot] [--interval S] [--status-cmd CMD] -- COMMAND...
# vmx-job stop NAME | status NAME | logs NAME [LINES] | list
# A job is a systemd service (vmxjob-NAME), started at every boot with --boot, logging to
# /var/lib/vmexec/jobs/NAME/log. Every interval (default 60 s) the agent
# publishes vmx/job-NAME: the unit state, the status command's output and
# the log's last lines.
# pid_alive PID (fake mode): the process runs. A killed process that nobody
# has reaped yet (a zombie) still answers `kill -0`; it does not count.
pid_alive() {
  [[ -n ${1:-} ]] && kill -0 "$1" 2>/dev/null &&
    [[ $(sed -E 's/^.*\) //; s/ .*//' "/proc/$1/stat" 2>/dev/null) != Z ]]
}
job_status_text() { # job_status_text NAME
  local j=$ROOT/jobs/$1 unit=vmxjob-$1
  echo "job=$1"
  echo "time=$(date -u +%FT%TZ)"
  if [[ -n $FAKE ]]; then
    if [[ -s $j/pid ]] && pid_alive "$(cat "$j/pid")"; then echo "ActiveState=active"; else echo "ActiveState=inactive"; fi
  else
    systemctl show "$unit" -p ActiveState -p SubState -p NRestarts -p ExecMainStartTimestamp -p ExecMainStatus 2>/dev/null
  fi
  if [[ -s $j/status_cmd ]]; then
    echo "--- status"
    (cd "$j" && timeout 30 bash "$j/status_cmd" 2>&1 | tail -c 4000)
  fi
  echo "--- log"
  tail -n 25 "$j/log" 2>/dev/null | cut -c1-300
}

job_publish() { # job_publish NAME
  local j=$ROOT/jobs/$1
  job_status_text "$1" | head -c 12000 >"$j/status.txt"
  ga_put "job-$1" "$j/status.txt"
  date +%s >"$j/published"
}

job_publish_due() {
  local j name iv last
  for j in "$ROOT"/jobs/*/; do
    [[ -d $j ]] || continue
    name=$(basename "$j")
    iv=$(cat "$j/interval" 2>/dev/null || echo 60)
    last=$(cat "$j/published" 2>/dev/null || echo 0)
    if [[ -e $j/removed ]]; then continue; fi
    (($(date +%s) - last >= iv)) && job_publish "$name"
  done
}

job() {
  local cmd=${1:-} name=${2:-}
  case $cmd in
    list) ls "$ROOT/jobs" 2>/dev/null; return ;;
    start | stop | status | logs | remove) vmx_valid job "$name" || { echo "invalid job name: $name" >&2; return 2; } ;;
    *) echo "usage: vmx-job start NAME [--restart] [--boot] [--interval S] [--status-cmd CMD] -- COMMAND... | stop NAME | status NAME | logs NAME [LINES] | remove NAME | list" >&2; return 2 ;;
  esac
  shift 2
  local j=$ROOT/jobs/$name unit=vmxjob-$name
  case $cmd in
    start)
      local restart=no interval=60 status_cmd='' boot=0
      while (($#)); do
        case $1 in
          --restart) restart=on-failure; shift ;;
          --boot) boot=1; shift ;; # also start at every boot (default: this boot only)
          --interval) interval=$2; shift 2 ;;
          --status-cmd) status_cmd=$2; shift 2 ;;
          --) shift; break ;;
          *) break ;;
        esac
      done
      (($#)) || { echo "vmx-job start: no command" >&2; return 2; }
      vmx_valid timeout "$interval" || { echo "invalid interval" >&2; return 2; }
      mkdir -p "$j" && rm -f "$j/removed"
      if [[ -z $FAKE ]] && systemctl is-active --quiet "$unit"; then echo "job $name is already running" >&2; return 1; fi
      printf '#!/bin/bash\n%s\n' "$(vmx_shell_quote "$@")" >"$j/cmd"
      if [[ -n $status_cmd ]]; then printf "%s\n" "$status_cmd" >"$j/status_cmd"; else rm -f "$j/status_cmd"; fi
      echo "$interval" >"$j/interval"
      echo "=== $(date -u +%FT%TZ) start: $*" >>"$j/log"
      if [[ -n $FAKE ]]; then
        (cd "$j" && setsid bash "$j/cmd" >>"$j/log" 2>&1 </dev/null & echo $! >"$j/pid")
      else
        vmx_job_unit "$name" "$j/cmd" "$j/log" "$restart" >"/etc/systemd/system/$unit.service"
        systemctl daemon-reload || return 1
        if ((boot)); then systemctl enable --now "$unit" >/dev/null 2>&1 || return 1; else systemctl start "$unit" || return 1; fi
      fi
      echo 0 >"$j/published" # publish on the agent's next pass
      echo "started job $name"
      ;;
    stop)
      if [[ -n $FAKE ]]; then
        local pid i
        pid=$(cat "$j/pid" 2>/dev/null || true)
        [[ -z $pid ]] || kill -- "-$pid" 2>/dev/null || kill "$pid" 2>/dev/null || true
        for ((i = 0; i < 50; i++)); do
          if ! pid_alive "$pid"; then break; fi
          sleep 0.1
        done
      else
        systemctl disable --now "$unit" >/dev/null 2>&1
      fi
      echo "=== $(date -u +%FT%TZ) stopped" >>"$j/log"
      job_publish "$name"
      echo "stopped job $name"
      ;;
    remove) # stop, and stop publishing (frees its guest attribute)
      job stop "$name" >/dev/null
      touch "$j/removed"; ga_del "job-$name"
      [[ -n $FAKE ]] || { rm -f "/etc/systemd/system/$unit.service"; systemctl daemon-reload; }
      echo "removed job $name (log kept in $j)"
      ;;
    status) job_status_text "$name" ;;
    logs) tail -n "${1:-100}" "$j/log" ;;
  esac
}

put() { # put KEY FILE|-
  vmx_valid job "${1:-}" || { echo "usage: vmx-put KEY FILE|- (KEY: [a-z][a-z0-9-]*)" >&2; return 2; }
  local f=${2:--} tmp
  if [[ $f == - ]]; then tmp=$(mktemp); cat >"$tmp"; f=$tmp; fi
  ga_put "u-$1" "$f"
  [[ -z ${tmp:-} ]] || rm -f "$tmp"
}

case ${1:-install} in
  install) [[ -z $FAKE ]] && install_agent ;;
  daemon) daemon ;;
  runone) runone "$2" ;;
  job) shift; job "$@" ;;
  put) shift; put "$@" ;;
  gcs) shift; gcs "$@" ;;
  *) echo "usage: agent.sh [install|daemon|runone ID|job ...|put KEY FILE]" >&2; exit 2 ;;
esac
