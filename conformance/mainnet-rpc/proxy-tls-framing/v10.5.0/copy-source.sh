#!/bin/sh
set -eu
cd "$(dirname "$0")"

core=/workspace/.cache/go/pkg/mod/github.com/celestiaorg/celestia-core@v0.42.3
cors=/workspace/.cache/go/pkg/mod/github.com/rs/cors@v1.11.1
goroot=/workspace/.cache/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.26.9.linux-amd64
mkdir -p go-source

copy() {
  origin=$1 destination=$2
  gzip -n -c "$origin" >"go-source/$destination.gz"
}

copy "$core/config/config.go" core-config.go
copy "$core/config/toml.go" core-toml.go
copy "$core/node/node.go" core-node.go
copy "$core/rpc/jsonrpc/server/http_server.go" core-http-server.go
copy "$core/rpc/jsonrpc/server/http_json_handler.go" core-http-json-handler.go
copy "$core/rpc/jsonrpc/server/ws_handler.go" core-ws-handler.go
copy "$cors/cors.go" cors.go
copy "$goroot/src/net/http/request.go" stdlib-net-http-request.go
copy "$goroot/src/net/http/server.go" stdlib-net-http-server.go
copy "$goroot/src/net/http/transfer.go" stdlib-net-http-transfer.go
copy "$goroot/src/net/http/internal/chunked.go" stdlib-net-http-chunked.go
copy "$goroot/src/net/http/h2_bundle.go" stdlib-net-http-h2-bundle.go
copy "$goroot/src/crypto/tls/tls.go" stdlib-crypto-tls.go
copy "$goroot/src/crypto/tls/handshake_server.go" stdlib-crypto-tls-handshake-server.go

cp /workspace/worktrees/rpc-191-proxy-tls/conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/oracle-build.json .
cp /workspace/worktrees/rpc-191-proxy-tls/conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/oracle-build-info.txt.gz .
cp /workspace/worktrees/rpc-191-proxy-tls/conformance/mainnet-rpc/comet-http-boundaries/v10.5.0/oracle-module.txt .
"$goroot/bin/go" version >go-compiler.txt
/usr/bin/curl --version >curl-version.txt
sha256sum /usr/bin/curl >curl-binary.sha256
