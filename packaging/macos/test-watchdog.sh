#!/usr/bin/env bash
# Tests for `celestia-rs-auto-update.sh watchdog` with curl, sudo and the
# node binaries stubbed. Run: packaging/macos/test-watchdog.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/rpc" "$t/state" "$t/val"
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
# The node binaries answer from $t/val/<home name>.{jailed,until,tombstoned}
# and record each unjail tx, with the passphrase they were given, in
# $t/unjails. Like the real keyrings, mocha's is `test` (no passphrase; a
# set CELESTIA_RS_KEYRING_PASSPHRASE locks it) and mainnet's is `file`
# (CELESTIA_RS_KEYRING_PASSPHRASE, else a line on stdin; EOF fails).
cat >"$t/bin/celestia-rs" <<EOF
#!/usr/bin/env bash
home=; prev=
for a in "\$@"; do [[ \$prev == --home ]] && home=\$a; prev=\$a; done
n=\$(basename "\$home")
pass=none
if [[ "\$1 \$2" != "keys show" && \$1 != tx ]]; then
  : # queries never open the keyring
elif [[ \$n == mocha ]]; then
  [[ -z \${CELESTIA_RS_KEYRING_PASSPHRASE+x} ]] || exit 1
elif [[ -n \${CELESTIA_RS_KEYRING_PASSPHRASE+x} ]]; then
  pass=\$CELESTIA_RS_KEYRING_PASSPHRASE
else
  IFS= read -r pass || exit 1
fi
case "\$1 \$2" in
"keys show") echo "celestiavaloper1\$n" ;;
"comet show-address") echo "celestiavalcons1\$n" ;;
"query staking") echo "{\"validator\":{\"jailed\":\$(cat $t/val/\$n.jailed 2>/dev/null || echo false)}}" ;;
"query slashing") echo "{\"val_signing_info\":{\"jailed_until\":\"\$(cat $t/val/\$n.until 2>/dev/null || echo 1970-01-01T00:00:00Z)\",\"tombstoned\":\$(cat $t/val/\$n.tombstoned 2>/dev/null || echo false)}}" ;;
"tx slashing")
  echo "\$n \$* pass=\$pass" >>$t/unjails
  [[ -f $t/val/\$n.txfail ]] && exit 1
  echo false >$t/val/\$n.jailed ;;
*) exit 1 ;;
esac
EOF
cp "$t/bin/celestia-rs" "$t/bin/celestia-rs-mainnet"
chmod +x "$t/bin/"*

mocha() { echo "$1 ${2:-false}" >"$t/rpc/127.0.0.1:26657"; }
mainnet() { echo "$1 ${2:-false}" >"$t/rpc/127.0.0.1:26757"; }
mocha_ref() { echo "$1 false" >"$t/rpc/rpc.celestia-mocha.com"; }
mainnet_ref() { echo "$1 false" >"$t/rpc/rpc.celestia.pops.one"; }

run() { # run [VAR=value...]: one watchdog pass
  local rc=0
  env PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" BIN="$t/bin" STATE="$t/state" \
    WATCHDOG_MAX_LAG=50 WATCHDOG_STRIKES=3 WATCHDOG_COOLDOWN=1800 UNJAIL_COOLDOWN=600 "$@" \
    "$here/celestia-rs-auto-update.sh" watchdog >>"$t/log" 2>&1 || rc=$?
  if ((rc)); then echo "FAIL: watchdog pass exited $rc"; fails=$((fails + 1)); fi
}
restarts() { if [[ -f $t/restarts ]]; then wc -l <"$t/restarts" | tr -d ' '; else echo 0; fi; }
unjails() { if [[ -f $t/unjails ]]; then wc -l <"$t/unjails" | tr -d ' '; else echo 0; fi; }
check() { # check <description> <expected> <actual>
  if [[ $2 != "$3" ]]; then echo "FAIL: $1: expected '$2', got '$3'"; fails=$((fails + 1)); fi
}
reset() {
  rm -rf "$t/state" "$t/restarts" "$t/log" "$t/unjails" "$t/val" "$t/unjail"
  mkdir -p "$t/state" "$t/val"
}

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

# --- unjail: only nodes with $CELESTIA_RS_DIR/unjail/<name> ------------------
reset
mocha 1000; mocha_ref 1000; mainnet 5000; mainnet_ref 5000
echo true >"$t/val/mocha.jailed"; echo true >"$t/val/mainnet.jailed"
run
check "unjail not opted in: no tx" 0 "$(unjails)"
mkdir -p "$t/unjail"; : >"$t/unjail/mocha"; printf 's3cret\n' >"$t/unjail/mainnet"
run
check "unjail: one tx per jailed node" 2 "$(unjails)"
grep -q "^mocha tx slashing unjail --from operator --home $t/mocha .*pass=none$" "$t/unjails" ||
  { echo "FAIL: mocha unjail command: $(cat "$t/unjails")"; fails=$((fails + 1)); }
grep -q "^mainnet tx slashing unjail --from operator --home $t/mainnet .*pass=s3cret$" "$t/unjails" ||
  { echo "FAIL: mainnet unjail passphrase: $(cat "$t/unjails")"; fails=$((fails + 1)); }
grep -q "mainnet: unjailed" "$t/log" || { echo "FAIL: unjail not logged"; fails=$((fails + 1)); }
run
check "unjailed: no further tx" 2 "$(unjails)"

# --- an empty file passes an empty passphrase to a `file` keyring --------------
reset
mocha 1000; mocha_ref 1000; mainnet 5000; mainnet_ref 5000
echo true >"$t/val/mainnet.jailed"
mkdir -p "$t/unjail"; : >"$t/unjail/mainnet"
run
grep -q "^mainnet tx slashing unjail .*pass=$" "$t/unjails" ||
  { echo "FAIL: mainnet empty passphrase: $(cat "$t/unjails" 2>/dev/null) $(cat "$t/log")"; fails=$((fails + 1)); }

# --- unjail waits until the node is in sync -------------------------------------
reset
mkdir -p "$t/unjail"; : >"$t/unjail/mocha"
echo true >"$t/val/mocha.jailed"
mocha 1000; mocha_ref 1100
run
check "behind: no unjail" 0 "$(unjails)"
mocha 1000 true
run
check "catching up: no unjail" 0 "$(unjails)"
mocha 1000; rm -f "$t/rpc/rpc.celestia-mocha.com"
run
check "reference RPC down: no unjail" 0 "$(unjails)"
mocha_ref 1000
run
check "in sync: unjail" 1 "$(unjails)"

# --- jail period not over, tombstoned: no tx ------------------------------------
reset
mkdir -p "$t/unjail"; : >"$t/unjail/mocha"
mocha 1000; mocha_ref 1000
echo true >"$t/val/mocha.jailed"; echo "2999-01-01T00:00:00.5Z" >"$t/val/mocha.until"
run
check "jailed_until in the future: no unjail" 0 "$(unjails)"
echo "1970-01-01T00:00:00Z" >"$t/val/mocha.until"; echo true >"$t/val/mocha.tombstoned"
run
check "tombstoned: no unjail" 0 "$(unjails)"
grep -q "mocha: tombstoned" "$t/log" || { echo "FAIL: tombstone not logged"; fails=$((fails + 1)); }

# --- a failed unjail is retried after UNJAIL_COOLDOWN ---------------------------
reset
mkdir -p "$t/unjail"; : >"$t/unjail/mocha"
mocha 1000; mocha_ref 1000
echo true >"$t/val/mocha.jailed"; touch "$t/val/mocha.txfail"
run; run
check "failed unjail: no retry within cooldown" 1 "$(unjails)"
grep -q "mocha: unjail failed" "$t/log" || { echo "FAIL: unjail failure not logged"; fails=$((fails + 1)); }
run UNJAIL_COOLDOWN=0
check "failed unjail: retry after cooldown" 2 "$(unjails)"

if ((fails)); then echo "$fails failure(s)"; exit 1; fi
echo "all watchdog tests passed"
