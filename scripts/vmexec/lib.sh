# shellcheck shell=bash
# Pure helpers shared by the vmexec client (scripts/vmexec/vmexec), its VM
# agent (agent.sh) and test.sh. No function here touches the network or the
# cloud. See scripts/vmexec/README.md for the protocol.

VMX_VERSION=1
# Instance metadata: one value <= 256 KiB, all values together <= 512 KiB.
# A request is "vmx1 TIMEOUT_S\n" + base64(gzip(script)).
VMX_REQ_MAX=245760
# Guest attributes: one value <= 256 KiB. Results carry at most this many
# bytes of the output's end, so the gzip + base64 encoding always fits.
VMX_TAIL_BYTES=180000
# One chunk of a published file (base64 text).
VMX_CHUNK_BYTES=240000

# vmx_valid KIND VALUE: strict format check for every external input.
vmx_valid() {
  case $1 in
    id) [[ $2 =~ ^[a-z0-9]{6,32}$ ]] ;;
    name) [[ $2 =~ ^[a-z][a-z0-9-]{0,40}[a-z0-9]$ ]] ;;
    job) [[ $2 =~ ^[a-z][a-z0-9-]{0,30}$ ]] ;;
    timeout) [[ $2 =~ ^[1-9][0-9]{0,6}$ ]] ;;
    label) [[ $2 =~ ^[a-z][a-z0-9_-]{0,62}$ ]] ;;
    *) return 1 ;;
  esac
}

# vmx_new_id: a fresh request id, time-ordered.
vmx_new_id() { printf '%s%s\n' "$(date -u +%y%m%d%H%M%S)" "$(od -An -N3 -tx1 /dev/urandom | tr -d ' \n')"; }

# vmx_encode_request TIMEOUT < script: the metadata value of one request.
# Fails if it would not fit in one metadata value.
vmx_encode_request() {
  local body
  vmx_valid timeout "$1" || { echo "invalid timeout: $1" >&2; return 1; }
  body=$(gzip -9c | base64 -w0)
  if ((${#body} + 32 > VMX_REQ_MAX)); then
    echo "request too large: ${#body} encoded bytes > $VMX_REQ_MAX (push large files with 'vmexec push')" >&2
    return 1
  fi
  printf 'vmx%s %s\n%s' "$VMX_VERSION" "$1" "$body"
}

# vmx_decode_request FILE SCRIPT_OUT: prints the timeout, writes the script.
vmx_decode_request() {
  local head timeout
  head=$(head -n1 "$1")
  [[ $head =~ ^vmx1\ ([1-9][0-9]{0,6})$ ]] || { echo "bad request header" >&2; return 1; }
  timeout=${BASH_REMATCH[1]}
  tail -n +2 "$1" | base64 -d | gunzip >"$2" || { echo "bad request body" >&2; return 1; }
  echo "$timeout"
}

# vmx_encode_result STATE RC BYTES OUTFILE [KEY=VALUE...]: a result value:
# "key=value" lines, a "--" line, then base64(gzip(the last VMX_TAIL_BYTES
# bytes of OUTFILE)). OUTFILE may be missing (no output yet).
vmx_encode_result() {
  local state=$1 rc=$2 bytes=$3 out=$4 kv
  shift 4
  printf 'state=%s\nrc=%s\nbytes=%s\ntruncated=%s\n' "$state" "$rc" "$bytes" "$((bytes > VMX_TAIL_BYTES ? 1 : 0))"
  for kv in "$@"; do printf '%s\n' "$kv"; done
  printf -- '--\n'
  if [[ -s $out ]]; then tail -c "$VMX_TAIL_BYTES" "$out"; fi | gzip -9c | base64 -w0
}

# vmx_result_field FILE KEY: one header field of a result value.
vmx_result_field() { awk -v k="$2" '$0 == "--" { exit } index($0, k "=") == 1 { print substr($0, length(k) + 2); exit }' "$1"; }

# vmx_result_output FILE: the output tail of a result value.
vmx_result_output() { awk 'f { print } $0 == "--" { f = 1 }' "$1" | base64 -d 2>/dev/null | gunzip 2>/dev/null; }

# vmx_chunk FILE PREFIX: gzip + base64 FILE into PREFIX.0, PREFIX.1, ... of
# at most VMX_CHUNK_BYTES each. Prints the count.
vmx_chunk() {
  local n=0 f
  gzip -6c "$1" | base64 -w0 | split -d -a 4 -b "$VMX_CHUNK_BYTES" - "$2.tmp."
  for f in "$2".tmp.*; do [[ -e $f ]] || continue; mv "$f" "$2.$n"; n=$((n + 1)); done
  if ((n == 0)); then : >"$2.0"; n=1; fi # an empty file still has one (empty) chunk
  echo "$n"
}

# vmx_unchunk COUNT PREFIX: inverse of vmx_chunk, to stdout.
vmx_unchunk() {
  local i
  for ((i = 0; i < $1; i++)); do cat "$2.$i" || return 1; done | base64 -d | gunzip
}

# vmx_split_raw FILE PREFIX SIZE: split FILE into raw pieces PREFIX.0... of
# SIZE bytes each (for pushing a file through requests). Prints the count.
vmx_split_raw() {
  local n=0 f
  split -d -a 4 -b "$3" "$1" "$2.tmp."
  for f in "$2".tmp.*; do [[ -e $f ]] || continue; mv "$f" "$2.$n"; n=$((n + 1)); done
  if ((n == 0)); then : >"$2.0"; n=1; fi
  echo "$n"
}

# vmx_shell_quote ARG...: one shell-quoted word per argument, space separated.
vmx_shell_quote() {
  local a words=()
  for a in "$@"; do words+=("$(printf '%q' "$a")"); done
  printf '%s\n' "${words[*]}"
}

# vmx_label_ok "k=v,k=v": every label is a valid GCE label.
vmx_labels_ok() {
  local kv
  local -a kvs
  IFS=, read -r -a kvs <<<"$1"
  ((${#kvs[@]} > 0)) || return 1
  for kv in "${kvs[@]}"; do
    [[ $kv =~ ^[a-z][a-z0-9_-]{0,62}=[a-z0-9_-]{0,63}$ ]] || return 1
  done
}

# vmx_new_keys LISTING DONE_DIR: request ids in a metadata attribute listing
# (one key per line) that have no DONE_DIR/<id> yet, in order.
vmx_new_keys() {
  local k id
  while IFS= read -r k; do
    [[ $k =~ ^vmx-req-([a-z0-9]{6,32})$ ]] || continue
    id=${BASH_REMATCH[1]}
    [[ -e $2/$id ]] || echo "$id"
  done <<<"$1" | sort
}

# vmx_job_unit NAME CMD_FILE LOG [RESTART]: a systemd unit file for a job.
# The command runs through bash so CMD_FILE may be any script.
vmx_job_unit() {
  cat <<EOF
[Unit]
Description=vmexec job $1
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=HOME=/root
Environment=PATH=/root/.cargo/bin:/usr/local/go/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
WorkingDirectory=/var/lib/vmexec/jobs/$1
ExecStart=/bin/bash $2
StandardOutput=append:$3
StandardError=append:$3
Restart=${4:-no}
RestartSec=30
LimitNOFILE=1048576
KillMode=mixed
TimeoutStopSec=120

[Install]
WantedBy=multi-user.target
EOF
}

# vmx_bundle REPO OUT: write a git bundle of HEAD's commits that no remote
# branch has. Prints HEAD's sha, then one prerequisite sha per line (pushed
# commits the receiver must fetch first). Writes no bundle and prints only
# HEAD's sha when HEAD is already on a remote branch.
vmx_bundle() {
  local head
  head=$(git -C "$1" rev-parse HEAD) || return 1
  echo "$head"
  rm -f "$2"
  [[ -n $(git -C "$1" rev-list -n1 HEAD --not --remotes) ]] || return 0
  git -C "$1" bundle create "$2" HEAD --not --remotes >/dev/null 2>&1 || return 1
  # Header lines "-<sha> <subject>" before the first blank line are the prerequisites.
  sed -n '/^$/q; s/^-\([0-9a-f]\{40\}\).*/\1/p' "$2"
}
