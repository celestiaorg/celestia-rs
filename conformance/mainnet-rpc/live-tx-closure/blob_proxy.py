#!/usr/bin/env python3
"""One-shot loopback capture proxy for the original CLI BlobTx envelope."""

from __future__ import annotations

import argparse
import base64
import hashlib
import http.server
import json
import os
import pathlib
import sys

import harness


class CaptureServer(http.server.ThreadingHTTPServer):
    daemon_threads = False
    allow_reuse_address = False

    def __init__(self, address: tuple[str, int], output: pathlib.Path) -> None:
        super().__init__(address, CaptureHandler)
        self.output = output
        self.requests = 0
        self.error: str | None = None


class CaptureHandler(http.server.BaseHTTPRequestHandler):
    server: CaptureServer

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def do_POST(self) -> None:  # noqa: N802 - BaseHTTPRequestHandler API
        try:
            harness.require(self.client_address[0] in {"127.0.0.1", "::1"}, "Blob proxy client is not loopback")
            harness.require(self.path == "/", "Blob proxy path differs")
            harness.require(self.server.requests == 0, "Blob proxy received more than one request")
            harness.require(self.headers.get("Transfer-Encoding") is None, "Blob proxy rejects transfer encoding")
            length = int(self.headers.get("Content-Length", "-1"))
            harness.require(0 <= length <= 2 * 1024 * 1024, "Blob proxy body size invalid")
            raw = self.rfile.read(length)
            harness.require(len(raw) == length, "Blob proxy request truncated")
            request = harness.strict_json(raw)
            harness.require_fields(request, {"jsonrpc", "id", "method", "params"}, "Blob proxy JSON-RPC request")
            harness.require(request["jsonrpc"] == "2.0" and request["id"] == 0 and request["method"] == "broadcast_tx_sync",
                            "Blob proxy accepts only id 0 broadcast_tx_sync")
            params = harness.require_fields(request["params"], {"tx"}, "Blob proxy params")
            envelope = base64.b64decode(params["tx"], validate=True)
            harness.decode_canonical_blob_tx(envelope)
            payload = harness.canonical_json({
                "raw_request": {"base64": base64.b64encode(raw).decode(), "sha256": hashlib.sha256(raw).hexdigest(), "size": len(raw)},
                "envelope": {"base64": base64.b64encode(envelope).decode(), "sha256": hashlib.sha256(envelope).hexdigest(), "size": len(envelope)},
                "request_id": 0, "method": "broadcast_tx_sync", "request_count": 1, "forward_count": 0,
            })
            descriptor = os.open(self.server.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, "wb") as output:
                output.write(payload)
            self.server.requests = 1
            response = harness.canonical_json({"jsonrpc": "2.0", "id": 0, "result": {"code": 0, "data": "", "log": "captured-not-forwarded", "hash": hashlib.sha256(envelope).hexdigest().upper()}})
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(response)
        except Exception as error:  # handler must preserve a bounded diagnostic
            self.server.error = f"{type(error).__name__}: {error}"[:4096]
            body = b'{"error":"capture rejected"}\n'
            self.send_response(400)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)


def serve(listen: str, output: pathlib.Path, timeout: float) -> None:
    host, port = harness.loopback_endpoint(f"tcp://{listen}", "Blob proxy listener")
    harness.reject_symlink_components(output.parent, "Blob proxy output parent")
    harness.require(not output.exists(), "Blob proxy output already exists")
    server = CaptureServer((host, port), output)
    server.timeout = timeout
    try:
        server.handle_request()
    finally:
        server.server_close()
    harness.require(server.error is None, f"Blob proxy rejected request: {server.error}")
    harness.require(server.requests == 1 and output.is_file(), "Blob proxy did not capture exactly one request")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--listen", required=True, help="numeric loopback host:port")
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--timeout", type=float, default=30)
    args = parser.parse_args()
    try:
        serve(args.listen, args.output, args.timeout)
    except (OSError, ValueError, json.JSONDecodeError, harness.HarnessError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
