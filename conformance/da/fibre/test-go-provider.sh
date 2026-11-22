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
overlay.write_text(fixture.replace(signature, "serverCfg.StoreFn = func(_ context.Context, sc appfibre.StoreConfig)"))
(work / "overlay.json").write_text(json.dumps({"Replace": {str(source): str(overlay)}}))
PY
if ! go test -p 2 -tags fibre_e2e -overlay "$work/overlay.json" -timeout 12m -count=1 -v -run '^TestFibreE2ESuite$' ./fibre >"$work/control.log" 2>&1; then
    # Keep ephemeral node keys and transaction payloads in the temporary directory.
    grep -E '(^#|^FAIL|--- FAIL:|fibre_e2e_test.go:[[:digit:]]+|Error:)' "$work/control.log" | tail -30 >&2 || true
    exit 1
fi
grep -E '(--- PASS: TestFibreE2ESuite|^ok[[:space:]])' "$work/control.log"
