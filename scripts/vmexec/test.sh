#!/usr/bin/env bash
# Tests scripts/vmexec offline: the pure helpers in lib.sh, then whole
# client/agent round trips with a fake gcloud and the agent in VMX_FAKE mode
# (files instead of the metadata server, background processes instead of
# systemd). Needs bash, jq, git, gzip, base64; no cloud access.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
# shellcheck source=scripts/vmexec/lib.sh
. "$here/lib.sh"
t=$(mktemp -d)
agent_pid=''
cleanup() {
  # The killed agent makes `wait` return 143; under `set -e` that would become
  # the exit status of the whole test.
  [[ -z $agent_pid ]] || { kill "$agent_pid" 2>/dev/null || true; wait "$agent_pid" 2>/dev/null || true; }
  pkill -f "$t/" 2>/dev/null || true
  pkill -f "$here/agent.sh" 2>/dev/null || true
  sleep 0.5
  rm -rf "$t" 2>/dev/null || { sleep 2; rm -rf "$t"; }
}
trap cleanup EXIT
fails=0
check() { local name=$1; shift; if "$@" >/dev/null 2>&1; then echo "ok   $name"; else echo "FAIL $name"; fails=$((fails + 1)); fi; }
refute() { local name=$1; shift; if "$@" >/dev/null 2>&1; then echo "FAIL $name"; fails=$((fails + 1)); else echo "ok   $name"; fi; }
eq() { if [[ $2 == "$3" ]]; then echo "ok   $1"; else echo "FAIL $1: want '$2' got '$3'"; fails=$((fails + 1)); fi; }

# --- validation -------------------------------------------------------------------
check  "id ok"               vmx_valid id 2611261200001a2b3c
refute "id upper"            vmx_valid id ABCDEF12
refute "id injection"        vmx_valid id 'abcdef;rm'
check  "name ok"             vmx_valid name rs-follow-1
refute "name trailing dash"  vmx_valid name rs-
refute "name upper"          vmx_valid name Rs
check  "job ok"              vmx_valid job follow
refute "job slash"           vmx_valid job a/b
check  "timeout ok"          vmx_valid timeout 3600
refute "timeout zero"        vmx_valid timeout 0
check  "labels ok"           vmx_labels_ok owner=claude-genesis,purpose=x
refute "labels upper"        vmx_labels_ok owner=Claude
check  "new id valid"        vmx_valid id "$(vmx_new_id)"

# --- request and result encoding ---------------------------------------------------
printf 'echo "hello $((1 + 2))"\nexit 3\n' >"$t/s.sh"
vmx_encode_request 60 <"$t/s.sh" >"$t/req"
eq     "request header"      "vmx1 60" "$(head -n1 "$t/req")"
eq     "request timeout"     60 "$(vmx_decode_request "$t/req" "$t/s2.sh")"
check  "request roundtrip"   cmp "$t/s.sh" "$t/s2.sh"
refute "request bad timeout" vmx_encode_request 0 <"$t/s.sh"
head -c 400000 /dev/urandom >"$t/big"
refute "request too large"   vmx_encode_request 60 <"$t/big"
printf 'vmx9 60\nabc' >"$t/badreq"
refute "request bad header"  vmx_decode_request "$t/badreq" "$t/x"

seq 1 100000 >"$t/out" # 588,895 bytes
vmx_encode_result "done" 0 "$(stat -c %s "$t/out")" "$t/out" started=x >"$t/res"
eq     "result state"        "done" "$(vmx_result_field "$t/res" state)"
eq     "result rc"           0 "$(vmx_result_field "$t/res" rc)"
eq     "result truncated"    1 "$(vmx_result_field "$t/res" truncated)"
eq     "result extra field"  x "$(vmx_result_field "$t/res" started)"
eq     "result tail end"     100000 "$(vmx_result_output "$t/res" | tail -n1)"
eq     "result tail size"    "$VMX_TAIL_BYTES" "$(vmx_result_output "$t/res" | wc -c)"
check  "result fits"         test "$(stat -c %s "$t/res")" -le 262144
vmx_encode_result running - 0 "$t/missing" >"$t/res2"
eq     "result no output"    "" "$(vmx_result_output "$t/res2")"

# --- chunking ------------------------------------------------------------------------
head -c 700000 /dev/urandom >"$t/rand"
n=$(vmx_chunk "$t/rand" "$t/c")
check  "chunks several"      test "$n" -ge 3
check  "chunk size"          test "$(stat -c %s "$t/c.0")" -le "$VMX_CHUNK_BYTES"
vmx_unchunk "$n" "$t/c" >"$t/rand2"
check  "chunk roundtrip"     cmp "$t/rand" "$t/rand2"
: >"$t/empty"
eq     "chunk empty count"   1 "$(vmx_chunk "$t/empty" "$t/e")"
eq     "raw split count"     6 "$(vmx_split_raw "$t/rand" "$t/r" 120000)"
cat "$t"/r.{0..5} >"$t/rand3"
check  "raw split roundtrip" cmp "$t/rand" "$t/rand3"

# --- request discovery ------------------------------------------------------------------
mkdir -p "$t/done/aaaaaa1"
eq     "new keys"            $'bbbbbb2\nccccccc3' "$(vmx_new_keys $'startup-script\nvmx-req-ccccccc3\nvmx-req-aaaaaa1\nvmx-req-bbbbbb2\nvmx-req-BAD!' "$t/done")"
eq     "quote"               "a\\ b c c\\;d" "$(vmx_shell_quote 'a b' c 'c;d')"
check  "job unit restart"    grep -q '^Restart=on-failure$' <(vmx_job_unit follow /x/cmd /x/log on-failure)

# --- git bundle of unpushed commits ---------------------------------------------------------
g() { git -C "$@"; }
git init -q "$t/repo"
g "$t/repo" config user.email t@t; g "$t/repo" config user.name t; g "$t/repo" config commit.gpgsign false
echo a >"$t/repo/a"; g "$t/repo" add a; g "$t/repo" commit -qm a
g "$t/repo" update-ref refs/remotes/origin/main HEAD # as if pushed
pushed=$(g "$t/repo" rev-parse HEAD)
mapfile -t b < <(vmx_bundle "$t/repo" "$t/b.bundle")
eq     "bundle pushed head"  "$pushed" "${b[0]}"
refute "bundle none written" test -e "$t/b.bundle"
echo b >"$t/repo/b"; g "$t/repo" add b; g "$t/repo" commit -qm b
mapfile -t b < <(vmx_bundle "$t/repo" "$t/b.bundle")
eq     "bundle head"         "$(g "$t/repo" rev-parse HEAD)" "${b[0]}"
eq     "bundle prerequisite" "$pushed" "${b[1]:-}"
# A shallow receiver that fetched only the prerequisite can apply the bundle.
git init -q "$t/vm"
g "$t/vm" fetch -q --depth=1 "file://$t/repo" "$pushed" 2>/dev/null
g "$t/vm" fetch -q "$t/b.bundle" HEAD 2>/dev/null
check  "bundle applies"      g "$t/vm" cat-file -e "${b[0]}^{commit}"

# --- client and agent round trips (fake cloud) ----------------------------------------------
F=$t/fake
mkdir -p "$F/md" "$F/ga/vmx" "$F/state" "$t/bin"
cat >"$t/bin/gcloud" <<'EOF'
#!/usr/bin/env bash
# Fake gcloud: instance metadata and guest attributes are files under $VMX_FAKE.
set -euo pipefail
echo "$*" >>"$VMX_FAKE/gcloud.log"
verb=$3
for a in "$@"; do
  case $a in
    --metadata-from-file=*) kv=${a#*=}; cp "${kv#*=}" "$VMX_FAKE/md/${kv%%=*}" ;;
    --keys=*) rm -f "$VMX_FAKE/md/${a#*=}" ;;
    --query-path=*) q=${a#*=} ;;
  esac
done
if [[ $1 == storage ]]; then # storage cp [--quiet] SRC DST: gs://BUCKET/OBJ is $VMX_FAKE/gcs/OBJ
  args=()
  for a in "${@:3}"; do [[ $a == --* ]] || args+=("$(sed -E 's,^gs://[^/]+/,'"$VMX_FAKE"'/gcs/,' <<<"$a")"); done
  mkdir -p "$(dirname "${args[1]}")"
  exec cp "${args[0]}" "${args[1]}"
fi
case $verb in
  add-metadata | remove-metadata) ;;
  describe)
    for f in "$VMX_FAKE/md"/*; do [[ -f $f ]] && jq -Rs --arg k "${f##*/}" '{key: $k, value: .}' "$f"; done |
      jq -s '{metadata: {items: .}}' ;;
  get-guest-attributes)
    ns=${q%%/*} key=${q#*/}
    if [[ -n $key ]]; then
      [[ -f $VMX_FAKE/ga/$ns/$key ]] || { echo "404" >&2; exit 1; }
      jq -Rs --arg k "$key" '[{key: $k, value: .}]' "$VMX_FAKE/ga/$ns/$key"
    else
      for f in "$VMX_FAKE/ga/$ns"/*; do [[ -f $f ]] && jq -Rs --arg k "${f##*/}" '{key: $k, value: .}' "$f"; done | jq -s .
    fi ;;
  *) echo "fake gcloud: unsupported $*" >&2; exit 2 ;;
esac
EOF
chmod +x "$t/bin/gcloud"
export VMX_FAKE=$F VMX_GCLOUD=$t/bin/gcloud VMX_POLL_MAX_S=1 VMX_HEARTBEAT_S=1 PATH=$t/bin:$PATH
# The fake agent's helpers: vmx-job and vmx-put on PATH, as the installer does on a VM.
printf '#!/bin/sh\nexec %q job "$@"\n' "$here/agent.sh" >"$t/bin/vmx-job"
printf '#!/bin/sh\nexec %q put "$@"\n' "$here/agent.sh" >"$t/bin/vmx-put"
printf '#!/bin/sh\nexec %q gcs "$@"\n' "$here/agent.sh" >"$t/bin/vmx-gcs"
chmod +x "$t/bin/vmx-job" "$t/bin/vmx-put" "$t/bin/vmx-gcs"
bash "$here/agent.sh" daemon >"$t/agent.log" 2>&1 &
agent_pid=$!
X="$here/vmexec"

out=$("$X" exec vm1 -- "echo hi; echo err >&2" 2>/dev/null)
eq     "exec output"         $'hi\nerr' "$out"
rc=0; "$X" exec vm1 -- 'exit 7' >/dev/null 2>&1 || rc=$?
eq     "exec exit status"    7 "$rc"
eq     "exec request removed" "" "$(ls "$F/md")"
out=$(printf 'cd /\npwd\necho "$VMX_ID" | wc -c\n' | "$X" exec vm1 -f - 2>/dev/null)
eq     "exec script stdin"   $'/\n19' "$out"
rc=0; "$X" exec vm1 --timeout 1 -- 'sleep 10' >/dev/null 2>&1 || rc=$?
eq     "exec timeout"        124 "$rc"
id=$("$X" exec vm1 --detach -- 'echo later')
check  "detach id"           vmx_valid id "$id"
eq     "result of detached"  later "$("$X" result vm1 "$id" 2>/dev/null)"
"$X" exec vm1 -- 'seq 1 300000' >"$t/long" 2>"$t/long.err"
eq     "long output tail"    300000 "$(tail -n1 "$t/long")"
check  "long output notice"  grep -q 'output truncated' "$t/long.err"
for _ in $(seq 1 40); do [[ -e $F/state/runs/$id/reaped ]] && break; sleep 0.5; done
check  "result reaped"       test ! -e "$F/ga/vmx/$id"

head -c 300000 /dev/urandom >"$t/payload"
"$X" push vm1 "$t/payload" "$t/vmfs/payload" >/dev/null 2>&1
check  "push"                cmp "$t/payload" "$t/vmfs/payload"
"$X" fetch vm1 "$t/vmfs/payload" "$t/fetched" >/dev/null 2>&1
check  "fetch"               cmp "$t/payload" "$t/fetched"

"$X" job vm1 start counter --interval 1 --status-cmd 'echo height=$(wc -l <log)' -- bash -c 'for i in $(seq 1 1000); do echo tick $i; sleep 0.1; done' >/dev/null 2>&1
for _ in $(seq 1 160); do grep -q 'height=' "$F/ga/vmx/job-counter" 2>/dev/null && break; sleep 0.25; done
st=$("$X" job vm1 status counter 2>/dev/null || true)
check  "job status published" grep -q '^ActiveState=active' <<<"$st"
check  "job status cmd"      grep -q '^height=' <<<"$st"
check  "job log"             grep -q 'tick' <<<"$("$X" job vm1 logs counter 5 2>/dev/null)"
"$X" job vm1 stop counter >/dev/null 2>&1
check  "job stopped"         grep -q '^ActiveState=inactive' <<<"$("$X" job vm1 status counter 2>/dev/null)"
check  "heartbeat"           grep -q '^version=1' "$F/ga/vmx/agent"
check  "status"              grep -q 'job-counter' <<<"$("$X" status vm1 2>/dev/null)"

# --- with a bucket (VMs created with --gcs) ----------------------------------------------------
printf 'test-bucket' >"$F/md/vmx-bucket"
head -c 900000 /dev/urandom >"$t/payload2"
"$X" push vm1 "$t/payload2" "$t/vmfs/payload2" >/dev/null 2>"$t/push2.err"
check  "gcs push"            cmp "$t/payload2" "$t/vmfs/payload2"
refute "gcs push in one go"  grep -q 'pushed 2/' "$t/push2.err"
check  "gcs push object"     bash -c "ls '$F/gcs/vmexec/vm1/push/' | grep -q ."
"$X" fetch vm1 "$t/vmfs/payload2" "$t/fetched2" >/dev/null 2>&1
check  "gcs fetch"           cmp "$t/payload2" "$t/fetched2"
"$X" exec vm1 -- 'seq 1 300000' >/dev/null 2>"$t/long2.err"
check  "gcs whole output"    grep -q 'gs://test-bucket/vmexec/fake/runs/.*\.out' "$t/long2.err"
check  "gcs output object"   bash -c "tail -n1 '$F'/gcs/vmexec/fake/runs/*.out | grep -qx 300000"
check  "gcs invalid object"  bash -c "! '$here/agent.sh' gcs get '../x' /dev/null"

echo
if ((fails)); then echo "$fails test(s) failed"; exit 1; fi
echo "all tests passed"
