#!/usr/bin/env python3
"""Capture bounded celestia-core HTTP precheck and connection behavior."""

from __future__ import annotations

import base64
import datetime
import hashlib
import json
import pathlib
import socket
from typing import Any


HOST = "127.0.0.1"
WORK = pathlib.Path("/workspace/scratch/rpc-181/comet-http")
PORTS = {"zero": 6320, "custom": 6330}
LIMITS = {
    "zero": {"batch": 0, "body": 4096, "header": 256},
    "custom": {"batch": 3, "body": 512, "header": 256},
}
DYNAMIC_RESPONSE_HEADERS = {
    "connection",
    "content-length",
    "date",
    "transfer-encoding",
    "x-server-time",
}
MAX_RESPONSE = 4 << 20


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def compact(value: Any) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode()


class HTTPConnection:
    def __init__(self, port: int):
        self.socket = socket.create_connection((HOST, port), timeout=5)
        self.socket.settimeout(5)
        self.buffer = bytearray()

    def close(self) -> None:
        self.socket.close()

    def send(self, raw: bytes) -> None:
        self.socket.sendall(raw)

    def _receive(self) -> bool:
        try:
            chunk = self.socket.recv(65536)
        except ConnectionResetError:
            return False
        if not chunk:
            return False
        self.buffer.extend(chunk)
        if len(self.buffer) > MAX_RESPONSE:
            raise ValueError("HTTP response exceeds 4 MiB")
        return True

    def _through(self, marker: bytes) -> bytes:
        while marker not in self.buffer:
            if not self._receive():
                raise EOFError("connection closed before HTTP delimiter")
        end = self.buffer.index(marker) + len(marker)
        value = bytes(self.buffer[:end])
        del self.buffer[:end]
        return value

    def _exact(self, size: int) -> bytes:
        while len(self.buffer) < size:
            if not self._receive():
                raise EOFError("connection closed before HTTP body completed")
        value = bytes(self.buffer[:size])
        del self.buffer[:size]
        return value

    def response(self, method: str) -> dict[str, Any]:
        head_raw = self._through(b"\r\n\r\n")
        lines = head_raw[:-4].split(b"\r\n")
        version, status_raw, reason_raw = lines[0].split(b" ", 2)
        headers: list[list[str]] = []
        for line in lines[1:]:
            name, value = line.split(b":", 1)
            headers.append([name.decode("latin1"), value.strip().decode("latin1")])
        lookup: dict[str, list[str]] = {}
        for name, value in headers:
            lookup.setdefault(name.lower(), []).append(value)
        status = int(status_raw)
        trailers: list[list[str]] = []
        encoded = bytearray()
        body = bytearray()
        if method == "HEAD" or 100 <= status < 200 or status in (204, 304):
            pass
        elif "chunked" in ",".join(lookup.get("transfer-encoding", [])).lower():
            while True:
                line = self._through(b"\r\n")
                encoded.extend(line)
                size = int(line[:-2].split(b";", 1)[0], 16)
                if size == 0:
                    trailer_raw = self._through(b"\r\n")
                    encoded.extend(trailer_raw)
                    while trailer_raw != b"\r\n":
                        name, value = trailer_raw[:-2].split(b":", 1)
                        trailers.append(
                            [name.decode("latin1"), value.strip().decode("latin1")]
                        )
                        trailer_raw = self._through(b"\r\n")
                        encoded.extend(trailer_raw)
                    break
                chunk = self._exact(size + 2)
                if chunk[-2:] != b"\r\n":
                    raise ValueError("invalid response chunk terminator")
                encoded.extend(chunk)
                body.extend(chunk[:-2])
        elif lookup.get("content-length"):
            size = int(lookup["content-length"][-1])
            value = self._exact(size)
            encoded.extend(value)
            body.extend(value)
        else:
            while self._receive():
                pass
            body.extend(self.buffer)
            encoded.extend(self.buffer)
            self.buffer.clear()
        raw = head_raw + bytes(encoded)
        stable_headers = [
            pair
            for pair in headers
            if pair[0].lower() not in DYNAMIC_RESPONSE_HEADERS
        ]
        result: dict[str, Any] = {
            "http_version": version.decode("latin1"),
            "status": status,
            "reason": reason_raw.decode("latin1"),
            "headers": headers,
            "stable_headers": stable_headers,
            "trailers": trailers,
            "body_base64": base64.b64encode(body).decode(),
            "body_sha256": sha(body),
            "raw_base64": base64.b64encode(raw).decode(),
            "raw_sha256": sha(raw),
            "raw_size": len(raw),
        }
        try:
            result["body_text"] = body.decode()
        except UnicodeDecodeError:
            pass
        return result


def request(
    port: int,
    method: str = "POST",
    target: str = "/",
    body: bytes = b"",
    headers: list[tuple[str, str]] | None = None,
    connection: str = "close",
    chunked: bool = False,
) -> bytes:
    fields = [
        ("Host", f"{HOST}:{port}"),
        ("Connection", connection),
        *(headers or []),
    ]
    if chunked:
        fields.append(("Transfer-Encoding", "chunked"))
        encoded = f"{len(body):x}\r\n".encode() + body + b"\r\n0\r\n\r\n"
    else:
        fields.append(("Content-Length", str(len(body))))
        encoded = body
    head = [
        f"{method} {target} HTTP/1.1",
        *(f"{name}: {value}" for name, value in fields),
    ]
    return ("\r\n".join(head) + "\r\n\r\n").encode("latin1") + encoded


def request_record(raw: bytes, method: str, target: str, body: bytes) -> dict[str, Any]:
    return {
        "method": method,
        "target": target,
        "decoded_body_size": len(body),
        "raw_base64": base64.b64encode(raw).decode(),
        "raw_sha256": sha(raw),
        "raw_size": len(raw),
    }


def exchange(
    oracle: str,
    name: str,
    category: str,
    *,
    method: str = "POST",
    target: str = "/",
    body: bytes = b"",
    headers: list[tuple[str, str]] | None = None,
    chunked: bool = False,
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    port = PORTS[oracle]
    raw = request(port, method, target, body, headers, chunked=chunked)
    connection = HTTPConnection(port)
    try:
        connection.send(raw)
        response = connection.response(method)
    finally:
        connection.close()
    result = {
        "name": name,
        "category": category,
        "oracle": oracle,
        "request": request_record(raw, method, target, body),
        "response": response,
    }
    if metadata:
        result["metadata"] = metadata
    print(name, response["status"], response.get("body_text", "")[:100])
    return result


def rpc(identifier: int | None, method: str = "health") -> dict[str, Any]:
    value: dict[str, Any] = {"jsonrpc": "2.0", "method": method}
    if identifier is not None:
        value["id"] = identifier
    return value


def sized_health(size: int) -> bytes:
    body = compact(rpc(1))
    if len(body) > size:
        raise ValueError("requested body size is smaller than health request")
    return body + b" " * (size - len(body))


def keepalive_case(
    oracle: str,
    name: str,
    first_body: bytes,
    *,
    first_target: str = "/",
) -> dict[str, Any]:
    port = PORTS[oracle]
    first = request(
        port,
        "POST",
        first_target,
        first_body,
        [("Content-Type", "application/json")],
        connection="keep-alive",
    )
    second_body = compact(rpc(900))
    second = request(
        port,
        "POST",
        "/",
        second_body,
        [("Content-Type", "application/json")],
        connection="close",
    )
    connection = HTTPConnection(port)
    exchanges = []
    outcome = "second-response"
    try:
        connection.send(first)
        exchanges.append(
            {
                "request": request_record(first, "POST", first_target, first_body),
                "response": connection.response("POST"),
            }
        )
        try:
            connection.send(second)
            exchanges.append(
                {
                    "request": request_record(second, "POST", "/", second_body),
                    "response": connection.response("POST"),
                }
            )
        except (BrokenPipeError, ConnectionResetError, EOFError, socket.timeout) as error:
            outcome = f"closed-before-second-response:{type(error).__name__}"
    finally:
        connection.close()
    print(name, outcome, [e["response"]["status"] for e in exchanges])
    return {
        "name": name,
        "category": "keep-alive",
        "oracle": oracle,
        "outcome": outcome,
        "exchanges": exchanges,
    }


def main() -> None:
    cases: list[dict[str, Any]] = []
    add = lambda *args, **kwargs: cases.append(exchange(*args, **kwargs))
    json_header = [("Content-Type", "application/json")]

    # Batch limits and notification filtering.
    add("custom", "custom-batch-at-limit", "batch", body=compact([rpc(i) for i in range(3)]), headers=json_header)
    add("custom", "custom-batch-over-limit", "batch", body=compact([rpc(i) for i in range(4)]), headers=json_header)
    add("custom", "custom-notifications-over-limit", "batch", body=compact([rpc(None) for _ in range(4)]), headers=json_header)
    add("custom", "custom-over-limit-with-scalar", "batch", body=compact([rpc(1), rpc(2), rpc(3), 1]), headers=json_header)
    add("zero", "zero-batch-four", "batch", body=compact([rpc(i) for i in range(4)]), headers=json_header)
    add("zero", "zero-batch-fifty", "batch", body=compact([rpc(i) for i in range(50)]), headers=json_header)
    add("custom", "single-notification", "notification", body=compact(rpc(None)), headers=json_header)
    add("custom", "notification-only-batch", "notification", body=compact([rpc(None), rpc(None)]), headers=json_header)
    add("custom", "mixed-notification-batch", "notification", body=compact([rpc(None), rpc(71), rpc(None)]), headers=json_header)
    add("custom", "empty-batch", "notification", body=b"[]", headers=json_header)

    # Exact decoded-body limits for Content-Length and chunked transfer coding.
    for size in (511, 512, 513):
        body = sized_health(size)
        add("custom", f"body-content-length-{size}", "body-boundary", body=body, headers=json_header, metadata={"configured_limit": 512})
        add("custom", f"body-chunked-{size}", "body-boundary", body=body, headers=json_header, chunked=True, metadata={"configured_limit": 512})

    # Precheck, route, and CORS/preflight precedence.
    preflight = [
        ("Origin", "https://allowed.example"),
        ("Access-Control-Request-Method", "POST"),
    ]
    disallowed = [
        ("Origin", "https://denied.example"),
        ("Access-Control-Request-Method", "POST"),
    ]
    batch4 = compact([rpc(i) for i in range(4)])
    add("custom", "preflight-normal-allowed", "precedence", method="OPTIONS", body=b"", headers=preflight)
    add("custom", "preflight-normal-denied-origin", "precedence", method="OPTIONS", body=b"", headers=disallowed)
    add("custom", "preflight-disallowed-request-header", "precedence", method="OPTIONS", body=b"", headers=preflight + [("Access-Control-Request-Headers", "X-Not-Allowed")])
    add("custom", "preflight-over-batch", "precedence", method="OPTIONS", body=batch4, headers=preflight)
    add("custom", "preflight-over-body", "precedence", method="OPTIONS", body=sized_health(513), headers=preflight)
    add("zero", "zero-preflight-over-body-unread", "precedence", method="OPTIONS", body=b"x" * 4097, headers=preflight)
    add("custom", "post-normal-cors", "precedence", body=compact(rpc(81)), headers=json_header + [("Origin", "https://allowed.example")])
    add("custom", "post-over-batch-cors", "precedence", body=batch4, headers=json_header + [("Origin", "https://allowed.example")])
    add("custom", "post-over-body-cors", "precedence", body=sized_health(513), headers=json_header + [("Origin", "https://allowed.example")])
    add("custom", "missing-path-over-batch", "precedence", target="/missing", body=batch4, headers=json_header)
    add("custom", "malformed-json-under-body", "precedence", body=b"{", headers=json_header)

    # Find the actual net/http header cutoff. MaxHeaderBytes includes an
    # implementation read-buffer allowance, so preserve every bounded probe.
    low, high = 0, 8192
    seen: dict[int, dict[str, Any]] = {}
    while high - low > 1:
        pad = (low + high) // 2
        result = exchange(
            "custom",
            f"header-search-pad-{pad}",
            "header-boundary-search",
            method="GET",
            target="/health",
            headers=[("X-Pad", "a" * pad)],
            metadata={"configured_limit": 256, "pad_bytes": pad},
        )
        seen[pad] = result
        if result["response"]["status"] == 431:
            high = pad
        else:
            low = pad
    cases.extend(seen[pad] for pad in sorted(seen))
    print("header cutoff", low, high)

    # Connection reuse after success, batch rejection, and body rejection.
    cases.append(keepalive_case("custom", "keepalive-success", compact(rpc(801))))
    cases.append(keepalive_case("custom", "keepalive-after-batch-error", batch4))
    cases.append(keepalive_case("custom", "keepalive-after-body-error", sized_health(513)))

    artifact = {
        "schema_version": 1,
        "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "oracles": {
            name: {"endpoint": f"http://{HOST}:{PORTS[name]}", "limits": LIMITS[name]}
            for name in PORTS
        },
        "dynamic_response_headers_excluded": sorted(DYNAMIC_RESPONSE_HEADERS),
        "header_search": {"last_accepted_pad": low, "first_rejected_pad": high},
        "sources_sha256": {
            str(path): sha(path.read_bytes())
            for path in [pathlib.Path(__file__), WORK / "start-oracle.sh", WORK / "mainnet-observation.json"]
        },
        "cases": cases,
    }
    output = WORK / "go-comet-http-boundaries.json"
    output.write_text(json.dumps(artifact, indent=2) + "\n")
    print(f"wrote {len(cases)} cases to {output}")


if __name__ == "__main__":
    main()
