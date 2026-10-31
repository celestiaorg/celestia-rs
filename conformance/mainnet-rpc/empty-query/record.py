#!/usr/bin/env python3
"""Record local Comet ABCI-query HTTP/WS outcomes without hiding failed reads.

usage: record.py --rpc http://127.0.0.1:26657 --out capture.json [--binary appd]
Only loopback listeners are accepted. The output preserves raw header/body/frame
bytes and distinguishes EOF/timeouts from JSON results. No third-party modules.
"""
import argparse
import base64
import datetime
import hashlib
import http.client
import ipaddress
import json
import pathlib
import socket
import struct
import subprocess
import urllib.parse

PATHS = ["/p2p", "/p2p/filter", "/p2p/filter/addr/x", "/p2p/filter/id/x",
         "/p2p/filter/other/x", "/p2p/filter//x", "/p2p/other/addr/x",
         "/p2p/filter/other/x/extra", "/p2p/filter/addr/x/extra"]
MAX_BYTES = 1024 * 1024
TIMEOUT = 3


def encoded(raw):
    return {"base64": base64.b64encode(raw).decode(),
            "text": raw.decode("utf-8", errors="replace")}


def error(exc):
    return {"error_type": type(exc).__name__, "error": str(exc)}


def http_request(conn, method, path, body=None):
    headers = {"Content-Type": "application/json"} if body else {}
    try:
        conn.request(method, path, body, headers)
        response = conn.getresponse()
        raw = response.read(MAX_BYTES + 1)
        if len(raw) > MAX_BYTES:
            raise ValueError("response exceeds capture limit")
        return {"status": response.status, "reason": response.reason,
                "headers": response.getheaders(), "body": encoded(raw)}
    except (OSError, ValueError, http.client.HTTPException) as exc:
        return error(exc)


def http_case(host, port, path, transport):
    conn = http.client.HTTPConnection(host, port, timeout=TIMEOUT)
    try:
        if transport == "uri":
            request_path = "/abci_query?" + urllib.parse.urlencode({"path": json.dumps(path)})
            result = http_request(conn, "GET", request_path)
            req = {"method": "GET", "path": request_path}
        else:
            query = {"jsonrpc": "2.0", "id": 7, "method": "abci_query", "params": {"path": path}}
            request = ([{"jsonrpc": "2.0", "id": 1, "method": "health", "params": {}}, query,
                        {"jsonrpc": "2.0", "id": 8, "method": "health", "params": {}}]
                       if transport == "jsonrpc_batch" else query)
            body = json.dumps(request, separators=(",", ":"))
            result = http_request(conn, "POST", "/", body)
            req = {"method": "POST", "path": "/", "body": body}
        # HTTPConnection reconnects after EOF. Compare the underlying socket to
        # distinguish reuse from recovery on a newly established connection.
        sock = conn.sock
        health = http_request(conn, "GET", "/health")
        same = sock is not None and sock is conn.sock
        return {"transport": transport, "query_path": path, "request": req,
                "response": result, "health_after": health, "same_connection": same}
    finally:
        conn.close()


def recv_exact(sock, length):
    out = bytearray()
    while len(out) < length:
        part = sock.recv(length - len(out))
        if not part:
            raise EOFError(f"EOF after {len(out)} of {length} frame bytes")
        out.extend(part)
    return bytes(out)


def send_frame(sock, opcode, payload):
    # RFC6455 client frames must be masked. Fixed masks make capture inputs
    # reproducible; this loopback-only recorder does not carry secrets.
    mask = b"\x12\x34\x56\x78"
    header = bytes([0x80 | opcode])
    size = len(payload)
    if size < 126:
        header += bytes([0x80 | size])
    elif size <= 65535:
        header += b"\xfe" + struct.pack("!H", size)
    else:
        header += b"\xff" + struct.pack("!Q", size)
    sock.sendall(header + mask + bytes(b ^ mask[i % 4] for i, b in enumerate(payload)))


def read_frame(sock):
    header = recv_exact(sock, 2)
    size = header[1] & 127
    ext = b""
    if size == 126:
        ext = recv_exact(sock, 2)
        size = struct.unpack("!H", ext)[0]
    elif size == 127:
        ext = recv_exact(sock, 8)
        size = struct.unpack("!Q", ext)[0]
    if size > MAX_BYTES:
        raise ValueError("WS frame exceeds capture limit")
    mask = recv_exact(sock, 4) if header[1] & 128 else b""
    payload = recv_exact(sock, size)
    wire = header + ext + mask + payload
    if mask:
        payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    return {"opcode": header[0] & 15, "fin": bool(header[0] & 128),
            "wire": encoded(wire), "payload": encoded(payload)}


def ws_response(sock):
    frames = []
    try:
        for _ in range(8):
            frame = read_frame(sock)
            frames.append(frame)
            if frame["opcode"] == 9:
                send_frame(sock, 10, base64.b64decode(frame["payload"]["base64"]))
            elif frame["opcode"] in (1, 2, 8):
                return {"frames": frames}
        raise ValueError("too many control frames")
    except (OSError, ValueError, EOFError) as exc:
        return {"frames": frames, **error(exc)}


def ws_case(host, port, path):
    sock = socket.create_connection((host, port), TIMEOUT)
    sock.settimeout(TIMEOUT)
    try:
        key = "dGhlIHNhbXBsZSBub25jZQ=="
        request = (f"GET /websocket HTTP/1.1\r\nHost: {host}:{port}\r\n"
                   f"Upgrade: websocket\r\nConnection: Upgrade\r\n"
                   f"Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\n\r\n")
        sock.sendall(request.encode())
        header = bytearray()
        while not header.endswith(b"\r\n\r\n"):
            if len(header) >= 65536:
                raise ValueError("upgrade header exceeds capture limit")
            header.extend(recv_exact(sock, 1))
        if not header.startswith(b"HTTP/1.1 101 "):
            raise ValueError(f"upgrade failed: {header!r}")
        body = json.dumps({"jsonrpc": "2.0", "id": 7, "method": "abci_query",
                           "params": {"path": path}}, separators=(",", ":"))
        send_frame(sock, 1, body.encode())
        result = ws_response(sock)
        health_body = '{"jsonrpc":"2.0","id":8,"method":"health","params":{}}'
        try:
            send_frame(sock, 1, health_body.encode())
            health = ws_response(sock)
        except OSError as exc:
            health = error(exc)
        return {"transport": "websocket", "query_path": path,
                "upgrade_request": request, "upgrade_response": encoded(header),
                "request": body, "response": result, "health_after": health}
    finally:
        sock.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rpc", required=True)
    parser.add_argument("--out", required=True)
    parser.add_argument("--binary")
    args = parser.parse_args()
    url = urllib.parse.urlsplit(args.rpc)
    if url.scheme != "http" or not ipaddress.ip_address(url.hostname).is_loopback:
        parser.error("--rpc must be an explicit loopback http address")
    host, port = url.hostname, url.port or 80
    cases = []
    for path in PATHS:
        cases.extend([http_case(host, port, path, transport) for transport in ("uri", "jsonrpc")])
        cases.append(ws_case(host, port, path))
    cases.append(http_case(host, port, "/p2p/filter/other/x", "jsonrpc_batch"))
    output = {"captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "rpc": args.rpc, "cases": cases,
              "capture_source_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(),
              "bounds": {"timeout_seconds": TIMEOUT, "max_response_bytes": MAX_BYTES}}
    if args.binary:
        binary = pathlib.Path(args.binary)
        output["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        output["go_build_info"] = subprocess.check_output(["go", "version", "-m", str(binary)], text=True)
    pathlib.Path(args.out).write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
