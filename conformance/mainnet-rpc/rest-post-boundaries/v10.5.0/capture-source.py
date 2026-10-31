#!/usr/bin/env python3
"""Capture bounded REST method, body, escaping, and protobuf-JSON behavior."""

from __future__ import annotations

import base64
import datetime
import hashlib
import json
import pathlib
import socket
from typing import Any


HOST = "127.0.0.1"
PORT = 6317
ROOT = pathlib.Path("/workspace/worktrees/rpc-191")
WORK = pathlib.Path("/workspace/scratch/rpc-181/rest-post")
MAX_RESPONSE = 4 << 20
DYNAMIC_RESPONSE_HEADERS = {
    "connection",
    "content-length",
    "date",
    "trailer",
    "transfer-encoding",
    "x-server-time",
}


def sha(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_varint(data: bytes, offset: int) -> tuple[int, int]:
    value = 0
    for shift in range(0, 70, 7):
        byte = data[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
    raise ValueError("oversized protobuf varint")


def length_field(data: bytes, number: int) -> bytes:
    offset = 0
    while offset < len(data):
        tag, offset = read_varint(data, offset)
        wire = tag & 7
        if wire == 0:
            _, offset = read_varint(data, offset)
            continue
        if wire != 2:
            raise ValueError(f"unsupported protobuf wire type {wire}")
        size, offset = read_varint(data, offset)
        value = data[offset : offset + size]
        if len(value) != size:
            raise ValueError("truncated protobuf field")
        offset += size
        if tag >> 3 == number:
            return value
    raise ValueError(f"protobuf field {number} not found")


def successful_tx() -> bytes:
    requests = json.loads(
        (ROOT / "conformance/mainnet-rpc/simulation/requests.json").read_text()
    )
    case = next(row for row in requests if row["name"] == "successful-simulation")
    return length_field(bytes.fromhex(case["request_hex"]), 2)


def parse_headers(raw: bytes) -> tuple[int, str, list[list[str]], bytes]:
    head, separator, rest = raw.partition(b"\r\n\r\n")
    if not separator:
        raise ValueError("response has no HTTP header terminator")
    lines = head.split(b"\r\n")
    status_line = lines[0].decode("latin1")
    _, status, reason = status_line.split(" ", 2)
    headers: list[list[str]] = []
    for line in lines[1:]:
        name, value = line.split(b":", 1)
        headers.append([name.decode("latin1"), value.strip().decode("latin1")])
    return int(status), reason, headers, rest


def header_values(headers: list[list[str]], name: str) -> list[str]:
    return [value for key, value in headers if key.lower() == name]


def decode_response(raw: bytes, method: str) -> dict[str, Any]:
    status, reason, headers, encoded = parse_headers(raw)
    trailers: list[list[str]] = []
    if method == "HEAD":
        body = b""
    elif "chunked" in ",".join(header_values(headers, "transfer-encoding")).lower():
        body = bytearray()
        offset = 0
        while True:
            end = encoded.find(b"\r\n", offset)
            if end < 0:
                raise ValueError("truncated chunk size")
            size = int(encoded[offset:end].split(b";", 1)[0], 16)
            offset = end + 2
            if size == 0:
                trailer_end = encoded.find(b"\r\n\r\n", offset)
                trailer_raw = encoded[offset:] if trailer_end < 0 else encoded[offset:trailer_end]
                for line in trailer_raw.split(b"\r\n"):
                    if not line:
                        continue
                    name, value = line.split(b":", 1)
                    trailers.append(
                        [name.decode("latin1"), value.strip().decode("latin1")]
                    )
                break
            chunk = encoded[offset : offset + size]
            if len(chunk) != size or encoded[offset + size : offset + size + 2] != b"\r\n":
                raise ValueError("truncated HTTP chunk")
            body.extend(chunk)
            offset += size + 2
        body = bytes(body)
    else:
        lengths = header_values(headers, "content-length")
        body = encoded[: int(lengths[-1])] if lengths else encoded
    stable = [
        pair
        for pair in headers
        if pair[0].lower() not in DYNAMIC_RESPONSE_HEADERS
    ]
    response: dict[str, Any] = {
        "status": status,
        "reason": reason,
        "headers": headers,
        "stable_headers": stable,
        "trailers": trailers,
        "body_base64": base64.b64encode(body).decode(),
        "body_sha256": sha(body),
        "raw_base64": base64.b64encode(raw).decode(),
        "raw_sha256": sha(raw),
        "raw_size": len(raw),
    }
    try:
        response["body_text"] = body.decode()
    except UnicodeDecodeError:
        pass
    return response


def request_bytes(
    method: str, target: str, headers: list[tuple[str, str]], body: bytes
) -> bytes:
    complete = [
        ("Host", f"{HOST}:{PORT}"),
        ("Connection", "close"),
        *headers,
    ]
    if not any(name.lower() in {"content-length", "transfer-encoding"} for name, _ in complete):
        complete.append(("Content-Length", str(len(body))))
    head = [f"{method} {target} HTTP/1.1", *(f"{name}: {value}" for name, value in complete)]
    return ("\r\n".join(head) + "\r\n\r\n").encode("latin1") + body


def exchange(raw: bytes, method: str) -> dict[str, Any]:
    with socket.create_connection((HOST, PORT), timeout=10) as connection:
        connection.settimeout(10)
        connection.sendall(raw)
        chunks = []
        size = 0
        while True:
            chunk = connection.recv(65536)
            if not chunk:
                break
            size += len(chunk)
            if size > MAX_RESPONSE:
                raise ValueError("REST response exceeds 4 MiB")
            chunks.append(chunk)
    return decode_response(b"".join(chunks), method)


def case(
    name: str,
    category: str,
    method: str,
    target: str,
    body: bytes = b"",
    headers: list[tuple[str, str]] | None = None,
) -> dict[str, Any]:
    headers = headers or []
    raw = request_bytes(method, target, headers, body)
    result = {
        "name": name,
        "category": category,
        "request": {
            "method": method,
            "target": target,
            "headers": [list(pair) for pair in headers],
            "body_base64": base64.b64encode(body).decode(),
            "raw_base64": base64.b64encode(raw).decode(),
            "raw_sha256": sha(raw),
            "raw_size": len(raw),
        },
        "response": exchange(raw, method),
    }
    print(name, result["response"]["status"], result["response"].get("body_text", "")[:100])
    return result


def compact(value: Any) -> bytes:
    return json.dumps(value, separators=(",", ":")).encode()


def main() -> None:
    cases: list[dict[str, Any]] = []
    add = lambda *args, **kwargs: cases.append(case(*args, **kwargs))
    json_header = [("Content-Type", "application/json")]
    routes = [
        ("batch", "/celestia/core/v1/tx/batch"),
        ("decode", "/cosmos/tx/v1beta1/decode"),
        ("decode-amino", "/cosmos/tx/v1beta1/decode/amino"),
        ("encode", "/cosmos/tx/v1beta1/encode"),
        ("encode-amino", "/cosmos/tx/v1beta1/encode/amino"),
        ("simulate", "/cosmos/tx/v1beta1/simulate"),
        ("broadcast", "/cosmos/tx/v1beta1/txs"),
        ("fibre-validate", "/fibre/v1/validate-payment-promise"),
        ("ibc-membership", "/ibc/core/client/v1/verify_membership"),
    ]
    for route_name, target in routes:
        add(f"route-{route_name}-empty-object", "supported-route", "POST", target, b"{}", json_header)
        add(f"route-{route_name}-empty-body", "supported-route", "POST", target, b"", json_header)

    decode = "/cosmos/tx/v1beta1/decode"
    for name, headers in [
        ("absent", []),
        ("json", [("Content-Type", "application/json")]),
        ("json-charset", [("Content-Type", "application/json; charset=utf-8")]),
        ("text", [("Content-Type", "text/plain")]),
        ("octet", [("Content-Type", "application/octet-stream")]),
        ("empty", [("Content-Type", "")]),
        (
            "repeated",
            [("Content-Type", "application/json"), ("Content-Type", "text/plain")],
        ),
    ]:
        add(f"content-type-{name}", "content-type", "POST", decode, b"{}", headers)

    for method in ["GET", "PUT", "PATCH", "DELETE", "OPTIONS", "HEAD", "post"]:
        add(f"method-{method.lower()}", "method", method, decode, b"{}", json_header)

    for name, target in [
        ("root", "/not-a-route"),
        ("known-service", "/cosmos/tx/v1beta1/not-a-method"),
        ("trailing-slash", "/cosmos/tx/v1beta1/decode/"),
        ("double-slash", "/cosmos/tx/v1beta1//decode"),
        ("encoded-name", "/cosmos/tx/v1beta1/%64ecode"),
        ("encoded-slash", "/cosmos/tx/v1beta1%2Fdecode"),
        ("invalid-percent", "/cosmos/tx/v1beta1/%ZZ"),
    ]:
        add(f"unknown-{name}", "unknown-route", "POST", target, b"{}", json_header)

    for name, method, target, body in [
        ("post-unused", "POST", decode + "?unused=x%20y+z", b"{}"),
        ("post-field", "POST", decode + "?tx_bytes=AA%3D%3D", b"{}"),
        ("get-unknown", "GET", "/cosmos/bank/v1beta1/params?unused=x%20y+z", b""),
        (
            "get-pagination",
            "GET",
            "/cosmos/bank/v1beta1/denoms_metadata?pagination.limit=01&pagination.count_total=true",
            b"",
        ),
        (
            "path-encoded-slash",
            "GET",
            "/cosmos/bank/v1beta1/balances/bad%2Fslash",
            b"",
        ),
        (
            "path-percent",
            "GET",
            "/cosmos/bank/v1beta1/balances/bad%2525",
            b"",
        ),
        ("path-plus", "GET", "/cosmos/bank/v1beta1/balances/a+b", b""),
        ("query-repeated", "GET", "/cosmos/bank/v1beta1/denoms_metadata?pagination.limit=1&pagination.limit=2", b""),
    ]:
        add(f"escape-{name}", "escaping", method, target, body, json_header if method == "POST" else [])

    json_edges = [
        ("malformed", b"{"),
        ("trailing-json", b"{} {}"),
        ("array", b"[]"),
        ("null", b"null"),
        ("string", b'""'),
        ("number", b"1"),
        ("boolean", b"true"),
        ("unknown-field", b'{"unknown":1}'),
        ("snake-empty", b'{"tx_bytes":""}'),
        ("camel-empty", b'{"txBytes":""}'),
        ("duplicate-alias", b'{"tx_bytes":"","txBytes":""}'),
        ("bytes-number", b'{"tx_bytes":1}'),
        ("bytes-array", b'{"tx_bytes":[]}'),
        ("bytes-object", b'{"tx_bytes":{}}'),
        ("bytes-null", b'{"tx_bytes":null}'),
        ("bytes-invalid-base64", b'{"tx_bytes":"!"}'),
        ("bytes-unpadded", b'{"tx_bytes":"AQ"}'),
        ("bytes-url-base64", b'{"tx_bytes":"-_=="}'),
        ("bytes-space", b'{"tx_bytes":"A A=="}'),
    ]
    for name, body in json_edges:
        add(f"json-{name}", "protobuf-json", "POST", decode, body, json_header)

    broadcast = "/cosmos/tx/v1beta1/txs"
    for name, body in [
        ("enum-unknown", {"tx_bytes": "", "mode": "NOPE"}),
        ("enum-large", {"tx_bytes": "", "mode": 2147483648}),
        ("enum-float", {"tx_bytes": "", "mode": 1.5}),
        ("enum-null", {"tx_bytes": "", "mode": None}),
        ("bytes-invalid", {"tx_bytes": "!", "mode": "BROADCAST_MODE_SYNC"}),
    ]:
        add(f"broadcast-{name}", "protobuf-json", "POST", broadcast, compact(body), json_header)

    for name, target, body in [
        ("encode-empty-tx", "/cosmos/tx/v1beta1/encode", {"tx": {}}),
        ("encode-nested-unknown", "/cosmos/tx/v1beta1/encode", {"tx": {"unknown": 1}}),
        ("amino-json-object", "/cosmos/tx/v1beta1/encode/amino", {"amino_json": "{}"}),
        ("amino-json-value", "/cosmos/tx/v1beta1/encode/amino", {"amino_json": "1"}),
        ("amino-binary-empty", "/cosmos/tx/v1beta1/decode/amino", {"amino_binary": ""}),
        ("ibc-int-string", "/ibc/core/client/v1/verify_membership", {"time_delay": "1", "block_delay": "2"}),
        ("ibc-int-number", "/ibc/core/client/v1/verify_membership", {"time_delay": 1, "block_delay": 2}),
        ("ibc-int-overflow", "/ibc/core/client/v1/verify_membership", {"time_delay": "18446744073709551616"}),
    ]:
        add(name, "protobuf-json", "POST", target, compact(body), json_header)

    tx = successful_tx()
    encoded_tx = base64.b64encode(tx).decode()
    add(
        "decode-valid-tx",
        "success-control",
        "POST",
        decode,
        compact({"tx_bytes": encoded_tx}),
        json_header,
    )
    add(
        "simulate-valid-tx",
        "success-control",
        "POST",
        "/cosmos/tx/v1beta1/simulate",
        compact({"tx_bytes": encoded_tx}),
        json_header,
    )
    decode_result = cases[-2]["response"]
    if decode_result["status"] == 200:
        decoded = json.loads(decode_result["body_text"])
        add(
            "encode-decoded-roundtrip",
            "success-control",
            "POST",
            "/cosmos/tx/v1beta1/encode",
            compact({"tx": decoded["tx"]}),
            json_header,
        )

    sources = [
        pathlib.Path(__file__),
        ROOT / "conformance/mainnet-rpc/simulation/requests.json",
        ROOT / "conformance/fixtures/devnet/empty/abci.rec",
        ROOT / "conformance/fixtures/devnet/empty/genesis.json",
    ]
    artifact = {
        "schema_version": 1,
        "captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "endpoint": f"http://{HOST}:{PORT}",
        "dynamic_response_headers_excluded": sorted(DYNAMIC_RESPONSE_HEADERS),
        "source_sha256": {str(path): sha(path.read_bytes()) for path in sources},
        "cases": cases,
    }
    output = WORK / "go-rest-boundaries.json"
    output.write_text(json.dumps(artifact, indent=2) + "\n")
    print(f"wrote {len(cases)} cases to {output}")


if __name__ == "__main__":
    main()
