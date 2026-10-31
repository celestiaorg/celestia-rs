#!/usr/bin/env bash
set -euo pipefail

source /workspace/scratch/rpc-181/env.sh
readonly APPD=/workspace/scratch/rpc-181/bin-release/celestia-appd
readonly ROOT=/workspace/worktrees/rpc-191
readonly WORK=/workspace/scratch/rpc-181/rest-post
readonly NODE_HOME="$WORK/home"

mkdir -p "$WORK"
"$APPD" init rest-post-boundaries --chain-id rs-rest-post-105-1 --home "$NODE_HOME" \
  >"$WORK/init.log" 2>&1
cp "$ROOT/conformance/fixtures/devnet/empty/genesis.json" "$NODE_HOME/config/genesis.json"
python3 - "$NODE_HOME/config/app.toml" <<'PY'
import pathlib
import re
import sys

path = pathlib.Path(sys.argv[1])
text = path.read_text().replace('pruning = "default"', 'pruning = "nothing"')
for section, port in (("api", 6317), ("grpc", 15090)):
    start = text.index(f"[{section}]")
    end = text.find("\n[", start + 1)
    end = len(text) if end < 0 else end
    block = text[start:end]
    block = re.sub(r"^enable = .*$", "enable = true", block, flags=re.MULTILINE)
    prefix = "tcp://" if section == "api" else ""
    block = re.sub(
        r"^address = .*$",
        f'address = "{prefix}127.0.0.1:{port}"',
        block,
        flags=re.MULTILINE,
    )
    text = text[:start] + block + text[end:]
path.write_text(text)
PY

exec "$APPD" start --force-no-bbr --home "$NODE_HOME" --with-comet=false \
  --address tcp://127.0.0.1:40658
