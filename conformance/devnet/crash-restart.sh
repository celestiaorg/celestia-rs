#!/usr/bin/env bash
# Whole-network crash inside the delayed-precommit window.
#
# Every validator is SIGKILLed while the network waits out the delayed
# precommit of a height (round step Prevote, every prevote cast, no
# precommit yet), then every node restarts from its home. The run reports,
# per attempt, whether the chain made progress within $WAIT_S seconds.
#
# Validators keep peer-gossiped proposals and block parts out of the WAL
# (`only_internal_wal = true`, the default), so after the restart only the
# proposer holds the block; it cannot serve it (the propagation reactor's
# proposal cache is not rebuilt from the WAL replay), and the others hold
# a polka for a block they do not have, received before they reach the
# prevote step, with their own nil prevote refused by the privval (they
# prevoted the block before the crash).
#
# usage: [CRASH_KINDS="go go go go"] [ATTEMPTS=3] [WAIT_S=90]
#        [DEVNET_PORT_OFFSET=19000] [DEVNET_BIN=dir] [CELESTIA_RS=path]
#        conformance/devnet/crash-restart.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
export PATH=$BIN:$PATH
OFF=${DEVNET_PORT_OFFSET:-19000}
KINDS=${CRASH_KINDS:-go go go go}
if [[ " $KINDS " == *" rs "* ]] && [ -z "${CELESTIA_RS:-}" ]; then
  (cd "$ROOT" && cargo build -q --release -p celestia-rs)
  CELESTIA_RS=$ROOT/target/release/celestia-rs
fi
RS=${CELESTIA_RS:-}
W=${CRASH_DIR:-$(mktemp -d)}
FAIL=()
echo "dir: $W"
# shellcheck source=lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"

read -r -a K <<<"$KINDS"
N=${#K[@]}
STAKES=$(for _ in "${K[@]}"; do printf '5000000000 '; done)
make_net "$W" crash-1 "$KINDS" "$STAKES"
for i in $(seq 0 $((N - 1))); do start_node "$W" "$i"; done
ensure_up "$W" $(seq 0 $((N - 1)))
wait_height "$W" 0 10 180 || { echo "network did not start"; exit 1; }

stalls=0
for a in $(seq 1 "${ATTEMPTS:-3}"); do
  # Wait for: step Prevote, 1.0-1.8 s after the round's start time (the
  # delayed precommit fires at start + 2.1 s).
  hit=$(python3 - "$(rpc "$W" 0)" <<'PY'
import calendar, json, re, sys, time, urllib.request
url = sys.argv[1]
end = time.time() + 120
seen = []
while time.time() < end:
    try:
        rs = json.load(urllib.request.urlopen(url + "/consensus_state", timeout=1))["result"]["round_state"]
    except Exception:
        time.sleep(0.05)
        continue
    h, r, st = rs["height/round/step"].split("/")
    m = re.match(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d+))?Z", rs["start_time"])
    start = calendar.timegm(time.strptime(m.group(1), "%Y-%m-%dT%H:%M:%S")) + float("0." + (m.group(2) or "0"))
    votes = rs.get("height_vote_set") or [{}]
    pv = votes[0].get("prevotes_bit_array", "")
    seen = (seen + [(rs["height/round/step"], round(time.time() - start, 2), pv[-12:])])[-8:]
    if st == "4" and 0.3 <= time.time() - start <= 1.8 and (not pv or "= 1.00" in pv or "= 0.75" in pv):
        print(h)
        sys.exit(0)
    time.sleep(0.02)
print("no window; last samples:", seen, file=sys.stderr)
sys.exit(1)
PY
) || { echo "attempt $a: no prevote window seen"; continue; }
  for i in $(seq 0 $((N - 1))); do kill -9 "$(cat "$W/n$i/pid")" 2>/dev/null || true; done
  for i in $(seq 0 $((N - 1))); do wait "$(cat "$W/n$i/pid")" 2>/dev/null || true; done
  echo "attempt $a: killed every node at height $hit (prevote step)"
  sleep 2
  for i in $(seq 0 $((N - 1))); do start_node "$W" "$i"; done
  ensure_up "$W" $(seq 0 $((N - 1)))
  if wait_height "$W" 0 $((hit + 2)) "${WAIT_S:-90}"; then
    echo "attempt $a: chain resumed (height $(height "$W" 0))"
  else
    echo "attempt $a: STALLED at height $(height "$W" 0) for ${WAIT_S:-90} s"
    stalls=$((stalls + 1))
    break
  fi
done
cleanup
echo "stalls: $stalls"
