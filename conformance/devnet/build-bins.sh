#!/usr/bin/env bash
# Builds the Go binaries the devnet scripts need into $DEVNET_BIN (default
# target/devnet-bin) from module-proxy sources: celestia-appd (celestia-app
# v10.5.0) and cometbft (celestia-core v0.42.3), like devnet.sh does.
# Module downloads are retried (transient proxy.golang.org failures).
# APP_VERSION=v9.0.8 builds that release instead (module .../v9);
# APPD_ONLY=1 skips cometbft.
#
# usage: [DEVNET_BIN=dir] [APP_VERSION=vX.Y.Z] [APPD_ONLY=1] conformance/devnet/build-bins.sh
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
# The Go release affects consensus-visible JSON (including amino sign
# bytes). Do not silently build the oracles with a newer installed Go.
# setup-go supplies GOTOOLCHAIN=local in CI after selecting this version.
export GOTOOLCHAIN=${GOTOOLCHAIN:-$(awk '$1 == "toolchain" { print $2; exit }' "$ROOT/conformance/devnet-oracle/go.mod")}
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
APP_VERSION=${APP_VERSION:-v10.5.0}
CORE_VERSION=${CORE_VERSION:-v0.42.3}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
mkdir -p "$BIN"
# Absolute: `go build -o` runs from the module's source directory.
BIN=$(cd "$BIN" && pwd)

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
  # An executable alone may belong to the previous parity target. A
  # version stamp is written only after a successful build.
  if [ -x "$BIN/$3" ] && [ "$(cat "$BIN/$3.module" 2>/dev/null)" = "$1" ]; then
    return
  fi
  local src
  src=$(cd "$ROOT/conformance/devnet-oracle" && retry go mod download -json "$1" | sed -n 's/.*"Dir": "\(.*\)",/\1/p')
  rm -rf "$WORK/src" && cp -r "$src" "$WORK/src" && chmod -R u+w "$WORK/src"
  # Module-proxy archives have no VCS metadata. Do not stamp a surrounding
  # workspace's repository (or fail when its .git is inaccessible).
  (cd "$WORK/src" && retry env GOFLAGS=-mod=mod go mod download && GOFLAGS=-mod=mod go build -buildvcs=false -o "$BIN/$3" "$2")
  printf '%s\n' "$1" >"$BIN/$3.module"
}
APP_MAJOR=${APP_VERSION%%.*}
build_go "github.com/celestiaorg/celestia-app/$APP_MAJOR@$APP_VERSION" ./cmd/celestia-appd celestia-appd
[ -n "${APPD_ONLY:-}" ] || build_go "github.com/celestiaorg/celestia-core@$CORE_VERSION" ./cmd/cometbft cometbft
ls -l "$BIN"
