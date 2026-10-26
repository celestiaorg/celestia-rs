#!/usr/bin/env bash
# Generate historical ante fixtures with the pinned v9 oracle dependency
# graph. The Go generator sources are shared; only the app import changes.
# Usage: conformance/antevectors/run-v9.sh <out-dir> [scenario]
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
mkdir -p "${1:?out-dir}"
OUT=$(cd "$1" && pwd)
SCENARIO=${2:-fee-boundaries}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
python3 - "$ROOT" "$WORK" <<'PY'
from pathlib import Path
import shutil
import sys
root, work = map(Path, sys.argv[1:])
for source in (root / "conformance/antevectors").glob("*.go"):
    (work / source.name).write_text(source.read_text().replace(
        "github.com/celestiaorg/celestia-app/v10/",
        "github.com/celestiaorg/celestia-app/v9/"))
for name in ("go.mod", "go.sum"):
    shutil.copyfile(root / "conformance/fuzzoracle/v9" / name, work / name)
PY
export GOTOOLCHAIN=${GOTOOLCHAIN:-$(awk '$1 == "toolchain" { print $2; exit }' "$ROOT/conformance/fuzzoracle/v9/go.mod")}
(cd "$WORK" && go run -buildvcs=false -mod=readonly . "$WORK/out" "$SCENARIO")
for file in "$WORK/out/"*.json*; do
    cp "$file" "$OUT/v9-$(basename "$file")"
done
