#!/bin/bash
# Run in the worker's checked-out source directory. The controller supplies
# the full source revision and the digest of an independently verified build.
set -euo pipefail
ref=$1 digest=$2
[[ $ref =~ ^[0-9a-f]{40}$ && $digest =~ ^[0-9a-f]{64}$ ]]
test "$(git rev-parse HEAD)" = "$ref"
mkdir -p target/release
tmp=$(mktemp target/release/.celestia-rs.XXXXXX)
trap 'rm -f "$tmp"' EXIT
vmx-gcs get "bin/$ref/celestia-rs-linux-amd64" "$tmp"
printf '%s  %s\n' "$digest" "$tmp" | sha256sum -c -
chmod 755 "$tmp"
version=$("$tmp" version --long)
grep -Fxq "commit: $ref" <<<"$version"
# Source builds install jq along with the compiler. The replay scripts
# also need it when the worker starts from a stock Ubuntu image.
if ! command -v jq >/dev/null; then
  apt-get -o DPkg::Lock::Timeout=600 update -q >/dev/null
  apt-get -o DPkg::Lock::Timeout=600 install -yq jq >/dev/null
fi
mv "$tmp" target/release/celestia-rs
