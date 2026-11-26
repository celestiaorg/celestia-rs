#!/usr/bin/env python3
"""Verify a Go SDK snapshot against the signed app hash before a replay window.

Pass the `<snapshot root>/<height>/<format>` chunk directory. The script
rebuilds every IAVL root from the snapshot stream, hashes the commit info and
compares it with the first header app hash of the window that starts at
`height + 1`. When the fixture pins that window's pre-state, it also names each
store whose root differs. For a height where no window starts, pass the app
hash of the signed header at `height + 1` with `--app-hash`. The snapshot must be complete: a truncated stream
fails. This checks a checkpoint independently of celestia-rs.
"""

import argparse
import hashlib
import json
from pathlib import Path
import zlib


FIXTURE = Path(__file__).resolve().parents[1] / "fixtures" / "mainnet" / "replay-windows.json"
# Format 2 is Cosmos SDK v0.46, which also commits this memory store.
FORMAT_2_MEMORY_STORES = ["mem_capability"]


def uvarint(data, i):
    shift = value = 0
    while True:
        byte = data[i]
        i += 1
        value |= (byte & 0x7F) << shift
        if byte < 0x80:
            return value, i
        shift += 7


def put_uvarint(value):
    out = bytearray()
    while value >= 0x80:
        out.append((value & 0x7F) | 0x80)
        value >>= 7
    out.append(value)
    return bytes(out)


def put_varint(value):
    return put_uvarint(value << 1 if value >= 0 else (-value << 1) - 1)


def fields(data):
    i = 0
    while i < len(data):
        tag, i = uvarint(data, i)
        if tag & 7 == 0:
            value, i = uvarint(data, i)
        elif tag & 7 == 2:
            size, i = uvarint(data, i)
            value, i = data[i:i + size], i + size
        else:
            raise ValueError(f"unexpected wire type {tag & 7}")
        yield tag >> 3, value


def sha256(data):
    return hashlib.sha256(data).digest()


def items(directory):
    """Yields the length-delimited snapshot items of the zlib chunk stream."""
    chunks = sorted((p for p in directory.iterdir() if p.name.isdigit()), key=lambda p: int(p.name))
    if [int(p.name) for p in chunks] != list(range(len(chunks))) or not chunks:
        raise ValueError(f"chunk files in {directory} are not 0..n-1")
    decoder, buffer, offset, adler = zlib.decompressobj(), b"", 0, 1
    for chunk in chunks:
        with chunk.open("rb") as handle:
            while data := handle.read(1 << 22):
                out = decoder.decompress(data)
                adler = zlib.adler32(out, adler)
                buffer = buffer[offset:] + out
                offset = 0
                while True:
                    try:
                        size, start = uvarint(buffer, offset)
                    except IndexError:
                        break
                    if start + size > len(buffer):
                        break
                    yield buffer[start:start + size]
                    offset = start + size
    # Cosmos SDK v0.46 closes its zlib writer twice (StreamWriter.Close), so
    # format 2 streams end with the Adler-32 trailer written a second time.
    # Go's reader ignores those four bytes; accept exactly them.
    trailer = adler.to_bytes(4, "big")
    if not decoder.eof or offset != len(buffer) or decoder.unused_data not in (b"", trailer):
        raise ValueError("snapshot stream is truncated or has trailing data")


def store_roots(directory):
    """Rebuilds each store's IAVL root from its post-order export nodes."""
    roots, name, stack = {}, None, []

    def finish():
        if name is not None:
            if len(stack) > 1:
                raise ValueError(f"store {name} has {len(stack)} unresolved subtrees")
            roots[name] = stack[0][2] if stack else sha256(b"")

    for item in items(directory):
        for field, value in fields(item):
            if field == 1:
                finish()
                name, stack = dict(fields(value)).get(1, b"").decode(), []
                if name in roots:
                    raise ValueError(f"duplicate store {name}")
            elif field == 2 and name is not None:
                node = {1: b"", 2: b"", 3: 0, 4: 0} | dict(fields(value))
                key, val, version, height = node[1], node[2], node[3], node[4]
                if height == 0:
                    body = put_varint(0) + put_varint(1) + put_varint(version)
                    body += put_uvarint(len(key)) + key + put_uvarint(32) + sha256(val)
                    stack.append((0, 1, sha256(body)))
                else:
                    if len(stack) < 2 or max(stack[-1][0], stack[-2][0]) >= height:
                        raise ValueError(f"malformed inner node in store {name}")
                    right, left = stack.pop(), stack.pop()
                    size = left[1] + right[1]
                    body = put_varint(height) + put_varint(size) + put_varint(version)
                    body += put_uvarint(32) + left[2] + put_uvarint(32) + right[2]
                    stack.append((height, size, sha256(body)))
            else:
                raise ValueError(f"unexpected snapshot item field {field}")
    finish()
    return {name: root.hex().upper() for name, root in roots.items()}


def merkle_root(leaves):
    if len(leaves) == 1:
        return sha256(b"\0" + leaves[0])
    split = 1
    while split * 2 < len(leaves):
        split *= 2
    return sha256(b"\1" + merkle_root(leaves[:split]) + merkle_root(leaves[split:]))


def commit_hash(roots):
    leaves = []
    for name in sorted(roots):
        value = sha256(bytes.fromhex(roots[name]))
        leaves.append(put_uvarint(len(name)) + name.encode() + put_uvarint(32) + value)
    return merkle_root(leaves).hex().upper()


def app_hash(value):
    if len(value) != 64 or any(c not in "0123456789abcdefABCDEF" for c in value):
        raise argparse.ArgumentTypeError("expected 64 hex digits")
    return value.upper()


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("chunks", type=Path, help="<snapshot root>/<height>/<format> directory")
    parser.add_argument(
        "--app-hash",
        type=app_hash,
        help="app hash of the signed header at height + 1, for a height where no replay window starts",
    )
    args = parser.parse_args(argv)
    directory = args.chunks.resolve()
    height, snapshot_format = int(directory.parent.name), int(directory.name)
    windows = json.loads(FIXTURE.read_text())["windows"]
    window = next((w for w in windows if w["start_height"] == height + 1), None)
    if window is None and args.app_hash is None:
        raise SystemExit(f"no replay window starts at {height + 1}")

    roots = store_roots(directory)
    if snapshot_format == 2:
        roots.update({name: "" for name in FORMAT_2_MEMORY_STORES})
    pinned = ((window or {}).get("pre_state") or {}).get("stores", {})
    for name in sorted(set(roots) | set(pinned)):
        if pinned and roots.get(name) != pinned.get(name):
            print(f"store {name}: snapshot {roots.get(name)}, mainnet {pinned.get(name)}")
    expected = args.app_hash or window["blocks"][0]["app_hash"]
    actual = commit_hash(roots)
    if actual != expected:
        raise SystemExit(f"app hash at {height}: snapshot {actual}, signed header {expected}")
    version = f"app v{window['app_version']}" if window else f"format {snapshot_format}"
    print(f"Verified {version} checkpoint at {height}: app hash {actual}")


if __name__ == "__main__":
    main()
