#!/usr/bin/env bash
# Capture only a freshly verified current v10.5.0 oracle in an isolated devnet.
# Usage: [DEVNET_BIN=...] [DEVNET_PORT_OFFSET=1300] record.sh <empty-output-dir>
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
OUT=${1:?usage: record.sh empty-output-directory}
mkdir -p "$OUT"
[ -z "$(ls -A "$OUT")" ] || { echo "output directory must be empty" >&2; exit 1; }
OUT=$(cd "$OUT" && pwd)
export GOTOOLCHAIN=${GOTOOLCHAIN:-$(awk '$1 == "toolchain" { print $2; exit }' "$ROOT/conformance/devnet-oracle/go.mod")}
export GOMEMLIMIT=${GOMEMLIMIT:-512MiB}
export DEVNET_BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
# This is the only public request, and it is read-only. Refuse to relabel an old
# oracle when the observed mainnet release has changed.
curl --fail --silent --show-error --connect-timeout 10 --max-time 30 \
  "${MAINNET_RPC:-https://rpc.celestia.pops.one}/abci_info" > "$OUT/mainnet-abci-info.json"
python3 - "$OUT/mainnet-abci-info.json" <<'PY'
import json, sys
version = json.load(open(sys.argv[1]))["result"]["response"]["version"]
assert version == "10.5.0", f"mainnet is {version}; select a new oracle explicitly"
PY
APP_VERSION=v10.5.0 CORE_VERSION=v0.42.3 APPD_ONLY=1 "$ROOT/conformance/devnet/build-bins.sh"
BIN=$(cd "$DEVNET_BIN" && pwd)
export PATH=$BIN:$PATH
python3 - "$BIN/celestia-appd" <<'PYBUILD'
import hashlib, json, pathlib, sys
binary = pathlib.Path(sys.argv[1])
manifest = binary.with_name(binary.name + ".build.json")
assert manifest.exists(), "oracle requires tagged build provenance; use updated conformance/devnet/build-bins.sh"
build = json.loads(manifest.read_text())
assert build["schema_version"] == 1 and build["module"] == "github.com/celestiaorg/celestia-app/v10@v10.5.0", build
assert build["binary_sha256"] == hashlib.sha256(binary.read_bytes()).hexdigest(), "oracle binary hash mismatch"
assert build["build_tags"] == ["ledger"], "oracle must use the pinned standalone ledger recipe"
assert "version.Version=10.5.0" in build["ldflags"] and "cmd.v2UpgradeHeight=0" in build["ldflags"], "oracle version/upgrade stamp missing"
PYBUILD
W=$(mktemp -d)
OFF=${DEVNET_PORT_OFFSET:-1300}
RS=unused
FAIL=()
# shellcheck source=../../devnet/lib-mixed.sh
source "$ROOT/conformance/devnet/lib-mixed.sh"
trap 'cleanup; rm -rf "$W"' EXIT
make_net "$W" rpc-empty-query-105 "go go" "5000000000 5000000000"
# Do not expose even the devnet listeners to the public network.
for i in 0 1; do
  sed -i.bak 's/0\.0\.0\.0:/127.0.0.1:/g' "$W/n$i/config/config.toml" "$W/n$i/config/app.toml"
  rm -f "$W/n$i/config/config.toml.bak" "$W/n$i/config/app.toml.bak"
  start_node "$W" "$i"
done
RPC=$(rpc "$W" 0)
ready=0
for attempt in $(seq 1 60); do
  if curl --fail --silent --max-time 1 "$RPC/health" > /dev/null; then ready=1; break; fi
  sleep 1
done
[ "$ready" = 1 ] || { echo "isolated oracle did not become healthy" >&2; exit 1; }
curl --fail --silent --max-time 5 "$RPC/abci_info" > "$OUT/oracle-abci-info.json"
python3 - "$OUT/oracle-abci-info.json" <<'PYVERSION'
import json, sys
assert json.load(open(sys.argv[1]))["result"]["response"]["version"] == "10.5.0", "running oracle is not version stamped"
PYVERSION
python3 "$ROOT/conformance/mainnet-rpc/empty-query/record.py" --rpc "$RPC" \
  --out "$OUT/empty-query.json" --binary "$BIN/celestia-appd"
# The standalone ABCI listener has its own fresh home, with no consensus or
# application state needed for the P2P-filter routes.
ABCI_PORT=$((20000 + OFF))
CELESTIA_APP_HOME="$W/abci" celestia-appd init empty-query-abci --chain-id rpc-empty-query-abci-105 --home "$W/abci" > /dev/null 2>&1
CELESTIA_APP_HOME="$W/abci" celestia-appd start --with-comet=false --address "tcp://127.0.0.1:$ABCI_PORT" --home "$W/abci" > "$W/abci.log" 2>&1 &
PIDS+=($!)
python3 - "$ABCI_PORT" <<'PYREADY'
import socket, sys, time
for _ in range(60):
    try:
        with socket.create_connection(("127.0.0.1", int(sys.argv[1])), 1):
            break
    except OSError:
        time.sleep(1)
else:
    raise SystemExit("standalone ABCI oracle did not become healthy")
PYREADY
python3 "$ROOT/conformance/mainnet-rpc/empty-query/record-abci.py" --abci "127.0.0.1:$ABCI_PORT" --binary "$BIN/celestia-appd" --out "$OUT/empty-query-abci.json"
python3 - "$ROOT" "$W/n0" "$OUT" "$BIN" <<'PY'
import hashlib, json, pathlib, subprocess, sys
root, home, out, bin_dir = map(pathlib.Path, sys.argv[1:])
capture = json.loads((out / "empty-query.json").read_text())
modules = {}
for name, module in [
    ("celestia_app", "github.com/celestiaorg/celestia-app/v10@v10.5.0"),
    ("celestia_core", "github.com/celestiaorg/celestia-core@v0.42.3"),
    ("cosmos_sdk", "github.com/celestiaorg/cosmos-sdk@v0.52.12"),
]:
    source = json.loads(subprocess.check_output(["go", "mod", "download", "-json", module], cwd=root / "conformance/devnet-oracle", text=True))
    modules[name] = {k: source[k] for k in ["Path", "Version", "Sum", "GoModSum", "Origin"]}
    if name != "celestia_app":
        assert source["Sum"] in capture["go_build_info"], f"wrong binary dependency: {module}"
build = json.loads((bin_dir / "celestia-appd.build.json").read_text())
assert build["binary_sha256"] == capture["binary_sha256"], "oracle changed during capture"
assert build["source_commit"] == modules["celestia_app"]["Origin"]["Hash"]
assert build["module_sum"] == modules["celestia_app"]["Sum"]
stamp = (bin_dir / "celestia-appd.module").read_text().strip()
assert stamp == "github.com/celestiaorg/celestia-app/v10@v10.5.0", stamp
meta = {
    "release": "v10.5.0", "sources": modules, "binary_sha256": capture["binary_sha256"],
    "go_build_info": capture["go_build_info"], "module_stamp": stamp, "build": build,
    "fixture_sha256": hashlib.sha256((out / "empty-query.json").read_bytes()).hexdigest(),
    "abci_fixture_sha256": hashlib.sha256((out / "empty-query-abci.json").read_bytes()).hexdigest(),
    "node_inputs_sha256": {p: hashlib.sha256((home / p).read_bytes()).hexdigest() for p in ["config/genesis.json", "config/config.toml", "config/app.toml"]},
    "capture_sources_sha256": {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in ["conformance/mainnet-rpc/empty-query/record.py", "conformance/mainnet-rpc/empty-query/record-abci.py", "conformance/mainnet-rpc/empty-query/record.sh", "conformance/devnet/build-bins.sh", "conformance/devnet/lib-mixed.sh"]},
    "chain_id": json.loads((home / "config/genesis.json").read_text())["chain_id"],
    "mainnet_release_observation": json.loads((out / "mainnet-abci-info.json").read_text()),
    "scope": "default no-callback stateless P2P filter routes through the actual embedded local Comet listener",
    "normalizations": ["HTTP Date and X-Server-Time values are dynamic", "WebSocket accept depends on client key; opcode/payload compared and raw wire retained"],
    "excluded": ["custom peer-filter callbacks", "Cosmos ABCIQuery wrapper", "proxy/TLS", "subscriptions/resource-load", "official multiplexer release binary equivalence and pre-v10 historical routing"],
}
(out / "empty-query-meta.json").write_text(json.dumps(meta, indent=2) + "\n")
PY
printf 'Capture and provenance saved to %s\n' "$OUT"
