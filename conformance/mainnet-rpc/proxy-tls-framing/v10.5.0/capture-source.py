#!/usr/bin/env python3
"""Capture the exact bounded proxy/TLS/framing corpus from tagged Go nodes."""

from __future__ import annotations

import base64
import datetime as dt
import hashlib
import json
import pathlib
import socket
import ssl
import subprocess
from typing import Any


HERE = pathlib.Path(__file__).parent
PLAIN = ("127.0.0.1", 6340)
TLS = ("127.0.0.1", 6350)
TLS_NODE_REST = ("127.0.0.1", 6352)
CERT = HERE / "tls" / "localhost.crt"
DYNAMIC = {"connection", "content-length", "date", "transfer-encoding", "x-server-time"}
HEALTH = b'{"jsonrpc":"2.0","id":1,"method":"health"}'


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def raw_record(raw: bytes) -> dict[str, Any]:
    return {
        "raw_base64": base64.b64encode(raw).decode(),
        "raw_sha256": sha(raw),
        "raw_size": len(raw),
    }


def parse_response(raw: bytes) -> dict[str, Any]:
    record = raw_record(raw)
    if not raw:
        return {**record, "kind": "eof"}
    if b"\r\n\r\n" not in raw:
        return {**record, "kind": "opaque"}
    head, encoded = raw.split(b"\r\n\r\n", 1)
    lines = head.split(b"\r\n")
    status = lines[0].decode("latin1").split(" ", 2)
    if len(status) == 2 and status[0] == "HTTP/2":
        status.append("")
    if len(status) != 3 or not status[1].isdigit():
        return {**record, "kind": "opaque"}
    headers: list[list[str]] = []
    for line in lines[1:]:
        if b":" not in line:
            return {**record, "kind": "opaque"}
        name, value = line.split(b":", 1)
        headers.append([name.decode("latin1"), value.strip().decode("latin1")])
    lookup: dict[str, list[str]] = {}
    for name, value in headers:
        lookup.setdefault(name.lower(), []).append(value)
    body = encoded
    if any("chunked" in value.lower() for value in lookup.get("transfer-encoding", [])):
        decoded = bytearray()
        offset = 0
        while True:
            end = encoded.find(b"\r\n", offset)
            if end < 0:
                raise ValueError("unterminated response chunk size")
            size = int(encoded[offset:end].split(b";", 1)[0], 16)
            offset = end + 2
            if size == 0:
                break
            decoded.extend(encoded[offset : offset + size])
            offset += size
            if encoded[offset : offset + 2] != b"\r\n":
                raise ValueError("bad response chunk terminator")
            offset += 2
        body = bytes(decoded)
    elif lookup.get("content-length"):
        length = int(lookup["content-length"][-1])
        if len(body) != length:
            raise ValueError(f"response content length {length}, received {len(body)}")
    return {
        **record,
        "kind": "http-response",
        "http_version": status[0],
        "status": int(status[1]),
        "reason": status[2],
        "headers": headers,
        "stable_headers": [pair for pair in headers if pair[0].lower() not in DYNAMIC],
        "body_base64": base64.b64encode(body).decode(),
        "body_sha256": sha(body),
        **({"body_text": body.decode()} if is_utf8(body) else {}),
    }


def split_responses(raw: bytes) -> list[bytes]:
    responses = []
    offset = 0
    while offset < len(raw):
        boundary = raw.find(b"\r\n\r\n", offset)
        if boundary < 0:
            responses.append(raw[offset:])
            break
        head_end = boundary + 4
        head = raw[offset:boundary]
        headers: dict[str, list[str]] = {}
        for line in head.split(b"\r\n")[1:]:
            if b":" not in line:
                responses.append(raw[offset:])
                return responses
            name, value = line.split(b":", 1)
            headers.setdefault(name.decode("latin1").lower(), []).append(
                value.strip().decode("latin1")
            )
        if headers.get("content-length"):
            end = head_end + int(headers["content-length"][-1])
        elif any("chunked" in value.lower() for value in headers.get("transfer-encoding", [])):
            marker = raw.find(b"\r\n0\r\n\r\n", head_end)
            if marker < 0:
                responses.append(raw[offset:])
                break
            end = marker + len(b"\r\n0\r\n\r\n")
        else:
            responses.append(raw[offset:])
            break
        responses.append(raw[offset:end])
        offset = end
    return responses


def is_utf8(data: bytes) -> bool:
    try:
        data.decode()
    except UnicodeDecodeError:
        return False
    return True


def receive_all(stream: socket.socket) -> bytes:
    chunks = []
    while True:
        try:
            chunk = stream.recv(65536)
        except (ConnectionResetError, socket.timeout, ssl.SSLError):
            break
        if not chunk:
            break
        chunks.append(chunk)
    return b"".join(chunks)


def exchange(
    name: str,
    category: str,
    raw: bytes,
    *,
    tls: bool = False,
    half_close: bool = False,
    address: tuple[str, int] | None = None,
) -> dict[str, Any]:
    address = address or (TLS if tls else PLAIN)
    stream: socket.socket = socket.create_connection(address, timeout=3)
    stream.settimeout(3)
    tls_info = None
    if tls:
        context = ssl.create_default_context(cafile=str(CERT))
        context.set_alpn_protocols(["http/1.1"])
        wrapped = context.wrap_socket(stream, server_hostname="localhost")
        tls_info = {
            "version": wrapped.version(),
            "cipher": wrapped.cipher(),
            "alpn": wrapped.selected_alpn_protocol(),
            "peer_certificate_sha256": sha(wrapped.getpeercert(binary_form=True)),
        }
        stream = wrapped
    try:
        stream.sendall(raw)
        if half_close:
            stream.shutdown(socket.SHUT_WR)
        response = receive_all(stream)
    finally:
        stream.close()
    result = {
        "name": name,
        "category": category,
        "transport": "tls" if tls else "plain",
        "request": raw_record(raw),
        "response": parse_response(response),
    }
    if tls_info is not None:
        result["tls"] = tls_info
    print(name, result["response"].get("status", result["response"]["kind"]))
    return result


def receive_until(stream: socket.socket, marker: bytes) -> bytes:
    data = bytearray()
    while marker not in data:
        chunk = stream.recv(65536)
        if not chunk:
            raise EOFError(f"EOF before {marker!r}")
        data.extend(chunk)
    boundary = data.index(marker) + len(marker)
    if boundary != len(data):
        raise ValueError("unexpected bytes after HTTP upgrade headers")
    return bytes(data)


def websocket_client_frame(opcode: int, payload: bytes) -> bytes:
    mask = b"\x12\x34\x56\x78"
    if len(payload) < 126:
        length = bytes([0x80 | len(payload)])
    elif len(payload) <= 0xFFFF:
        length = b"\xfe" + len(payload).to_bytes(2, "big")
    else:
        length = b"\xff" + len(payload).to_bytes(8, "big")
    encoded = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
    return bytes([0x80 | opcode]) + length + mask + encoded


def receive_exact(stream: socket.socket, length: int) -> bytes:
    value = bytearray()
    while len(value) < length:
        chunk = stream.recv(length - len(value))
        if not chunk:
            raise EOFError(f"truncated input: wanted {length}, received {len(value)}")
        value.extend(chunk)
    return bytes(value)


def websocket_server_frame(stream: socket.socket) -> tuple[bytes, bool, int, bytes]:
    first = receive_exact(stream, 2)
    fin = bool(first[0] & 0x80)
    opcode = first[0] & 0x0F
    masked = first[1] & 0x80
    length = first[1] & 0x7F
    extra = b""
    if length == 126:
        extra = receive_exact(stream, 2)
        length = int.from_bytes(extra, "big")
    elif length == 127:
        extra = receive_exact(stream, 8)
        length = int.from_bytes(extra, "big")
    if masked:
        raise ValueError("server WebSocket frame is masked")
    payload = receive_exact(stream, length)
    return first + extra + payload, fin, opcode, payload


def wss_jsonrpc_case() -> dict[str, Any]:
    context = ssl.create_default_context(cafile=str(CERT))
    context.set_alpn_protocols(["http/1.1"])
    with socket.create_connection(TLS, timeout=3) as tcp:
        with context.wrap_socket(tcp, server_hostname="localhost") as stream:
            stream.settimeout(3)
            handshake_request = request(
                method=b"GET",
                target=b"/websocket",
                headers=[
                    (b"Upgrade", b"websocket"),
                    (b"Sec-WebSocket-Key", b"dGhlIHNhbXBsZSBub25jZQ=="),
                    (b"Sec-WebSocket-Version", b"13"),
                ],
                connection=b"Upgrade",
            )
            stream.sendall(handshake_request)
            handshake_response = receive_until(stream, b"\r\n\r\n")
            request_frame = websocket_client_frame(1, HEALTH)
            stream.sendall(request_frame)
            response_frame, response_fin, response_opcode, response_payload = websocket_server_frame(stream)
            response_json = json.loads(response_payload)
            if not response_fin or response_opcode != 1:
                raise ValueError("WSS JSON-RPC response is not one final text frame")
            if response_json != {"jsonrpc": "2.0", "id": 1, "result": {}}:
                raise ValueError(f"unexpected WSS health response: {response_json!r}")
            close_request = websocket_client_frame(8, (1000).to_bytes(2, "big"))
            stream.sendall(close_request)
            close_response, close_fin, close_opcode, close_payload = websocket_server_frame(stream)
            if not close_fin or close_opcode != 8 or int.from_bytes(close_payload[:2], "big") != 1000:
                raise ValueError("WSS close response is not one final normal-close frame")
            if stream.recv(1) != b"":
                raise ValueError("WSS connection remained open after clean close")
            return {
                "name": "tls-wss-jsonrpc-clean-close",
                "category": "tls",
                "transport": "wss",
                "tls": {
                    "version": stream.version(),
                    "cipher": stream.cipher(),
                    "alpn": stream.selected_alpn_protocol(),
                    "peer_certificate_sha256": sha(stream.getpeercert(binary_form=True)),
                },
                "handshake_request": raw_record(handshake_request),
                "handshake_response": parse_response(handshake_response),
                "request_frame": {**raw_record(request_frame), "opcode": 1, "payload_sha256": sha(HEALTH)},
                "response_frame": {
                    **raw_record(response_frame),
                    "fin": response_fin,
                    "opcode": response_opcode,
                    "payload_base64": base64.b64encode(response_payload).decode(),
                    "payload_sha256": sha(response_payload),
                    "payload_text": response_payload.decode(),
                },
                "close_request": {**raw_record(close_request), "code": 1000},
                "close_response": {
                    **raw_record(close_response),
                    "fin": close_fin,
                    "opcode": close_opcode,
                    "payload_base64": base64.b64encode(close_payload).decode(),
                    "code": int.from_bytes(close_payload[:2], "big"),
                },
                "outcome": "clean-eof",
            }


def http2_case() -> dict[str, Any]:
    command = [
        "/usr/bin/curl",
        "--silent",
        "--show-error",
        "--http2",
        "--noproxy",
        "*",
        "--include",
        "--raw",
        "--cacert",
        str(CERT),
        "--resolve",
        "localhost:6350:127.0.0.1",
        "https://localhost:6350/health",
    ]
    completed = subprocess.run(command, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    response = parse_response(completed.stdout)
    if response.get("http_version") != "HTTP/2":
        raise ValueError(f"curl did not negotiate HTTP/2: {response!r}")
    return {
        "name": "tls-health-http2",
        "category": "tls",
        "transport": "tls-http2",
        "command": command,
        "response": response,
        "stderr": completed.stderr.decode(),
    }


def pipeline_exchange(name: str, first: bytes, second: bytes) -> dict[str, Any]:
    stream = socket.create_connection(PLAIN, timeout=3)
    stream.settimeout(3)
    try:
        stream.sendall(first + second)
        raw = receive_all(stream)
    finally:
        stream.close()
    messages = split_responses(raw)
    result = {
        "name": name,
        "category": "framing-connection",
        "transport": "plain",
        "requests": [raw_record(first), raw_record(second)],
        "raw_responses": raw_record(raw),
        "responses": [parse_response(message) for message in messages],
    }
    print(name, [response.get("status", response["kind"]) for response in result["responses"]])
    return result


def request(
    *,
    method: bytes = b"POST",
    target: bytes = b"/",
    version: bytes = b"HTTP/1.1",
    host: bytes | None = b"localhost",
    headers: list[tuple[bytes, bytes]] | None = None,
    body: bytes = b"",
    content_length: bool = True,
    connection: bytes | None = b"close",
) -> bytes:
    fields = [] if host is None else [(b"Host", host)]
    fields.extend(headers or [])
    if connection is not None:
        fields.append((b"Connection", connection))
    if content_length:
        fields.append((b"Content-Length", str(len(body)).encode()))
    head = b" ".join((method, target, version)) + b"\r\n"
    head += b"".join(name + b": " + value + b"\r\n" for name, value in fields)
    return head + b"\r\n" + body


def chunked(body: bytes, *, extension: bytes = b"") -> bytes:
    return f"{len(body):x}".encode() + extension + b"\r\n" + body + b"\r\n0\r\n\r\n"


def tls_special_cases() -> list[dict[str, Any]]:
    cases = [
        exchange("tls-health-http1", "tls", request(method=b"GET", target=b"/health", body=b""), tls=True),
        http2_case(),
        exchange(
            "tls-jsonrpc-health-http1",
            "tls",
            request(headers=[(b"Content-Type", b"application/json")], body=HEALTH),
            tls=True,
        ),
        exchange(
            "tls-cors-preflight-http1",
            "tls",
            request(
                method=b"OPTIONS",
                target=b"/health",
                headers=[
                    (b"Origin", b"https://allowed.example"),
                    (b"Access-Control-Request-Method", b"POST"),
                    (b"Access-Control-Request-Headers", b"Content-Type, X-Test"),
                ],
            ),
            tls=True,
        ),
        wss_jsonrpc_case(),
        exchange(
            "tls-node-rest-get-latest-block",
            "tls",
            request(method=b"GET", target=b"/cosmos/base/tendermint/v1beta1/blocks/latest"),
            address=TLS_NODE_REST,
        ),
    ]
    context = ssl.create_default_context(cafile=str(CERT))
    try:
        with socket.create_connection(TLS, timeout=3) as tcp:
            context.wrap_socket(tcp, server_hostname="provider.invalid")
    except ssl.SSLCertVerificationError as error:
        cases.append(
            {
                "name": "tls-certificate-name-mismatch",
                "category": "tls",
                "transport": "tls",
                "outcome": "certificate-name-mismatch",
                "verify_code": error.verify_code,
            }
        )
    else:
        raise AssertionError("wrong TLS server name unexpectedly verified")

    plain = socket.create_connection(TLS, timeout=3)
    plain.settimeout(3)
    raw = request(method=b"GET", target=b"/health")
    try:
        plain.sendall(raw)
        response = receive_all(plain)
    finally:
        plain.close()
    cases.append(
        {
            "name": "plaintext-on-tls-port",
            "category": "tls",
            "transport": "plain-to-tls-port",
            "request": raw_record(raw),
            "response": parse_response(response),
        }
    )

    context = ssl.create_default_context(cafile=str(CERT))
    try:
        with socket.create_connection(PLAIN, timeout=3) as tcp:
            context.wrap_socket(tcp, server_hostname="localhost")
    except ssl.SSLError:
        cases.append(
            {
                "name": "tls-on-plaintext-port",
                "category": "tls",
                "transport": "tls-to-plain-port",
                "outcome": "tls-handshake-rejected",
            }
        )
    else:
        raise AssertionError("TLS handshake unexpectedly succeeded on plaintext port")
    return cases


def proxy_cases() -> list[dict[str, Any]]:
    forwarded = [
        (b"Forwarded", b"for=192.0.2.10;host=rpc.provider.example;proto=https"),
        (b"X-Forwarded-For", b"192.0.2.10"),
        (b"X-Forwarded-Host", b"rpc.provider.example"),
        (b"X-Forwarded-Proto", b"https"),
    ]
    return [
        exchange("proxy-root-internal-host", "proxy", request(method=b"GET", host=b"backend.internal")),
        exchange("proxy-root-public-host", "proxy", request(method=b"GET", host=b"rpc.provider.example")),
        exchange(
            "proxy-forwarded-headers-inert",
            "proxy",
            request(method=b"GET", host=b"backend.internal", headers=forwarded),
        ),
        exchange(
            "proxy-cors-forwarded-headers-inert",
            "proxy",
            request(
                method=b"OPTIONS",
                target=b"/health",
                host=b"backend.internal",
                headers=forwarded
                + [
                    (b"Origin", b"https://allowed.example"),
                    (b"Access-Control-Request-Method", b"POST"),
                    (b"Access-Control-Request-Headers", b"Content-Type"),
                ],
            ),
        ),
        exchange(
            "proxy-health-forwarded-headers-inert",
            "proxy",
            request(method=b"GET", target=b"/health", host=b"backend.internal", headers=forwarded),
        ),
        exchange(
            "proxy-absolute-form-target",
            "proxy",
            request(method=b"GET", target=b"http://rpc.provider.example/", host=b"backend.internal"),
        ),
    ]


def framing_cases() -> list[dict[str, Any]]:
    length = str(len(HEALTH)).encode()
    cases: list[tuple[str, bytes, bool]] = [
        (
            "framing-content-length-duplicate-same",
            request(headers=[(b"Content-Length", length)], body=HEALTH),
            False,
        ),
        (
            "framing-content-length-comma-same",
            request(headers=[(b"Content-Length", length + b", " + length)], body=HEALTH, content_length=False),
            False,
        ),
        (
            "framing-transfer-encoding-with-content-length",
            request(
                headers=[(b"Transfer-Encoding", b"chunked"), (b"Content-Length", b"999")],
                body=chunked(HEALTH),
                content_length=False,
            ),
            False,
        ),
        (
            "framing-chunk-extension",
            request(
                headers=[(b"Transfer-Encoding", b"chunked")],
                body=chunked(HEALTH, extension=b";foo=bar"),
                content_length=False,
            ),
            False,
        ),
        (
            "framing-declared-trailer",
            request(
                headers=[(b"Transfer-Encoding", b"chunked"), (b"Trailer", b"X-Trace")],
                body=f"{len(HEALTH):x}".encode() + b"\r\n" + HEALTH + b"\r\n0\r\nX-Trace: done\r\n\r\n",
                content_length=False,
            ),
            False,
        ),
        ("framing-missing-host", request(host=None, body=HEALTH), False),
        (
            "framing-duplicate-host",
            request(host=b"one.example", headers=[(b"Host", b"two.example")], body=HEALTH),
            False,
        ),
        (
            "framing-header-name-space",
            b"POST / HTTP/1.1\r\nHost: localhost\r\nBad Header: value\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            False,
        ),
        (
            "framing-header-without-colon",
            b"POST / HTTP/1.1\r\nHost: localhost\r\nBroken\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            False,
        ),
        (
            "framing-obs-fold",
            b"POST / HTTP/1.1\r\nHost: localhost\r\nX-Test: one\r\n two\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            False,
        ),
        (
            "framing-nul-header-value",
            b"POST / HTTP/1.1\r\nHost: localhost\r\nX-Test: one\x00two\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            False,
        ),
        (
            "framing-invalid-method-token",
            b"GE T / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            False,
        ),
        (
            "framing-invalid-http-version",
            request(version=b"HTTP/1.2", body=HEALTH),
            False,
        ),
        (
            "framing-invalid-target-control",
            request(target=b"/bad\x00target", body=HEALTH),
            False,
        ),
        (
            "framing-content-length-different",
            request(headers=[(b"Content-Length", str(len(HEALTH) + 1).encode())], body=HEALTH),
            False,
        ),
        (
            "framing-content-length-negative",
            request(headers=[(b"Content-Length", b"-1")], body=b"", content_length=False),
            False,
        ),
        (
            "framing-content-length-plus",
            request(headers=[(b"Content-Length", b"+0")], body=b"", content_length=False),
            False,
        ),
        (
            "framing-content-length-nondigit",
            request(headers=[(b"Content-Length", b"nope")], body=b"", content_length=False),
            False,
        ),
        (
            "framing-transfer-encoding-gzip",
            request(headers=[(b"Transfer-Encoding", b"gzip")], body=b"", content_length=False),
            False,
        ),
        (
            "framing-transfer-encoding-nonfinal-chunked",
            request(headers=[(b"Transfer-Encoding", b"chunked, gzip")], body=chunked(HEALTH), content_length=False),
            False,
        ),
        (
            "framing-transfer-encoding-duplicate-chunked",
            request(headers=[(b"Transfer-Encoding", b"chunked, chunked")], body=chunked(HEALTH), content_length=False),
            False,
        ),
        (
            "framing-invalid-chunk-size",
            request(headers=[(b"Transfer-Encoding", b"chunked")], body=b"xyz\r\n", content_length=False),
            True,
        ),
        (
            "framing-invalid-chunk-terminator",
            request(
                headers=[(b"Transfer-Encoding", b"chunked")],
                body=f"{len(HEALTH):x}".encode() + b"\r\n" + HEALTH + b"xx0\r\n\r\n",
                content_length=False,
            ),
            True,
        ),
        (
            "framing-forbidden-trailer",
            request(
                headers=[(b"Transfer-Encoding", b"chunked"), (b"Trailer", b"Content-Length")],
                body=f"{len(HEALTH):x}".encode() + b"\r\n" + HEALTH + b"\r\n0\r\nContent-Length: 0\r\n\r\n",
                content_length=False,
            ),
            False,
        ),
        (
            "framing-truncated-content-length",
            request(headers=[(b"Content-Length", str(len(HEALTH) + 5).encode())], body=HEALTH, content_length=False),
            True,
        ),
        (
            "framing-truncated-chunk",
            request(headers=[(b"Transfer-Encoding", b"chunked")], body=b"10\r\nshort", content_length=False),
            True,
        ),
    ]
    return [
        exchange(
            name,
            "framing-valid-controls" if index < 5 else "framing-malformed-inputs",
            raw,
            half_close=half_close,
        )
        for index, (name, raw, half_close) in enumerate(cases)
    ]


def framing_connection_cases() -> list[dict[str, Any]]:
    second = request(
        headers=[(b"Content-Type", b"application/json")],
        body=HEALTH.replace(b'"id":1', b'"id":2'),
    )
    transfer_length = request(
        headers=[
            (b"Content-Type", b"application/json"),
            (b"Transfer-Encoding", b"chunked"),
            (b"Content-Length", b"999"),
        ],
        body=chunked(HEALTH),
        content_length=False,
        connection=b"keep-alive",
    )
    conflicting_length = request(
        headers=[
            (b"Content-Type", b"application/json"),
            (b"Content-Length", str(len(HEALTH) + 1).encode()),
        ],
        body=HEALTH,
        connection=b"keep-alive",
    )
    invalid_chunk = request(
        headers=[
            (b"Content-Type", b"application/json"),
            (b"Transfer-Encoding", b"chunked"),
        ],
        body=b"zz\r\n",
        content_length=False,
        connection=b"keep-alive",
    )
    return [
        pipeline_exchange("framing-transfer-length-keepalive", transfer_length, second),
        pipeline_exchange("framing-conflicting-length-closes", conflicting_length, second),
        pipeline_exchange("framing-invalid-chunk-closes", invalid_chunk, second),
    ]


def main() -> None:
    inventory = json.loads((HERE / "campaign.json").read_text())["categories"]
    cases = tls_special_cases() + proxy_cases() + framing_cases() + framing_connection_cases()
    expected = [
        name for category, names in inventory.items() if category != "tls-config" for name in names
    ]
    actual = [case["name"] for case in cases]
    if actual != expected:
        raise ValueError(f"case order differs from frozen inventory: {actual!r}")
    output = {
        "schema_version": 1,
        "captured_at": dt.datetime.now(dt.timezone.utc).isoformat(),
        "dynamic_response_headers_excluded_from_stable_projection": sorted(DYNAMIC),
        "cases": cases,
    }
    (HERE / "transport-capture.json").write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
