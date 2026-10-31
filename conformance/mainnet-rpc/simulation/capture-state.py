#!/usr/bin/env python3
"""Record the frozen local app's raw ABCI Info and committed state identity."""
import argparse
import ipaddress
import json
from pathlib import Path
import socket
import struct


def encode_varint(value):
    out = bytearray()
    while value > 127:
        out.append(value & 127 | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def varint(stream):
    value = 0
    for shift in range(0, 70, 7):
        byte = stream.read(1)
        if not byte:
            raise EOFError("truncated ABCI varint")
        value |= (byte[0] & 127) << shift
        if byte[0] < 128:
            return value
    raise ValueError("oversized ABCI varint")


def fields(data):
    import io
    stream = io.BytesIO(data)
    result = {}
    while stream.tell() < len(data):
        tag = varint(stream)
        if tag & 7 == 0:
            value = varint(stream)
        elif tag & 7 == 2:
            size = varint(stream)
            value = stream.read(size)
            if len(value) != size:
                raise EOFError("truncated protobuf field")
        else:
            raise ValueError("unexpected Info wire type")
        result[tag >> 3] = value
    return result


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--address", required=True)
parser.add_argument("--genesis", required=True, type=Path)
parser.add_argument("--out", required=True, type=Path)
parser.add_argument("--replay", type=Path, help="Replay only recorded public ABCI requests; recorded answers are ignored")
args = parser.parse_args()
host, port = args.address.rsplit(":", 1)
if not ipaddress.ip_address(host).is_loopback:
    parser.error("ABCI capture only supports explicit loopback addresses")
replayed = 0
if args.replay:
    data = args.replay.read_bytes()
    offset = 0
    connections = {}
    try:
        while offset < len(data):
            if len(data) - offset < 9:
                raise ValueError("truncated recording header")
            connection, direction, size = struct.unpack_from("<IBI", data, offset)
            offset += 9
            payload = data[offset:offset + size]
            offset += size
            if len(payload) != size or direction not in (0, 1):
                raise ValueError("invalid recording frame")
            if direction == 1:
                continue
            if connection not in connections:
                sock = socket.create_connection((host, int(port)), timeout=10)
                connections[connection] = [sock, sock.makefile("rb"), 0]
            state = connections[connection]
            state[0].sendall(encode_varint(size) + payload)
            state[2] += 1
            replayed += 1
            if 2 in fields(payload):
                while state[2]:
                    length = varint(state[1])
                    if length > 64 << 20:
                        raise ValueError("ABCI replay response exceeds 64 MiB")
                    response = state[1].read(length)
                    if len(response) != length or 1 in fields(response):
                        raise ValueError("failed or truncated ABCI replay response")
                    state[2] -= 1
        if any(state[2] for state in connections.values()):
            raise ValueError("recording ended without flushing pending responses")
    finally:
        for sock, reader, _ in connections.values():
            reader.close()
            sock.close()
with socket.create_connection((host, int(port)), timeout=10) as sock:
    sock.sendall(b"\x02\x1a\x00\x02\x12\x00")
    reader = sock.makefile("rb")
    size = varint(reader)
    if size > 1 << 20:
        raise ValueError("ABCI Info exceeds 1 MiB")
    raw = reader.read(size)
    if len(raw) != size:
        raise EOFError("truncated ABCI Info")
info = fields(fields(raw)[4])
result = {"replayed_requests": replayed, "abci_info_hex": raw.hex(), "data": info[1].decode(), "version": info[2].decode(),
          "app_version": info[3], "height": info[4], "app_hash": info[5].hex().upper(),
          "chain_id": json.loads(args.genesis.read_text())["chain_id"]}
args.out.write_text(json.dumps(result, indent=2) + "\n")
