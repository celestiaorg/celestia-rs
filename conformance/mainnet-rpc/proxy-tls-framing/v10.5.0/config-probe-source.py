#!/usr/bin/env python3
"""Probe one tagged-Go TLS configuration outcome without normalizing failures."""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import re
import socket
import ssl


HERE = pathlib.Path(__file__).parent
spec = importlib.util.spec_from_file_location("capture_source", HERE / "capture-source.py")
capture = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(capture)


def connect(port: int, tls: bool) -> dict:
    try:
        stream: socket.socket = socket.create_connection(("127.0.0.1", port), timeout=2)
        stream.settimeout(2)
        tls_info = None
        if tls:
            context = ssl.create_default_context(cafile=str(capture.CERT))
            context.set_alpn_protocols(["http/1.1"])
            wrapped = context.wrap_socket(stream, server_hostname="localhost")
            tls_info = {
                "version": wrapped.version(),
                "cipher": wrapped.cipher(),
                "alpn": wrapped.selected_alpn_protocol(),
                "peer_certificate_sha256": capture.sha(wrapped.getpeercert(binary_form=True)),
            }
            stream = wrapped
        raw = capture.request(method=b"GET", target=b"/health")
        try:
            stream.sendall(raw)
            response = capture.receive_all(stream)
        finally:
            stream.close()
        result = {
            "outcome": "exchange",
            "request": capture.raw_record(raw),
            "response": capture.parse_response(response),
        }
        if tls_info is not None:
            result["tls"] = tls_info
        return result
    except (ConnectionError, OSError, ssl.SSLError) as error:
        return {
            "outcome": "connection-failure",
            "error_class": type(error).__name__,
        }


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--name", required=True)
parser.add_argument("--port", required=True, type=int)
parser.add_argument("--expected", choices=("plain", "tls", "unavailable"), required=True)
parser.add_argument("--config", required=True, type=pathlib.Path)
parser.add_argument("--log", required=True, type=pathlib.Path)
parser.add_argument("--out", required=True, type=pathlib.Path)
args = parser.parse_args()

probes = {"plain": connect(args.port, False), "tls": connect(args.port, True)}
if args.expected == "plain":
    assert probes["plain"].get("response", {}).get("status") == 200
    assert probes["tls"]["outcome"] == "connection-failure"
elif args.expected == "tls":
    assert probes["tls"].get("response", {}).get("status") == 200
    assert probes["plain"].get("response", {}).get("status") == 400
else:
    assert probes["plain"].get("response", {}).get("kind") != "http-response"
    assert probes["tls"].get("response", {}).get("kind") != "http-response"

config = args.config.read_bytes()
result = {
    "name": args.name,
    "category": "tls-config",
    "expected_transport": args.expected,
    "config_sha256": capture.sha(config),
    "config_size": len(config),
    "probes": probes,
}
if args.expected == "unavailable":
    log = args.log.read_text(errors="replace")
    failures = re.findall(r'RPC HTTPS server stopped err="([^"]+)"', log)
    if not failures:
        raise ValueError("tagged Go TLS startup failure was not recorded")
    result["tls_start_error"] = failures[-1]
args.out.write_text(json.dumps(result, indent=2) + "\n")
print(args.name, args.expected)
