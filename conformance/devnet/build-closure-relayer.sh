#!/usr/bin/env bash
# Build the Go relayer (cosmos/relayer v2.6.0, `rly`) from the Go module
# proxy into $DEVNET_BIN (default target/devnet-bin), for the IBC workload.
#
# RLY_FIXED_GAS=<n> deliberately replaces gas simulation with a fixed limit
# to isolate IBC query/state coverage from the separate Simulate campaign.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
BIN=${DEVNET_BIN:-$ROOT/target/devnet-bin}
mkdir -p "$BIN"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
SRC=$(cd "$ROOT/conformance/devnet-oracle" && GOFLAGS=-mod=mod go mod download -json github.com/cosmos/relayer/v2@v2.6.0 |
  sed -n 's/.*"Dir": "\(.*\)",/\1/p')
cp -r "$SRC" "$WORK/src" && chmod -R u+w "$WORK/src"
python3 - "$WORK/src/relayer/chains/cosmos/tx.go" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
sig = ("func (cc *CosmosProvider) CalculateGas(ctx context.Context, txf tx.Factory, "
       "signingKey string, msgs ...sdk.Msg) (txtypes.SimulateResponse, uint64, error) {\n")
assert sig in s
s = s.replace(sig, sig + '''	if g := os.Getenv("RLY_FIXED_GAS"); g != "" {
		n, err := strconv.ParseUint(g, 10, 64)
		return txtypes.SimulateResponse{}, n, err
	}
''')
s = s.replace('\t"strconv"\n', '\t"os"\n\t"strconv"\n', 1)
open(p, "w").write(s)
PY
(cd "$WORK/src" && GOFLAGS=-mod=mod GOWORK=off go build -buildvcs=false -o "$BIN/rly" .)
echo "built $BIN/rly"
