#!/usr/bin/env bash
# Tests packaging/linux/celestia-rs-auto-update.sh with stub curl, systemctl
# and installer: no network, no systemd. Runs on Linux and macOS.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
script=$here/celestia-rs-auto-update.sh
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fails=0
pass() { printf 'ok   %s\n' "$1"; }
fail() {
  printf 'FAIL %s\n' "$1"
  sed 's/^/     | /' "$work/out"
  fails=$((fails + 1))
}

# binary PATH VERSION: a fake celestia-rs that prints VERSION.
binary() {
  printf '#!/bin/sh\necho %s\n' "$2" >"$1"
  chmod 755 "$1"
}

mkdir -p "$work/tools"
# curl: channel files, the installer at a tag, and the node's /status.
cat >"$work/tools/curl" <<'EOF'
#!/usr/bin/env bash
url=${!#}
echo "curl $url" >>"$T/calls"
case $url in
  */channels/*) [[ -f $T/channel ]] || exit 22; cat "$T/channel" ;;
  */scripts/install.sh)
    # A fake installer: installs a binary reporting $T/built-version.
    cat <<INSTALLER
set -eu
[ "\$1 \$2" = "--version \$(cat $T/channel)" ] || { echo "bad args: \$*" >&2; exit 1; }
mkdir -p "\$CELESTIA_RS_HOME/bin"
printf '#!/bin/sh\necho %s\n' "\$(cat $T/built-version)" >"\$CELESTIA_RS_HOME/bin/celestia-rs"
chmod 755 "\$CELESTIA_RS_HOME/bin/celestia-rs"
INSTALLER
    ;;
  */status)
    h=$(cat "$T/height" 2>/dev/null || echo 100)
    [[ -f $T/stuck ]] || echo $((h + 1)) >"$T/height"
    printf '{"result":{"sync_info":{"latest_block_height":"%s","catching_up":false}}}\n' "$h"
    ;;
  *) echo "unexpected URL $url" >&2; exit 1 ;;
esac
EOF
# shellcheck disable=SC2016 # expanded by the stub
printf '#!/bin/sh\necho "systemctl $*" >>"$T/calls"\n' >"$work/tools/systemctl"
chmod 755 "$work/tools/curl" "$work/tools/systemctl"

# reset: node runs 0.1.0-main.1; the channel names v0.1.0-main.2, whose
# build reports 0.1.0-main.2.
reset() {
  rm -rf "$work/t"
  mkdir -p "$work/t/bin"
  binary "$work/t/bin/celestia-rs" 0.1.0-main.1
  echo v0.1.0-main.2 >"$work/t/channel"
  echo 0.1.0-main.2 >"$work/t/built-version"
}

run() {
  set +e
  T=$work/t PATH="$work/tools:$PATH" CHANNEL=mocha BIN=$work/t/bin/celestia-rs \
    STATE=$work/t/state MIN_BLOCKS=3 HEALTH_TIMEOUT=5 HEALTH_POLL=0 \
    bash "$script" "$@" >"$work/out" 2>&1
  status=$?
  set -e
  return $status
}
runs() { "$work/t/bin/celestia-rs"; }
restarts() { grep -c '^systemctl restart celestia-rs$' "$work/t/calls" 2>/dev/null || true; }

t="installs the channel's release and restarts the service"
reset
if run && [ "$(runs)" = 0.1.0-main.2 ] && [ "$(restarts)" = 1 ] &&
  [ "$("$work/t/bin/celestia-rs.prev")" = 0.1.0-main.1 ] &&
  grep -q 'raw.githubusercontent.com/celestiaorg/celestia-rs/v0.1.0-main.2/scripts/install.sh' "$work/t/calls"; then
  pass "$t"
else fail "$t"; fi

t="already on the channel's release: no install, no restart"
if run && grep -q "up to date" "$work/out" && [ "$(restarts)" = 1 ]; then pass "$t"; else fail "$t"; fi

t="an unhealthy node is rolled back and the release is not retried"
reset
touch "$work/t/stuck"
if ! run && [ "$(runs)" = 0.1.0-main.1 ] && [ "$(restarts)" = 2 ] &&
  grep -qx v0.1.0-main.2 "$work/t/state/bad-tags"; then
  pass "$t"
else fail "$t"; fi
rm "$work/t/stuck"
if run && grep -q "rolled back before" "$work/out" && [ "$(runs)" = 0.1.0-main.1 ]; then
  pass "$t (next pass)"
else fail "$t (next pass)"; fi

t="a build that reports another version never replaces the binary"
reset
echo 0.1.0-main.3 >"$work/t/built-version"
if ! run && [ "$(runs)" = 0.1.0-main.1 ] && [ "$(restarts)" = 0 ]; then pass "$t"; else fail "$t"; fi

t="an invalid or unreadable channel changes nothing"
reset
echo '../main' >"$work/t/channel"
if ! run && grep -q "invalid release tag" "$work/out" && [ "$(restarts)" = 0 ]; then
  pass "$t"
else fail "$t"; fi
rm "$work/t/channel"
if ! run && grep -q "cannot read channel" "$work/out" && [ "$(restarts)" = 0 ]; then
  pass "$t (unreadable)"
else fail "$t (unreadable)"; fi

t="DRY_RUN installs nothing"
reset
if DRY_RUN=1 run && [ "$(runs)" = 0.1.0-main.1 ] && [ "$(restarts)" = 0 ] &&
  grep -q "would update" "$work/out"; then
  pass "$t"
else fail "$t"; fi

echo
if ((fails > 0)); then
  echo "$fails scenario(s) failed"
  exit 1
fi
echo "all scenarios passed"
