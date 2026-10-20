#!/usr/bin/env bash
# Tests for the health check of `celestia-rs-auto-update.sh` (the `check`
# mode, which runs the same check as a deploy) with curl and the app-hash
# check stubbed. Run: packaging/macos/test-health.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
mkdir -p "$t/bin" "$t/state" "$t/node"
fails=0

# --- stubs -------------------------------------------------------------------
# curl answers for the local mocha node only; mainnet is "down", so `check`
# skips it. /status reports $t/node/height and then advances it by one
# block; /commit?height=N has the validator's signature unless N is listed
# in $t/node/missed. Voting power is $t/node/power.
cat >"$t/bin/curl" <<EOF
#!/usr/bin/env bash
url=\${!#}
[[ \$url == http://127.0.0.1:26657/* ]] || exit 7
case \$url in
*/status)
  h=\$(cat $t/node/height)
  echo \$((h + 1)) >$t/node/height
  echo "{\"result\":{\"sync_info\":{\"latest_block_height\":\"\$h\",\"catching_up\":false},\"validator_info\":{\"address\":\"VAL\",\"voting_power\":\"\$(cat $t/node/power)\"}}}" ;;
*/commit\?height=*)
  n=\${url##*=}
  a=VAL
  grep -qx "\$n" $t/node/missed 2>/dev/null && a=OTHER
  echo "{\"result\":{\"signed_header\":{\"commit\":{\"signatures\":[{\"validator_address\":\"\$a\"}]}}}}" ;;
*) exit 7 ;;
esac
EOF
printf '#!/usr/bin/env bash\nexit 1\n' >"$t/bin/celestia-rs"
cp "$t/bin/celestia-rs" "$t/bin/celestia-rs-mainnet"
chmod +x "$t/bin/"*

check_mode() { # check_mode <description> <expected exit>
  local rc=0
  env PATH="$t/bin:$PATH" CELESTIA_RS_DIR="$t" BIN="$t/bin" STATE="$t/state" \
    CHECK_APPHASH=true MIN_BLOCKS=2 SIGN_WINDOW=20 MIN_SIGNED=15 HEALTH_TIMEOUT=60 HEALTH_POLL=0 \
    "$here/celestia-rs-auto-update.sh" check >>"$t/log" 2>&1 || rc=$?
  # Exit 1 means some node is unhealthy; mainnet is always down here, so
  # look at mocha's verdict in the log.
  if grep -q "mocha: healthy" <(tail -n 3 "$t/log"); then rc=0; else rc=1; fi
  if [[ $rc == "$2" ]]; then echo "ok: $1"; else
    echo "FAIL: $1 (expected $2, got $rc)"; fails=$((fails + 1))
  fi
}

# --- cases -------------------------------------------------------------------

# A restart costs a validator the blocks it was down for and the ones it
# block-synced: with Mocha's ~3 s blocks and launchd's 30 s throttle, ~12.
# Those were missed before the check began and say nothing about the new
# binary.
echo 1000 >"$t/node/height"
echo 1 >"$t/node/power"
seq 991 1002 >"$t/node/missed"
check_mode "blocks missed before the node was back in consensus do not count" 0

# Missing blocks once it is back in consensus is unhealthy.
echo 1000 >"$t/node/height"
seq 1003 1014 >"$t/node/missed"
check_mode "blocks missed after the node is back in consensus count" 1

# A node without voting power does not sign.
echo 1000 >"$t/node/height"
echo 0 >"$t/node/power"
check_mode "a node without voting power is healthy without signing" 0

if ((fails)); then
  echo "--- log"; cat "$t/log"
  echo "$fails failed"; exit 1
fi
echo "all passed"
