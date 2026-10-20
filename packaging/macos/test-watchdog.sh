#!/usr/bin/env bash
# Tests for `celestia-rs-auto-update.sh watchdog` with curl and sudo
# stubbed on PATH. Run: packaging/macos/test-watchdog.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/rpc" "$t/state"
fails=0

# --- stubs -------------------------------------------------------------------
# curl answers /status from $t/rpc/<host:port>: "<height> <catching_up>", or
# fails when the file is missing (node or reference RPC down).
cat >"$t/bin/curl" <<EOF
#!/usr/bin/env bash
url=\${!#}
key=\${url#*://}; key=\${key%%/*}
f=$t/rpc/\$key
[[ -f \$f ]] || exit 7
read -r h cu <"\$f"
echo "{\"result\":{\"sync_info\":{\"latest_block_height\":\"\$h\",\"catching_up\":\$cu}}}"
EOF
# git fails, so a pass that falls through to a deploy stops at once.
printf '#!/usr/bin/env bash\nexit 1\n' >"$t/bin/git"
# sudo records each command line instead of running it.
cat >"$t/bin/sudo" <<EOF
#!/usr/bin/env bash
echo "\$*" >>$t/restarts
EOF
chmod +x "$t/bin/"*

mocha() { echo "$1 ${2:-false}" >"$t/rpc/127.0.0.1:26657"; }
mainnet() { echo "$1 ${2:-false}" >"$t/rpc/127.0.0.1:26757"; }
mocha_ref() { echo "$1 false" >"$t/rpc/rpc.celestia-mocha.com"; }
mainnet_ref() { echo "$1 false" >"$t/rpc/rpc.celestia.pops.one"; }

run() { # run [VAR=value...]: one watchdog pass
  local rc=0
  env PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" STATE="$t/state" WATCHDOG_MAX_LAG=50 \
    WATCHDOG_STRIKES=3 WATCHDOG_COOLDOWN=1800 "$@" \
    "$here/celestia-rs-auto-update.sh" watchdog >>"$t/log" 2>&1 || rc=$?
  if ((rc)); then echo "FAIL: watchdog pass exited $rc"; fails=$((fails + 1)); fi
}
restarts() { if [[ -f $t/restarts ]]; then wc -l <"$t/restarts" | tr -d ' '; else echo 0; fi; }
check() { # check <description> <expected> <actual>
  if [[ $2 != "$3" ]]; then echo "FAIL: $1: expected '$2', got '$3'"; fails=$((fails + 1)); fi
}
reset() { rm -rf "$t/state" "$t/restarts" "$t/log"; mkdir -p "$t/state"; }

# --- in sync: nothing happens ---------------------------------------------------
reset
mocha 1000; mocha_ref 1010; mainnet 5000; mainnet_ref 5001
for _ in 1 2 3 4; do run; done
check "in sync: no restart" 0 "$(restarts)"

# --- behind for WATCHDOG_STRIKES passes: restart that node only -----------------
reset
mocha 1000; mocha_ref 1100
run; run
check "behind 2 passes: no restart yet" 0 "$(restarts)"
run
check "behind 3 passes: one restart" 1 "$(restarts)"
check "restarts the mocha label" "-n /bin/launchctl kickstart -k system/org.celestia.celestia-rs" "$(head -1 "$t/restarts")"
grep -q "mocha: 100 blocks behind" "$t/log" || { echo "FAIL: lag not logged"; fails=$((fails + 1)); }

# --- cooldown: no second restart within WATCHDOG_COOLDOWN -----------------------
run; run; run
check "cooldown: still one restart" 1 "$(restarts)"
# ... and another one once the cooldown has passed.
run WATCHDOG_COOLDOWN=0; run WATCHDOG_COOLDOWN=0; run WATCHDOG_COOLDOWN=0
check "after cooldown: second restart" 2 "$(restarts)"

# --- a pass back in sync resets the strikes ------------------------------------
reset
mocha 1000; mocha_ref 1100
run; run
mocha 1095
run
mocha 1000
run; run
check "strikes reset by an in-sync pass" 0 "$(restarts)"

# --- block-syncing (catching_up) is left alone ---------------------------------
reset
mocha 1000 true; mocha_ref 5000
for _ in 1 2 3 4; do run; done
check "catching up: no restart" 0 "$(restarts)"

# --- local or reference RPC down: no restart -----------------------------------
reset
rm -f "$t/rpc/127.0.0.1:26657"; mocha_ref 5000
for _ in 1 2 3 4; do run; done
check "local RPC down: no restart" 0 "$(restarts)"
mocha 1000; rm -f "$t/rpc/rpc.celestia-mocha.com"
for _ in 1 2 3 4; do run; done
check "reference RPC down: no restart" 0 "$(restarts)"

# --- a deploy in progress (auto-update lock) is left alone ---------------------
reset
mocha 1000; mocha_ref 5000
mkdir "$t/state/lock"
for _ in 1 2 3 4; do run; done
check "deploy lock held: no restart" 0 "$(restarts)"
[[ -d $t/state/lock ]] || { echo "FAIL: watchdog removed the deploy lock"; fails=$((fails + 1)); }
# A lock older than WATCHDOG_LOCK_STALE is from a pass that was killed.
for _ in 1 2 3; do run WATCHDOG_LOCK_STALE=0; done
check "stale deploy lock: restart" 1 "$(restarts)"

# --- mainnet uses its own label ------------------------------------------------
reset
mocha 1000; mocha_ref 1000; mainnet 5000; mainnet_ref 9000
run; run; run
check "mainnet restart" "-n /bin/launchctl kickstart -k system/org.celestia.celestia-rs-mainnet" "$(cat "$t/restarts")"

if ((fails)); then echo "$fails failure(s)"; exit 1; fi
echo "all watchdog tests passed"
