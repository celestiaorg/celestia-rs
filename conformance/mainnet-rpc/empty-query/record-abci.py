#!/usr/bin/env python3
"""Capture bounded ABCI Query/Flush frames from an explicit loopback Go app."""
import argparse
import datetime
import hashlib
import ipaddress
import json
import pathlib
import socket

from record import MAX_BYTES, PATHS, TIMEOUT, recv_exact


def varint(number):
    out = bytearray()
    while number > 127:
        out.append((number & 127) | 128)
        number >>= 7
    return bytes(out) + bytes([number])


def frame(payload):
    return varint(len(payload)) + payload


def read(sock):
    size = 0
    prefix = bytearray()
    for shift in range(0, 70, 7):
        byte = recv_exact(sock, 1)[0]
        prefix.append(byte)
        size |= (byte & 127) << shift
        if not byte & 128:
            break
    else:
        raise ValueError("ABCI length varint overflow")
    if size > MAX_BYTES:
        raise ValueError("ABCI response exceeds capture limit")
    payload = recv_exact(sock, size)
    return {"wire_hex": (prefix + payload).hex(), "payload_hex": payload.hex()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--abci", required=True, help="IP:port, loopback only")
    parser.add_argument("--out", required=True)
    parser.add_argument("--binary", required=True)
    args = parser.parse_args()
    host, port = args.abci.rsplit(":", 1)
    if not ipaddress.ip_address(host).is_loopback:
        parser.error("only explicit loopback addresses are allowed")
    cases = []
    with socket.create_connection((host, int(port)), TIMEOUT) as sock:
        sock.settimeout(TIMEOUT)
        for path in PATHS:
            # Request.query (6), RequestQuery.path (2), then Request.flush (2).
            data = path.encode()
            query = b"\x12" + varint(len(data)) + data
            request = frame(b"\x32" + varint(len(query)) + query)
            flush = frame(b"\x12\x00")
            sock.sendall(request + flush)
            response, flushed = read(sock), read(sock)
            assert flushed["payload_hex"] == "1a00", flushed
            cases.append({"query_path": path, "request_wire_hex": request.hex(),
                          "flush_wire_hex": flush.hex(), "response": response,
                          "flush_response": flushed,
                          "missing_response": response["payload_hex"] == ""})
        echo = frame(b"\x0a\x07\x0a\x05alive")
        sock.sendall(echo + flush)
        health, flushed = read(sock), read(sock)
        assert health["payload_hex"] == "12070a05616c697665" and flushed["payload_hex"] == "1a00"
    output = {"captured_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "abci": args.abci, "cases": cases, "echo_after": health,
              "echo_request_wire_hex": echo.hex(), "flush_after": flushed,
              "binary_sha256": hashlib.sha256(pathlib.Path(args.binary).read_bytes()).hexdigest(),
              "capture_source_sha256": hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest()}
    pathlib.Path(args.out).write_text(json.dumps(output, indent=2) + "\n")


if __name__ == "__main__":
    main()
