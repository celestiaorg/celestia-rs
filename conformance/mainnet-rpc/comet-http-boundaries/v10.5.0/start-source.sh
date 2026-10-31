#!/usr/bin/env bash
set -euo pipefail

source /workspace/scratch/rpc-181/env.sh

readonly NAME=${1:?name}
readonly RPC_PORT=${2:?rpc port}
readonly P2P_PORT=${3:?p2p port}
readonly BATCH=${4:?batch limit}
readonly BODY=${5:?body limit}
readonly HEADER=${6:?header limit}
readonly APPD=/workspace/scratch/rpc-181/bin-release/celestia-appd
readonly WORK=/workspace/scratch/rpc-181/comet-http
readonly NODE_HOME="$WORK/$NAME-home"
readonly CHAIN_ID="rs-$NAME-1"

rm -rf "$NODE_HOME"
"$APPD" init "$NAME" --chain-id "$CHAIN_ID" --home "$NODE_HOME" \
  >"$WORK/$NAME-init.log" 2>&1
"$APPD" keys add val0 --keyring-backend test --home "$NODE_HOME" \
  >"$WORK/$NAME-key.json" 2>"$WORK/$NAME-key.log"
readonly ADDRESS=$("$APPD" keys show val0 -a --keyring-backend test --home "$NODE_HOME")
"$APPD" genesis add-genesis-account "$ADDRESS" 1000000000000utia --home "$NODE_HOME"
"$APPD" genesis gentx val0 5000000000utia --chain-id "$CHAIN_ID" \
  --keyring-backend test --fees 500000utia --home "$NODE_HOME" \
  >"$WORK/$NAME-gentx.log" 2>&1
"$APPD" genesis collect-gentxs --home "$NODE_HOME" \
  >"$WORK/$NAME-collect.log" 2>&1

python3 - "$NODE_HOME/config/config.toml" "$NODE_HOME/config/app.toml" \
  "$RPC_PORT" "$P2P_PORT" "$BATCH" "$BODY" "$HEADER" <<'PY'
import pathlib
import re
import sys

config = pathlib.Path(sys.argv[1])
app = pathlib.Path(sys.argv[2])
rpc_port, p2p_port, batch, body, header = map(int, sys.argv[3:])

def replace(section: str, key: str, value: str, text: str) -> str:
    start = text.index(f"[{section}]")
    end = text.find("\n[", start + 1)
    end = len(text) if end < 0 else end
    block = text[start:end]
    block, count = re.subn(
        rf"^{re.escape(key)}\s*=.*$", f"{key} = {value}", block,
        count=1, flags=re.MULTILINE,
    )
    if count != 1:
        raise ValueError(f"missing {section}.{key}")
    return text[:start] + block + text[end:]

text = config.read_text()
text = re.sub(
    r'^priv_validator_grpc_laddr\s*=.*$',
    'priv_validator_grpc_laddr = ""',
    text,
    count=1,
    flags=re.MULTILINE,
)
text = re.sub(
    r'^grpc_laddr\s*=.*$',
    'grpc_laddr = ""',
    text,
    count=1,
    flags=re.MULTILINE,
)
for section, key, value in [
    ("rpc", "laddr", f'"tcp://127.0.0.1:{rpc_port}"'),
    ("rpc", "cors_allowed_origins", '["https://allowed.example"]'),
    ("rpc", "cors_allowed_methods", '["HEAD", "GET", "POST"]'),
    ("rpc", "cors_allowed_headers", '["Origin", "Accept", "Content-Type", "X-Requested-With", "X-Server-Time", "X-Test"]'),
    ("rpc", "max_request_batch_size", str(batch)),
    ("rpc", "max_body_bytes", str(body)),
    ("rpc", "max_header_bytes", str(header)),
    ("p2p", "laddr", f'"tcp://127.0.0.1:{p2p_port}"'),
]:
    text = replace(section, key, value, text)
config.write_text(text)

text = app.read_text()
text = replace("api", "enable", "false", text)
text = replace("grpc", "enable", "false", text)
app.write_text(text)
PY

exec "$APPD" start --force-no-bbr --home "$NODE_HOME"
