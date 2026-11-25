#!/usr/bin/env bash
# Prepare one reusable heavy-chain recording on the existing campaign VM.
set -euo pipefail
base=2da1bf7cc88a1afbe84f2c0ef04cfda11a3fc2d8
mkdir -p "$HOME/genesis-sync"
cd "$HOME/genesis-sync"
exec 9>setup.lock
flock -n 9 || { echo 'Another setup is running'; exit 1; }
if [[ -f ready ]]; then exit 0; fi
sudo apt-get -qq update
sudo env DEBIAN_FRONTEND=noninteractive apt-get -qq install -y \
  build-essential clang libclang-dev cmake pkg-config protobuf-compiler \
  libssl-dev libudev-dev git curl jq golang-go
if [[ ! -x "$HOME/.cargo/bin/rustup" ]]; then
  curl --fail --silent --show-error --retry 3 https://sh.rustup.rs -o rustup-init.sh
  sh rustup-init.sh -y --profile minimal
fi
export PATH="$HOME/.cargo/bin:$PATH"
if [[ ! -d source/.git ]]; then
  git init source
  git -C source remote add origin https://github.com/celestiaorg/celestia-rs.git
fi
cd source
git fetch --depth 1 origin "$base"
git checkout --detach "$base"
conformance/devnet/build-bins.sh
cargo build --release -p abci-server --bin abci-recorder
cp target/release/abci-recorder target/devnet-bin/
cargo build --release -p celestia-rs --example bench_load
cargo build --release -p celestia-rs
cargo build --release -p devnet-conformance --example abci_bench
BENCH_HEIGHTS=80 conformance/bench/gen-chain.sh
touch ../ready
