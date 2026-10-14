#!/usr/bin/env bash
# Regenerates the `celestia-rs init` templates from Go `celestia-appd init`
# (the pinned celestia-app release): config.toml, app.toml, client.toml and
# the default genesis, with the moniker / chain id / genesis time replaced
# by placeholders. usage: templates/regen.sh [path/to/celestia-appd]
set -euo pipefail
APPD=${1:-celestia-appd}
DIR=$(cd "$(dirname "$0")" && pwd)
H=$(mktemp -d)
trap 'rm -rf "$H"' EXIT
"$APPD" init TEMPLATE_MONIKER --chain-id template-chain-id --home "$H" >/dev/null 2>&1
sed -e 's/^moniker = "TEMPLATE_MONIKER"$/moniker = "{{MONIKER}}"/' "$H/config/config.toml" > "$DIR/config.toml"
cp "$H/config/app.toml" "$DIR/app.toml"
sed -e 's/^chain-id = "template-chain-id"$/chain-id = "{{CHAIN_ID}}"/' "$H/config/client.toml" > "$DIR/client.toml"
sed -e 's/^  "genesis_time": ".*",$/  "genesis_time": "{{GENESIS_TIME}}",/' \
  -e 's/^  "chain_id": "template-chain-id",$/  "chain_id": "{{CHAIN_ID}}",/' "$H/config/genesis.json" > "$DIR/genesis.json"
grep -c '{{' "$DIR/config.toml" "$DIR/client.toml" "$DIR/genesis.json"
