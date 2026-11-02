#!/usr/bin/env bash
# Build the exact Rust campaign binary and atomically bind it to its source.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
OUT=${1:?usage: build-rs.sh OUTPUT_BINARY}
OUT=$(realpath -m "$OUT")
COMMIT=$(git -C "$ROOT" rev-parse HEAD)
git -C "$ROOT" verify-commit "$COMMIT" >/dev/null
[ -z "$(git -C "$ROOT" status --porcelain)" ] || {
  echo "Rust build requires a clean signed checkout" >&2
  exit 1
}
LOCK_SHA=$(sha256sum "$ROOT/Cargo.lock" | awk '{print $1}')
RUSTC_VV=$(rustc -vV)
CARGO_VERSION=$(cargo -V)
BUILD_COMMAND=(cargo build --locked --release --package celestia-rs --bin celestia-rs)
(cd "$ROOT" && "${BUILD_COMMAND[@]}")
BUILD_TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
if [[ "$BUILD_TARGET_DIR" != /* ]]; then
  BUILD_TARGET_DIR="$ROOT/$BUILD_TARGET_DIR"
fi
SOURCE_BINARY="$BUILD_TARGET_DIR/release/celestia-rs"
[ -x "$SOURCE_BINARY" ]
[ "$COMMIT" = "$(git -C "$ROOT" rev-parse HEAD)" ]
[ -z "$(git -C "$ROOT" status --porcelain)" ]
[ "$LOCK_SHA" = "$(sha256sum "$ROOT/Cargo.lock" | awk '{print $1}')" ]
mkdir -p "$(dirname "$OUT")"
cp "$SOURCE_BINARY" "$OUT.tmp"
chmod 0755 "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
VERSION_LONG=$("$OUT" version --long)
python3 - "$OUT" "$OUT.build.json.tmp" "$COMMIT" "$LOCK_SHA" \
  "$RUSTC_VV" "$CARGO_VERSION" "$VERSION_LONG" <<'PY'
import datetime,hashlib,json,pathlib,sys
binary,receipt,commit,lock_sha,rustc_vv,cargo_version,version_long=sys.argv[1:]
value={
 "schema_version":1,
 "built_at":datetime.datetime.now(datetime.timezone.utc).isoformat(),
 "binary_sha256":hashlib.sha256(pathlib.Path(binary).read_bytes()).hexdigest(),
 "source_commit":commit,
 "source_commit_signature_verified":True,
 "cargo_lock_sha256":lock_sha,
 "rustc_vv":rustc_vv,
 "cargo_version":cargo_version,
 "target":"x86_64-unknown-linux-gnu",
 "build_command":["cargo","build","--locked","--release","--package","celestia-rs","--bin","celestia-rs"],
 "profile":"release",
 "package":"celestia-rs",
 "binary":"celestia-rs",
 "locked":True,
 "default_features":True,
 "features":["rocksdb","jemalloc"],
 "version_long":version_long,
}
pathlib.Path(receipt).write_text(json.dumps(value,indent=2)+"\n")
PY
mv "$OUT.build.json.tmp" "$OUT.build.json"
printf '%s\n' "$OUT" "$OUT.build.json"
