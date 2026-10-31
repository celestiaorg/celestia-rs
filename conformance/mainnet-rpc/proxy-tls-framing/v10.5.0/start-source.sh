#!/usr/bin/env bash
set -euo pipefail

source /workspace/scratch/rpc-181/env.sh

readonly NAME=${1:?plain or tls}
readonly RPC_PORT=${2:?rpc port}
readonly P2P_PORT=${3:?p2p port}
readonly API_PORT=${4:?api port}
readonly GRPC_PORT=${5:?grpc port}
readonly TLS_MODE=${6:?cert-only, key-only, relative, absolute, missing or mismatch}
readonly APPD=/workspace/scratch/rpc-181/bin-release/celestia-appd
readonly WORK=/workspace/scratch/rpc-181/proxy-tls
readonly NODE_HOME="$WORK/$NAME-home"
readonly CHAIN_ID="rs-proxy-$NAME-1"

case "$TLS_MODE" in
  cert-only|key-only|relative|absolute|missing|mismatch) ;;
  *) echo "invalid TLS mode: $TLS_MODE" >&2; exit 2 ;;
esac

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

if [[ "$TLS_MODE" == relative ]]; then
  cp "$WORK/tls/localhost.crt" "$NODE_HOME/config/localhost.crt"
  cp "$WORK/tls/localhost.key" "$NODE_HOME/config/localhost.key"
fi

python3 - "$NODE_HOME/config/config.toml" "$NODE_HOME/config/app.toml" \
  "$RPC_PORT" "$P2P_PORT" "$API_PORT" "$GRPC_PORT" "$TLS_MODE" "$WORK" <<'PY'
import pathlib
import re
import sys

config = pathlib.Path(sys.argv[1])
app = pathlib.Path(sys.argv[2])
rpc_port, p2p_port, api_port, grpc_port = map(int, sys.argv[3:7])
tls_mode = sys.argv[7]
work = pathlib.Path(sys.argv[8])

def replace(section: str, key: str, value: str, text: str) -> str:
    start = text.index(f"[{section}]")
    end = text.find("\n[", start + 1)
    end = len(text) if end < 0 else end
    block = text[start:end]
    block, count = re.subn(
        rf"^{re.escape(key)}\s*=.*$",
        f"{key} = {value}",
        block,
        count=1,
        flags=re.MULTILINE,
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
tls_values = {
    "cert-only": ('"ignored.crt"', '""'),
    "key-only": ('""', '"ignored.key"'),
    "relative": ('"localhost.crt"', '"localhost.key"'),
    "absolute": (f'"{work / "tls/localhost.crt"}"', f'"{work / "tls/localhost.key"}"'),
    "missing": (f'"{work / "tls/missing.crt"}"', f'"{work / "tls/missing.key"}"'),
    "mismatch": (f'"{work / "tls/localhost.crt"}"', f'"{work / "tls/mismatch.key"}"'),
}[tls_mode]
for section, key, value in [
    ("rpc", "laddr", f'"tcp://127.0.0.1:{rpc_port}"'),
    ("rpc", "cors_allowed_origins", '["https://allowed.example"]'),
    ("rpc", "cors_allowed_methods", '["HEAD", "GET", "POST"]'),
    (
        "rpc",
        "cors_allowed_headers",
        '["Origin", "Accept", "Content-Type", "X-Requested-With", "X-Server-Time", "X-Test"]',
    ),
    ("rpc", "tls_cert_file", tls_values[0]),
    ("rpc", "tls_key_file", tls_values[1]),
    ("p2p", "laddr", f'"tcp://127.0.0.1:{p2p_port}"'),
]:
    text = replace(section, key, value, text)
config.write_text(text)

text = app.read_text()
text = replace("api", "enable", "true", text)
text = replace("api", "address", f'"tcp://127.0.0.1:{api_port}"', text)
text = replace("grpc", "enable", "true", text)
text = replace("grpc", "address", f'"127.0.0.1:{grpc_port}"', text)
app.write_text(text)
PY

exec "$APPD" start --force-no-bbr --home "$NODE_HOME"
