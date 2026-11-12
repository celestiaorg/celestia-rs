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

download_module() {
  # Each retry must replace the prior JSON error response, not append it.
  go mod download -json "$1" > "$WORK/module.json"
}

build_go() { # <module@version> <pkg> <name>
  local src source_commit tags="" ldflags="" version compiler
  local -a flags=()
  (cd "$ROOT/conformance/devnet-oracle" && retry download_module "$1")
  src=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["Dir"])' "$WORK/module.json")
  source_commit=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["Origin"]["Hash"])' "$WORK/module.json")
  compiler=$(go version)
  go env -json GOOS GOARCH CGO_ENABLED GOAMD64 GOARM GOEXPERIMENT GOFIPS140 > "$WORK/go-env.json"
  if [ "$3" = celestia-appd ]; then
    # Match the pinned app Makefile's standalone recipe. A plain go build
    # omits ledger-linked descriptors and leaves public app versions empty.
    # This deliberately excludes the multiplexer and its historical binaries.
    version=${1##*@}; version=${version#v}
    tags=ledger
    ldflags="-X github.com/cosmos/cosmos-sdk/version.Name=celestia-app -X github.com/cosmos/cosmos-sdk/version.AppName=celestia-appd -X github.com/cosmos/cosmos-sdk/version.Version=$version -X github.com/cosmos/cosmos-sdk/version.Commit=$source_commit -X github.com/cosmos/cosmos-sdk/version.BuildTags=$tags -X ${1%@*}/cmd/celestia-appd/cmd.v2UpgradeHeight=0"
    flags=(-tags="$tags" -ldflags="$ldflags")
  fi
  python3 - "$WORK/module.json" "$compiler" "$tags" "$ldflags" "$2" "$WORK/expected.json" "$WORK/go-env.json" <<'PY'
import json, sys
module = json.load(open(sys.argv[1]))
recipe = {"schema_version": 1, "module": module["Path"] + "@" + module["Version"],
          "source_commit": module["Origin"]["Hash"], "module_sum": module["Sum"],
          "compiler": sys.argv[2], "build_tags": sys.argv[3].split(",") if sys.argv[3] else [],
          "ldflags": sys.argv[4], "package": sys.argv[5], "buildvcs": False,
          "goflags": "-mod=mod", "target": json.load(open(sys.argv[7]))}
with open(sys.argv[6], "w") as out:
    json.dump(recipe, out, sort_keys=True, indent=2)
PY
  # Recipe and content must both match: module-only stamps could silently
  # reuse an untagged binary or one made with a different compiler.
  if [ -x "$BIN/$3" ] && python3 - "$WORK/expected.json" "$BIN/$3" <<'PY'
import hashlib, json, pathlib, sys
try:
    expected = json.load(open(sys.argv[1]))
    binary = pathlib.Path(sys.argv[2])
    actual = json.loads(binary.with_name(binary.name + ".build.json").read_text())
    digest = actual.pop("binary_sha256")
    sys.exit(0 if actual == expected and hashlib.sha256(binary.read_bytes()).hexdigest() == digest else 1)
except (OSError, ValueError, KeyError):
    sys.exit(1)
PY
  then
    return
  fi
  rm -rf "$WORK/src" && cp -r "$src" "$WORK/src" && chmod -R u+w "$WORK/src"
  # Module-proxy archives have no VCS metadata. Do not stamp a surrounding
  # workspace's repository (or fail when its .git is inaccessible).
  (cd "$WORK/src" && retry env GOFLAGS=-mod=mod go mod download && GOFLAGS=-mod=mod go build -buildvcs=false "${flags[@]}" -o "$WORK/$3" "$2")
  if [ "$3" = celestia-appd ]; then
    # The app pre-instantiates its upgrade keeper even for `version`;
    # keep that command's filesystem effects in the temporary build home.
    [ "$(CELESTIA_APP_HOME="$WORK/version-home" "$WORK/$3" version)" = "$version" ] || { echo "oracle version stamp mismatch" >&2; return 1; }
  fi
  mv "$WORK/$3" "$BIN/$3"
  go version -m "$BIN/$3" > "$BIN/$3.build-info.txt"
  python3 - "$WORK/expected.json" "$BIN/$3" <<'PY'
import hashlib, json, pathlib, sys
recipe = json.load(open(sys.argv[1]))
binary = pathlib.Path(sys.argv[2])
recipe["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
binary.with_name(binary.name + ".build.json").write_text(json.dumps(recipe, sort_keys=True, indent=2) + "\n")
PY
  printf '%s\n' "$1" >"$BIN/$3.module"
}
APP_MAJOR=${APP_VERSION%%.*}
build_go "github.com/celestiaorg/celestia-app/$APP_MAJOR@$APP_VERSION" ./cmd/celestia-appd celestia-appd
[ -n "${APPD_ONLY:-}" ] || build_go "github.com/celestiaorg/celestia-core@$CORE_VERSION" ./cmd/cometbft cometbft
ls -l "$BIN"
