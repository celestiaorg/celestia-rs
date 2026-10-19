#!/usr/bin/env bash
# Builds the Go binaries the devnet scripts need into $DEVNET_BIN (default
# target/devnet-bin) from module-proxy sources: celestia-appd (celestia-app
# v10.4.0) and cometbft (celestia-core v0.42.3), like devnet.sh does.
# Module downloads are retried (transient proxy.golang.org failures).
# APP_VERSION=v9.0.8 builds that release instead (module .../v9);
# APPD_ONLY=1 skips cometbft.
#
# usage: [DEVNET_BIN=dir] [APP_VERSION=vX.Y.Z] [APPD_ONLY=1] conformance/devnet/build-bins.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
APP_VERSION=${APP_VERSION:-v10.4.0}
CORE_VERSION=${CORE_VERSION:-v0.42.3}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$BIN"

retry() { # <cmd...>: up to 5 attempts with backoff
  local n
  for n in 1 2 3 4 5; do
    "$@" && return 0
    echo "attempt $n of '$*' failed; retrying in $((n * 10))s" >&2
    sleep $((n * 10))
  done
  return 1
}

build_go() { # <module@version> <pkg> <name>
  [ -x "$BIN/$3" ] && return
  local src
  src=$(cd "$ROOT/conformance/devnet-oracle" && retry go mod download -json "$1" | sed -n 's/.*"Dir": "\(.*\)",/\1/p')
  rm -rf "$WORK/src" && cp -r "$src" "$WORK/src" && chmod -R u+w "$WORK/src"
  (cd "$WORK/src" && retry env GOFLAGS=-mod=mod go mod download && GOFLAGS=-mod=mod go build -o "$BIN/$3" "$2")
}
APP_MAJOR=${APP_VERSION%%.*}
build_go "github.com/celestiaorg/celestia-app/$APP_MAJOR@$APP_VERSION" ./cmd/celestia-appd celestia-appd
[ -n "${APPD_ONLY:-}" ] || build_go "github.com/celestiaorg/celestia-core@$CORE_VERSION" ./cmd/cometbft cometbft
ls -l "$BIN"
