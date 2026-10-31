#!/usr/bin/env python3
"""Derive embedded immutable registries from the reviewed transport captures."""
import gzip
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


def varint(number):
    encoded = bytearray()
    while number > 127:
        encoded.append((number & 127) | 128)
        number >>= 7
    encoded.append(number)
    return bytes(encoded)


for profile in ("app", "core"):
    capture = ROOT / "conformance/grpc-reflection/v10.5.0" / f"{profile}.json.gz"
    data = json.loads(gzip.decompress(capture.read_bytes()))
    descriptors = [bytes.fromhex(raw) for _, raw in sorted(data["files"].items())]
    wire = b"".join(b"\x0a" + varint(len(raw)) + raw for raw in descriptors)
    dest = ROOT / "crates/tier-4-runtime/grpc-reflection/src"
    (dest / f"{profile}-files.bin.gz").write_bytes(gzip.compress(wire, compresslevel=9, mtime=0))
    (dest / f"{profile}-services.txt").write_text("\n".join(data["services"]) + "\n")

    (dest / f"{profile}-extensions.txt").write_text("".join(
        f"{name} {number}\n" for name, numbers in sorted(data["extensions"].items()) for number in sorted(numbers)
    ))

    lookups = bytearray()
    for key, files in sorted(data["lookup_files"].items()):
        key = key.encode()
        entry = b"\x0a" + varint(len(key)) + key
        for raw in files:
            raw = bytes.fromhex(raw)
            entry += b"\x12" + varint(len(raw)) + raw
        lookups += b"\x0a" + varint(len(entry)) + entry
    (dest / f"{profile}-lookups.bin.gz").write_bytes(gzip.compress(bytes(lookups), compresslevel=9, mtime=0))
