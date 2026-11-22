#!/usr/bin/env bash
# Run the release's node-service, app and on-chain provider discovery on loopback.
set -euo pipefail
node=$(realpath "${1:?pass the pinned celestia-node checkout}")
revision=$(git -C "$node" rev-parse HEAD)
case "$revision" in
    d5dfb21a0885622828e30e8e2daf50e5d1bb72b2|af9897bb471959660a4179048656694f3013b2f5) ;;
    *) echo "unexpected node revision $revision" >&2; exit 1 ;;
esac
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
if [[ ${2:-} == --rust-control ]]; then
    mkdir -p "$work/probe/src"
    cp "$(dirname "$0")/download_control.rs" "$work/probe/src/main.rs"
    cat >"$work/probe/Cargo.toml" <<'TOML'
[package]
name = "fibre-download-control"
version = "0.0.0"
edition = "2024"

[dependencies]
celestia-fibre = "=1.3.0"
hex = "0.4"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
TOML
    cargo build --manifest-path "$work/probe/Cargo.toml" --target-dir "${CARGO_TARGET_DIR:-$work/target}"
    export RUST_FIBRE_PROBE
    RUST_FIBRE_PROBE=$(realpath "${CARGO_TARGET_DIR:-$work/target}/debug/fibre-download-control")
elif [[ ${2:-} == --rust-node ]]; then
    export DA_FIBRE_NODE DA_FIBRE_GO_CLIENT RUST_FIBRE_PROBE
    DA_FIBRE_NODE=$(realpath "${3:?path to celestia-rs built with da-fibre}")
    DA_FIBRE_GO_CLIENT="$work/go-download-client"
    RUST_FIBRE_PROBE=$(realpath "$(dirname "$0")/test-light-download.sh")
    client=$(realpath "$(dirname "$0")/node_download.go")
    (cd "$node" && go build -o "$DA_FIBRE_GO_CLIENT" "$client")
fi
cd "$node/nodebuilder/tests/tastora"
# Both node tags' suite supplies the older memory-store factory signature.
# Adapt only that test callback to v11's context argument through a Go overlay.
python3 - "$PWD/fibre/fibre_e2e_test.go" "$work" <<'PY'
import json
import sys
from pathlib import Path
source, work = map(Path, sys.argv[1:])
fixture = source.read_text()
signature = "serverCfg.StoreFn = func(sc appfibre.StoreConfig)"
if fixture.count(signature) != 1:
    raise SystemExit("unexpected pinned Fibre test callback")
overlay = work / "fibre_e2e_test.go"
fixture = fixture.replace('"encoding/json"', '"encoding/json"\n "encoding/hex"\n "os"\n "os/exec"')
needle = 'require.Equal(t, data, got)'
assert fixture.count(needle) == 1
fixture = fixture.replace(needle, needle + """
    if probe := os.Getenv("RUST_FIBRE_PROBE"); probe != "" {
    output := filepath.Join(t.TempDir(), "downloaded")
    command := exec.CommandContext(ctx, probe, "http://"+s.conn.Target(), s.cctx.ChainID, hex.EncodeToString(blobID[:]), output)
    detail, err := command.CombinedOutput()
    require.NoError(t, err, string(detail))
    actual, err := os.ReadFile(output)
    require.NoError(t, err)
    require.Equal(t, data, actual)
    t.Log("Rust Fibre client returned identical 32768 Go-uploaded bytes")
    }
""")
overlay.write_text(fixture.replace(signature, "serverCfg.StoreFn = func(_ context.Context, sc appfibre.StoreConfig)"))
(work / "overlay.json").write_text(json.dumps({"Replace": {str(source): str(overlay)}}))
PY
if ! go test -p 2 -tags fibre_e2e -overlay "$work/overlay.json" -timeout 12m -count=1 -v -run '^TestFibreE2ESuite$' ./fibre >"$work/control.log" 2>&1; then
    # Keep ephemeral node keys and transaction payloads in the temporary directory.
    grep -E '(^#|^FAIL|--- FAIL:|fibre_e2e_test.go:[[:digit:]]+|Error:)' "$work/control.log" | tail -30 >&2 || true
    exit 1
fi
grep -E '(Rust Fibre client returned|--- PASS: TestFibreE2ESuite|^ok[[:space:]])' "$work/control.log"
