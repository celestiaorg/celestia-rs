#!/usr/bin/env python3
"""Fetch the format-3 Mainnet checkpoint at 15,558,000 for historical replay."""

import argparse
from concurrent.futures import ThreadPoolExecutor
import hashlib
from pathlib import Path
import tarfile
import tempfile
import time
from urllib.request import Request, urlopen


URL = "https://snaps.qubelabs.io/celestia/celestia_2026-11-03.tar"
ETAG = '"6aea4a03-95f79f8e800"'
ARCHIVE_SIZE = 10_305_672_898_560
HEIGHT = 15_558_000
FORMAT = 3
CHUNKS = 69
CHUNK_SIZE = 10_000_000
LAST_SIZE = 3_688_602
FIRST_HEADER = 44_984_958_464
TAR_BLOCK = 512
# SDK snapshot catalog in docs/evidence/validator-qualification-closeout-2026-11-13.
SHA256 = "46cd5415fbdf58471eeb8bccd910df5ddc318a0b7d34925034bda1d109b12b6e"


def chunk_size(index):
    return LAST_SIZE if index == CHUNKS - 1 else CHUNK_SIZE


def fetch_chunk(directory, index):
    stride = TAR_BLOCK * (1 + (CHUNK_SIZE + TAR_BLOCK - 1) // TAR_BLOCK)
    start = FIRST_HEADER + index * stride
    size = chunk_size(index)
    end = start + TAR_BLOCK + size - 1
    request = Request(
        f"{URL}?chunk={HEIGHT}-{index}",
        headers={"Range": f"bytes={start}-{end}", "If-Match": ETAG},
    )
    for attempt in range(4):
        try:
            with urlopen(request, timeout=60) as response:
                if (
                    response.status != 206
                    or response.headers.get("ETag") != ETAG
                    or response.headers.get("Content-Range")
                    != f"bytes {start}-{end}/{ARCHIVE_SIZE}"
                ):
                    raise ValueError(f"archive identity or range mismatch for chunk {index}")
                member = tarfile.TarInfo.frombuf(
                    response.read(TAR_BLOCK), encoding="utf-8", errors="strict"
                )
                if (
                    not member.isfile()
                    or member.name != f"./snapshots/{HEIGHT}/{FORMAT}/{index}"
                    or member.size != size
                ):
                    raise ValueError(f"unexpected archive member for chunk {index}: {member.name}")
                data = response.read(size + 1)
                if len(data) != size:
                    raise ValueError(f"wrong length for chunk {index}: {len(data)}")
            (directory / str(index)).write_bytes(data)
            return
        except OSError:
            if attempt == 3:
                raise
            time.sleep(attempt + 1)


def verify(directory):
    actual = {path.name for path in directory.iterdir()}
    if actual != {str(index) for index in range(CHUNKS)}:
        raise ValueError(f"incorrect chunk indexes in {directory}")
    digest = hashlib.sha256()
    for index in range(CHUNKS):
        path = directory / str(index)
        if path.stat().st_size != chunk_size(index):
            raise ValueError(f"incorrect chunk size in {path}")
        with path.open("rb") as chunk:
            while data := chunk.read(1024 * 1024):
                digest.update(data)
    if digest.hexdigest() != SHA256:
        raise ValueError(f"checkpoint SHA-256 mismatch in {directory}: {digest.hexdigest()}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("root", type=Path, help="destination snapshot root")
    args = parser.parse_args()
    root = args.root.resolve()
    root.mkdir(parents=True, exist_ok=True)
    destination = root / str(HEIGHT)
    if destination.exists():
        verify(destination / str(FORMAT))
    else:
        with tempfile.TemporaryDirectory(prefix=".mainnet-checkpoint-", dir=root) as scratch:
            staged = Path(scratch) / str(HEIGHT)
            chunks = staged / str(FORMAT)
            chunks.mkdir(parents=True)
            with ThreadPoolExecutor(max_workers=8) as pool:
                list(pool.map(lambda index: fetch_chunk(chunks, index), range(CHUNKS)))
            verify(chunks)
            staged.rename(destination)
    print(f"Verified checkpoint at {destination}, SHA-256 {SHA256}")


if __name__ == "__main__":
    main()
